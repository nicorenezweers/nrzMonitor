//! Alarme: Regeln auf Messwerten (`core.registry` + `core.ring`) und Zuständen (Container,
//! ausbleibende Daten), alle fünf Sekunden geprüft. Ereignisse `alert.raised`/`alert.resolved`,
//! Momentaufnahme `alerts.active`, Benachrichtigungen über die Kanäle aus den Einstellungen.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    time::Duration,
};

use parking_lot::Mutex;
use shared::{
    ChartMeta, Unit,
    alerts::{ActiveAlert, AlertConfig, Cmp, Expr, LabelFilter, Rule, RuleKind, StateCheck, glob, parse_filter},
    catalog::docker::{DOCKER_CPU, DOCKER_MEM},
    docker::Container,
    events::{Event, Severity},
    model::duration,
};

use crate::{core::Core, util};

pub mod notify;
pub mod rules;

/// Sekunden zwischen zwei Prüfungen.
const EVERY: u64 = 5;
/// Nach dem Start so lange warten (Raten brauchen Vorwerte, Sammler müssen anlaufen).
const WARMUP: i64 = 20;

/// Zustand je Regel für die API: (laufend, geprüft, Fehler).
type StatusMap = BTreeMap<String, (u32, u32, Option<String>)>;
static STATUS: Mutex<StatusMap> = Mutex::new(BTreeMap::new());

/// (laufende Alarme, geprüfte Objekte, Fehler) einer Regel.
pub fn rule_status(id: &str) -> (u32, u32, Option<String>) {
    STATUS.lock().get(id).cloned().unwrap_or_default()
}

pub fn start(core: &Core) {
    let core = core.clone();
    tokio::spawn(async move {
        let mut engine = Engine::new(core);
        let mut iv = tokio::time::interval(Duration::from_secs(EVERY));
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            iv.tick().await;
            engine.round();
        }
    });
}

/// Eine Beobachtung je Regel und Objekt in einer Runde.
struct Obs {
    key: String,
    subject: String,
    chart: Option<String>,
    /// `None` = kein Wert (Zustand bleibt, wie er ist).
    target: Option<Severity>,
    value: String,
    /// Für Schwellen-Regeln: Vergleich und Einheit (für die Meldung).
    thresholds: Option<(Cmp, Unit)>,
    /// Meldung bei Zustandsregeln ohne Schwellen.
    detail: String,
}

#[derive(Debug, Clone)]
struct AState {
    rule_id: String,
    rule: String,
    group: String,
    notify: bool,
    level: Severity,
    pending: Option<i64>,
    since: i64,
    updated: i64,
    value: String,
    title: String,
    message: String,
    subject: String,
    chart: Option<String>,
    seen: bool,
    notified: bool,
    last_notified: i64,
    repeats: u32,
}

impl AState {
    fn active(&self) -> bool {
        self.level >= Severity::Warn
    }

    fn view(&self, key: &str) -> ActiveAlert {
        ActiveAlert {
            id: key.to_string(),
            rule: self.rule.clone(),
            severity: self.level,
            title: self.title.clone(),
            message: self.message.clone(),
            subject: self.subject.clone(),
            since: self.since,
            value: self.value.clone(),
            chart: self.chart.clone(),
            rule_id: self.rule_id.clone(),
            group: self.group.clone(),
            updated: self.updated,
        }
    }
}

/// Neue Stufe aus Wert und bisheriger Stufe; Hysterese hält laufende Alarme etwas länger.
pub fn level(rule: &Rule, v: f64, cur: Severity) -> Severity {
    let h = rule.hysteresis.abs();
    let adj = |thr: f64, active: bool| {
        if !active {
            thr
        } else {
            match rule.cmp {
                Cmp::Above => thr - h,
                Cmp::Below => thr + h,
            }
        }
    };
    if let Some(c) = rule.crit
        && rule.cmp.hit(v, adj(c, cur == Severity::Crit)) {
            return Severity::Crit;
        }
    if let Some(w) = rule.warn
        && rule.cmp.hit(v, adj(w, cur >= Severity::Warn)) {
            return Severity::Warn;
        }
    Severity::Info
}

fn filter_ok(filters: &[LabelFilter], get: &dyn Fn(&str) -> Option<String>) -> bool {
    filters.iter().all(|f| match get(&f.key) {
        Some(v) => glob(&f.value, &v) != f.negate,
        None => f.negate,
    })
}

struct Engine {
    core: Core,
    alerts: HashMap<String, AState>,
    /// Letzter Neustart-Zähler je Container und Zuwächse (Zeit, Anzahl).
    restarts_last: HashMap<String, u32>,
    restarts: HashMap<String, VecDeque<(i64, u32)>>,
    /// OOM-Zähler, OOM-Flag und Zeit des letzten OOM je Container.
    ooms: HashMap<String, (u64, bool, i64)>,
}

impl Engine {
    fn new(core: Core) -> Self {
        Self { core, alerts: HashMap::new(), restarts_last: HashMap::new(), restarts: HashMap::new(), ooms: HashMap::new() }
    }

    fn round(&mut self) {
        let now_ms = util::now_ms();
        let now = now_ms / 1000;
        if now - self.core.started_at < WARMUP {
            return;
        }
        let cfg: AlertConfig = self.core.settings.get("alerts");
        let rules = rules::effective(&cfg);
        let charts = self.core.registry.list(false);
        let containers: Vec<Container> = self
            .core
            .snaps
            .get("docker.containers")
            .and_then(|s| serde_json::from_value(s.data.clone()).ok())
            .unwrap_or_default();
        let cores = self
            .core
            .registry
            .get("cpu.cores")
            .map(|m| m.dims.len())
            .filter(|n| *n > 0)
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from)) as f64;
        self.track(&containers, now);
        for a in self.alerts.values_mut() {
            a.seen = false;
        }
        let mut status = BTreeMap::new();
        for rule in rules.iter().filter(|r| r.enabled) {
            let mut obs = Vec::new();
            let res = match rule.kind {
                RuleKind::Metric => self.metric(rule, &charts, cores, now, &mut obs),
                RuleKind::State => {
                    self.state(rule, &containers, &charts, now, &mut obs);
                    Ok(())
                }
            };
            let checked = obs.len() as u32;
            for o in obs {
                self.step(rule, o, now_ms, &cfg);
            }
            status.insert(rule.id.clone(), (0u32, checked, res.err()));
        }
        // Verschwundene Objekte und abgeschaltete Regeln: Alarme beenden.
        let gone: Vec<String> = self.alerts.iter().filter(|(_, a)| !a.seen).map(|(k, _)| k.clone()).collect();
        for k in gone {
            if let Some(a) = self.alerts.remove(&k)
                && a.active() {
                    self.resolved(&a, now_ms, &cfg);
                }
        }
        // Erinnerungen.
        if cfg.repeat_minutes > 0 {
            let every = i64::from(cfg.repeat_minutes) * 60_000;
            let due: Vec<String> = self
                .alerts
                .iter()
                .filter(|(_, a)| a.active() && a.notified && a.repeats < cfg.max_repeats && now_ms - a.last_notified >= every)
                .map(|(k, _)| k.clone())
                .collect();
            for k in due {
                if let Some(a) = self.alerts.get_mut(&k) {
                    a.repeats += 1;
                    a.last_notified = now_ms;
                    let msg = notify::Message {
                        title: format!("Noch aktiv: {}", a.title),
                        body: format!("{}\nseit {}", a.message, duration(((now_ms - a.since) / 1000) as f64)),
                        severity: a.level,
                        status: "reminder",
                        payload: serde_json::json!({ "event": "alert.reminder", "alert": a.view(&k) }),
                    };
                    notify::send_all(&self.core, &cfg.channels, msg);
                }
            }
        }
        self.alerts.retain(|_, a| a.active() || a.pending.is_some());
        for a in self.alerts.values().filter(|a| a.active()) {
            if let Some(s) = status.get_mut(&a.rule_id) {
                s.0 += 1;
            }
        }
        *STATUS.lock() = status;
        let mut active: Vec<ActiveAlert> = self.alerts.iter().filter(|(_, a)| a.active()).map(|(k, a)| a.view(k)).collect();
        active.sort_by(|a, b| b.severity.cmp(&a.severity).then(b.since.cmp(&a.since)).then_with(|| a.id.cmp(&b.id)));
        self.core.snapshot("alerts.active", &active);
    }

    /// Neustarts und OOM je Container mitschreiben.
    fn track(&mut self, containers: &[Container], now: i64) {
        let names: HashSet<&str> = containers.iter().map(|c| c.name.as_str()).collect();
        self.restarts_last.retain(|k, _| names.contains(k.as_str()));
        self.restarts.retain(|k, _| names.contains(k.as_str()));
        self.ooms.retain(|k, _| names.contains(k.as_str()));
        for c in containers {
            match self.restarts_last.insert(c.name.clone(), c.restarts) {
                Some(prev) if c.restarts > prev => {
                    self.restarts.entry(c.name.clone()).or_default().push_back((now, c.restarts - prev));
                }
                _ => {}
            }
            if let Some(q) = self.restarts.get_mut(&c.name) {
                while q.front().is_some_and(|(t, _)| now - t > 86_400) {
                    q.pop_front();
                }
            }
            match self.ooms.get_mut(&c.name) {
                Some((count, killed, ts)) => {
                    if c.oom_kills > *count || (c.oom_killed && !*killed) {
                        *ts = now;
                    }
                    *count = c.oom_kills;
                    *killed = c.oom_killed;
                }
                None => {
                    // Beim ersten Blick nur ein frisches OOM-Ende zählen.
                    let ts = if c.oom_killed && !c.running() && c.finished > now - 3600 { c.finished } else { 0 };
                    self.ooms.insert(c.name.clone(), (c.oom_kills, c.oom_killed, ts));
                }
            }
        }
    }

    fn cur_level(&self, key: &str) -> Severity {
        self.alerts.get(key).map_or(Severity::Info, |a| a.level)
    }

    /// Wert einer Reihe: Mittel über `window` Sekunden oder der neueste frische Wert.
    fn series_value(&self, series: u32, window: u32, interval: u32, now: i64) -> Option<f64> {
        if window > interval {
            let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
            let (vals, _, _) = self.core.ring.read(series, now - i64::from(window), window, 1, avg);
            return vals.first().copied().flatten();
        }
        let (ts, v) = self.core.ring.latest(series)?;
        (ts >= now - i64::from(interval.max(1)) * 3 - 5).then_some(v)
    }

    fn metric(&self, rule: &Rule, charts: &[ChartMeta], cores: f64, now: i64, obs: &mut Vec<Obs>) -> Result<(), String> {
        let expr = Expr::parse(&rule.expr)?;
        let filters = parse_filter(&rule.filter);
        for meta in charts.iter().filter(|m| rule.matches_context(&m.context)) {
            let get_label = |k: &str| match k {
                "id" | "chart" => Some(meta.id.clone()),
                "instance" => Some(meta.family.clone()),
                _ => meta.labels.get(k).cloned(),
            };
            if !filter_ok(&filters, &get_label) {
                continue;
            }
            let vals: Vec<Option<f64>> = meta.dims.iter().map(|d| self.series_value(d.series, rule.window, meta.interval, now)).collect();
            let get = |id: &str| meta.dims.iter().position(|d| d.id == id).and_then(|i| vals[i]);
            let all = || vals.iter().flatten().copied().collect::<Vec<f64>>();
            let v = expr.eval(&get, &all, cores);
            let unit = if expr.is_ratio() { Unit::Percent } else { meta.unit };
            let key = format!("{}:{}", rule.id, meta.id);
            let cur = self.cur_level(&key);
            obs.push(Obs {
                subject: if meta.family.is_empty() { self.core.hostname.clone() } else { meta.family.clone() },
                chart: Some(meta.id.clone()),
                target: v.map(|v| level(rule, v, cur)),
                value: v.map_or_else(|| "–".to_string(), |v| unit.format(v)),
                thresholds: Some((rule.cmp, unit)),
                detail: String::new(),
                key,
            });
        }
        Ok(())
    }

    fn state(&self, rule: &Rule, containers: &[Container], charts: &[ChartMeta], now: i64, obs: &mut Vec<Obs>) {
        let filters = parse_filter(&rule.filter);
        let sev = if rule.severity >= Severity::Warn { rule.severity } else { Severity::Warn };
        let flag = |b: bool| Some(if b { sev } else { Severity::Info });
        if rule.check == StateCheck::NoData {
            for meta in charts.iter().filter(|m| rule.matches_context(&m.context)) {
                let get_label = |k: &str| if k == "id" { Some(meta.id.clone()) } else { meta.labels.get(k).cloned() };
                if !filter_ok(&filters, &get_label) {
                    continue;
                }
                let Some(last) = meta.dims.iter().filter_map(|d| self.core.ring.last_ts(d.series)).max() else { continue };
                // Seltene Diagramme: Schwelle gilt zusätzlich zum eigenen Takt.
                let age = (now - last - i64::from(meta.interval)).max(0) as f64;
                let key = format!("{}:{}", rule.id, meta.id);
                obs.push(Obs {
                    subject: if meta.family.is_empty() { meta.title.clone() } else { format!("{} {}", meta.title, meta.family) },
                    chart: Some(meta.id.clone()),
                    target: Some(level(rule, age, self.cur_level(&key))),
                    value: Unit::Seconds.format(age),
                    thresholds: Some((Cmp::Above, Unit::Seconds)),
                    detail: String::new(),
                    key,
                });
            }
            return;
        }
        for c in containers {
            let get_label = |k: &str| match k {
                "container" | "name" => Some(c.name.clone()),
                "app" => Some(c.app.clone()),
                "image" => Some(c.image.clone()),
                "service" => Some(c.service.clone()),
                "state" => Some(c.state.clone()),
                _ => None,
            };
            if !filter_ok(&filters, &get_label) {
                continue;
            }
            let key = format!("{}:{}", rule.id, c.name);
            let cpu_chart = Some(DOCKER_CPU.chart_id(Some(&c.name)));
            let o = match rule.check {
                StateCheck::ContainerExited => {
                    let bad = matches!(c.state.as_str(), "exited" | "dead") && c.exit_code != 0 && !c.expected_exit;
                    Obs {
                        target: flag(bad),
                        value: format!("Code {}", c.exit_code),
                        thresholds: None,
                        detail: format!("Beendet mit Code {}{}", c.exit_code, if c.oom_killed { " (Speichermangel)" } else { "" }),
                        chart: cpu_chart,
                        subject: c.name.clone(),
                        key,
                    }
                }
                StateCheck::ContainerUnhealthy => Obs {
                    target: flag(c.running() && c.unhealthy()),
                    value: c.health.clone(),
                    thresholds: None,
                    detail: "Healthcheck schlägt fehl".to_string(),
                    chart: cpu_chart,
                    subject: c.name.clone(),
                    key,
                },
                StateCheck::RestartLoop => {
                    let window = i64::from(rule.window.max(60));
                    let n: u32 = self.restarts.get(&c.name).map_or(0, |q| q.iter().filter(|(t, _)| now - t <= window).map(|(_, d)| d).sum());
                    let cur = self.cur_level(&key);
                    Obs {
                        target: Some(level(rule, f64::from(n), cur)),
                        value: format!("{n} in {}", duration(window as f64)),
                        thresholds: Some((rule.cmp, Unit::Count)),
                        detail: String::new(),
                        chart: cpu_chart,
                        subject: c.name.clone(),
                        key,
                    }
                }
                StateCheck::ContainerOom => {
                    let last = self.ooms.get(&c.name).map_or(0, |o| o.2);
                    let window = i64::from(rule.window.max(60));
                    Obs {
                        target: flag(last > 0 && now - last <= window),
                        value: if last > 0 { format!("vor {}", duration((now - last) as f64)) } else { "–".into() },
                        thresholds: None,
                        detail: format!("Speichermangel: Prozess beendet ({} OOM seit Start)", c.oom_kills.max(u64::from(c.oom_killed))),
                        chart: Some(DOCKER_MEM.chart_id(Some(&c.name))),
                        subject: c.name.clone(),
                        key,
                    }
                }
                StateCheck::MemLimit => {
                    let Some(pct) = c.mem_pct().filter(|_| c.running()) else { continue };
                    let cur = self.cur_level(&key);
                    Obs {
                        target: Some(level(rule, pct, cur)),
                        value: Unit::Percent.format(util::round(pct, 1)),
                        thresholds: Some((rule.cmp, Unit::Percent)),
                        detail: String::new(),
                        chart: Some(DOCKER_MEM.chart_id(Some(&c.name))),
                        subject: c.name.clone(),
                        key,
                    }
                }
                StateCheck::NoData => continue,
            };
            obs.push(o);
        }
    }

    fn step(&mut self, rule: &Rule, o: Obs, now_ms: i64, cfg: &AlertConfig) {
        let a = self.alerts.entry(o.key.clone()).or_insert_with(|| AState {
            rule_id: rule.id.clone(),
            rule: rule.name.clone(),
            group: rule.group.clone(),
            notify: rule.notify,
            level: Severity::Info,
            pending: None,
            since: now_ms,
            updated: now_ms,
            value: String::new(),
            title: String::new(),
            message: String::new(),
            subject: String::new(),
            chart: None,
            seen: true,
            notified: false,
            last_notified: 0,
            repeats: 0,
        });
        a.seen = true;
        a.updated = now_ms;
        a.notify = rule.notify;
        a.rule.clone_from(&rule.name);
        a.value.clone_from(&o.value);
        a.subject.clone_from(&o.subject);
        a.chart.clone_from(&o.chart);
        a.title = format!("{}: {}", rule.name, o.subject);
        let Some(target) = o.target else { return };
        let message = |lvl: Severity| match o.thresholds {
            Some((cmp, unit)) => {
                let thr = if lvl == Severity::Crit { rule.crit } else { rule.warn };
                match thr {
                    Some(t) => format!("{} {} {}", o.value, cmp.symbol(), unit.format(t)),
                    None => o.value.clone(),
                }
            }
            None => o.detail.clone(),
        };
        if target > a.level {
            let since = *a.pending.get_or_insert(now_ms);
            if now_ms - since >= i64::from(rule.duration) * 1000 {
                let escalate = a.active();
                a.level = target;
                a.pending = None;
                if !escalate {
                    a.since = since;
                    a.repeats = 0;
                }
                a.message = message(target);
                let snapshot = a.clone();
                if let Some(a) = self.alerts.get_mut(&o.key)
                    && a.notify {
                        a.notified = true;
                        a.last_notified = now_ms;
                    }
                self.raised(&o.key, &snapshot, escalate, cfg);
            }
            return;
        }
        a.pending = None;
        if a.active() {
            a.message = message(a.level);
        }
        if target < a.level {
            if target < Severity::Warn {
                let snapshot = a.clone();
                a.level = Severity::Info;
                a.notified = false;
                self.resolved(&snapshot, now_ms, cfg);
            } else {
                a.level = target;
                a.message = message(target);
            }
        }
    }

    fn raised(&self, key: &str, a: &AState, escalate: bool, cfg: &AlertConfig) {
        let mut ev = Event::new("alert", "alert.raised", a.level, &a.subject, a.title.clone())
            .message(a.message.clone())
            .label("rule", a.rule_id.clone())
            .label("alert", key);
        if let Some(c) = &a.chart {
            ev = ev.label("chart", c.clone());
        }
        self.core.event(ev);
        if a.notify {
            notify::send_all(
                &self.core,
                &cfg.channels,
                notify::Message {
                    title: if escalate { format!("Kritisch: {}", a.title) } else { a.title.clone() },
                    body: format!("{}\n{}", a.message, self.core.hostname),
                    severity: a.level,
                    status: "raised",
                    payload: serde_json::json!({ "event": "alert.raised", "alert": a.view(key), "host": self.core.hostname }),
                },
            );
        }
    }

    fn resolved(&self, a: &AState, now_ms: i64, cfg: &AlertConfig) {
        let took = duration(((now_ms - a.since) / 1000).max(0) as f64);
        let mut ev = Event::new("alert", "alert.resolved", Severity::Ok, &a.subject, format!("Behoben: {}", a.title))
            .message(format!("Dauerte {took} – zuletzt {}", a.value))
            .label("rule", a.rule_id.clone());
        if let Some(c) = &a.chart {
            ev = ev.label("chart", c.clone());
        }
        self.core.event(ev);
        if a.notified && cfg.notify_resolved {
            notify::send_all(
                &self.core,
                &cfg.channels,
                notify::Message {
                    title: format!("Behoben: {}", a.title),
                    body: format!("Dauerte {took}\n{}", self.core.hostname),
                    severity: Severity::Ok,
                    status: "resolved",
                    payload: serde_json::json!({ "event": "alert.resolved", "rule": a.rule_id, "subject": a.subject, "host": self.core.hostname }),
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_with_hysteresis() {
        let r = Rule { warn: Some(80.0), crit: Some(90.0), hysteresis: 5.0, ..Rule::default() };
        assert_eq!(level(&r, 70.0, Severity::Info), Severity::Info);
        assert_eq!(level(&r, 85.0, Severity::Info), Severity::Warn);
        assert_eq!(level(&r, 95.0, Severity::Info), Severity::Crit);
        assert_eq!(level(&r, 87.0, Severity::Crit), Severity::Crit, "bleibt kritisch bis unter 85");
        assert_eq!(level(&r, 84.0, Severity::Crit), Severity::Warn);
        assert_eq!(level(&r, 77.0, Severity::Warn), Severity::Warn, "bleibt Warnung bis unter 75");
        assert_eq!(level(&r, 74.0, Severity::Warn), Severity::Info);
        let below = Rule { cmp: Cmp::Below, crit: Some(0.5), ..Rule::default() };
        assert_eq!(level(&below, 0.0, Severity::Info), Severity::Crit);
        assert_eq!(level(&below, 1.0, Severity::Crit), Severity::Info);
    }

    #[test]
    fn filters() {
        let f = parse_filter("app=nrz-*, device!=loop*");
        let get = |k: &str| match k {
            "app" => Some("nrz-git".to_string()),
            "device" => Some("nvme0n1".to_string()),
            _ => None,
        };
        assert!(filter_ok(&f, &get));
        let get2 = |k: &str| if k == "device" { Some("loop0".to_string()) } else { Some("nrz-git".to_string()) };
        assert!(!filter_ok(&f, &get2));
    }
}
