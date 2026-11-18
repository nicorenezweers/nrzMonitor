//! Takt-Sammler: jede Sekunde die Werte aller laufenden Container, Summen je App und gesamt,
//! Momentaufnahmen `docker.containers` und `docker.apps`.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Instant,
};

use shared::{
    catalog::{Tpl, docker::*},
    docker::{App, Container},
};

use super::{
    Entry, Shared,
    cgroup::{self, Files, Mode, Raw},
};
use crate::{
    collectors::{Collector, Tick},
    core::Core,
    registry::{ChartDef, ChartHandle},
    util::{Counter, nan, round},
};

/// Herkunft der Rohwerte: echte cgroups oder Beispieldaten.
pub trait Source: Send + 'static {
    /// Bestand geändert (oder regelmässig): Dateien vorbereiten. Liefert volle ID → cgroup-Ordner.
    fn sync(&mut self, entries: &[Entry]) -> HashMap<String, String>;
    /// Werte eines Containers lesen; `false` = nichts gefunden.
    fn read(&mut self, full_id: &str, slow: bool, dt: f64, out: &mut Raw) -> bool;
}

/// Echte Werte aus `/sys/fs/cgroup` und `/proc/<pid>/net/dev`.
pub struct CgroupSource {
    root: Option<(Mode, PathBuf)>,
    proc: PathBuf,
    files: HashMap<String, (i32, Files)>,
    buf: Vec<u8>,
}

impl CgroupSource {
    pub fn new(sys: &Path, proc: &Path) -> Self {
        let root = cgroup::detect(sys);
        match &root {
            Some((m, p)) => tracing::info!("Docker: cgroup {m:?} unter {}", p.display()),
            None => tracing::warn!("Docker: keine cgroups unter {} gefunden – keine Messwerte je Container", sys.display()),
        }
        Self { root, proc: proc.to_path_buf(), files: HashMap::new(), buf: Vec::with_capacity(8192) }
    }
}

impl Source for CgroupSource {
    fn sync(&mut self, entries: &[Entry]) -> HashMap<String, String> {
        let Some((mode, root)) = &self.root else { return HashMap::new() };
        let running: HashSet<&str> = entries.iter().filter(|e| e.c.running() || e.c.state == "paused").map(|e| e.full_id.as_str()).collect();
        self.files.retain(|id, _| running.contains(id.as_str()));
        for e in entries.iter().filter(|e| running.contains(e.full_id.as_str())) {
            let stale = self.files.get(&e.full_id).is_none_or(|(pid, _)| *pid != e.pid);
            if stale {
                match Files::new(*mode, root, &self.proc, &e.full_id, e.pid, !e.c.host_net) {
                    Some(f) => {
                        self.files.insert(e.full_id.clone(), (e.pid, f));
                    }
                    None => {
                        self.files.remove(&e.full_id);
                    }
                }
            }
        }
        self.files.iter().map(|(id, (_, f))| (id.clone(), f.dir.display().to_string())).collect()
    }

    fn read(&mut self, full_id: &str, slow: bool, _dt: f64, out: &mut Raw) -> bool {
        match self.files.get_mut(full_id) {
            Some((_, f)) => {
                f.read(&mut self.buf, slow, out);
                true
            }
            None => false,
        }
    }
}

/// Diagramme eines Containers.
struct Charts {
    cpu: ChartHandle,
    mem: ChartHandle,
    net: ChartHandle,
    io: ChartHandle,
    pids: ChartHandle,
    throttle: ChartHandle,
}

const PER_CONTAINER: [&Tpl; 6] = [&DOCKER_CPU, &DOCKER_MEM, &DOCKER_NET, &DOCKER_IO, &DOCKER_PIDS, &DOCKER_THROTTLE];
const PER_APP: [&Tpl; 4] = [&APP_CPU, &APP_MEM, &APP_NET, &APP_IO];

/// Aktuelle Werte eines Containers.
#[derive(Debug, Default, Clone, Copy)]
struct Cur {
    user: f64,
    sys: f64,
    anon: f64,
    file: f64,
    kernel: f64,
    swap: f64,
    mem: f64,
    mem_total: f64,
    rx: f64,
    tx: f64,
    read: f64,
    write: f64,
    pids: f64,
    throttle: f64,
}

impl Cur {
    fn cpu(&self) -> f64 {
        nz(self.user) + nz(self.sys)
    }
}

/// NaN → 0 (für Summen).
fn nz(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

struct Slot {
    entry: Entry,
    charts: Option<Charts>,
    /// Name, unter dem die Diagramme angemeldet sind.
    chart_name: String,
    user: Counter,
    sys: Counter,
    periods: Counter,
    throttled: Counter,
    io_r: Counter,
    io_w: Counter,
    rx: Counter,
    tx: Counter,
    cur: Cur,
    oom_kills: u64,
    /// Geglättete Werte für die Rangliste der Summen (CPU, Speicher, Empfangen, Gesendet).
    rank: [f64; 4],
}

impl Slot {
    fn new(entry: Entry) -> Self {
        Self {
            entry,
            charts: None,
            chart_name: String::new(),
            user: Counter::new(),
            sys: Counter::new(),
            periods: Counter::new(),
            throttled: Counter::new(),
            io_r: Counter::new(),
            io_w: Counter::new(),
            rx: Counter::new(),
            tx: Counter::new(),
            cur: Cur::default(),
            oom_kills: 0,
            rank: [0.0; 4],
        }
    }
}

/// Summen-Diagramm „je Container/App“: die grössten einzeln, Rest als `other`. Plätze bleiben
/// stabil (Farbe = Platz); ein Platz wechselt nur, wenn sein Mitglied aus der Spitze fällt.
pub struct TopN {
    tpl: &'static Tpl,
    max: usize,
    pub slots: Vec<String>,
    handle: Option<ChartHandle>,
}

impl TopN {
    pub fn new(tpl: &'static Tpl, max: usize) -> Self {
        Self { tpl, max, slots: Vec::new(), handle: None }
    }

    /// Plätze neu verteilen. `ranked` = (Name, Wert) aller vorhandenen Mitglieder.
    pub fn rebalance(&mut self, reg: &crate::registry::Registry, ranked: &[(String, f64)]) {
        let mut sorted: Vec<&(String, f64)> = ranked.iter().filter(|(_, v)| *v > 0.0).collect();
        sorted.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let target: Vec<&str> = sorted.iter().take(self.max).map(|(n, _)| n.as_str()).collect();
        let present: HashSet<&str> = ranked.iter().map(|(n, _)| n.as_str()).collect();
        let mut newcomers = target.iter().filter(|n| !self.slots.iter().any(|s| s == *n)).copied().collect::<Vec<_>>().into_iter();
        let mut next = Vec::with_capacity(self.max);
        for s in &self.slots {
            if target.contains(&s.as_str()) {
                next.push(s.clone());
            } else if let Some(n) = newcomers.next() {
                next.push(n.to_string());
            } else if present.contains(s.as_str()) {
                next.push(s.clone()); // bleibt, solange niemand nachrückt
            }
        }
        for n in newcomers {
            if next.len() < self.max {
                next.push(n.to_string());
            }
        }
        if next != self.slots || self.handle.is_none() {
            self.slots = next;
            let def = ChartDef::tpl(self.tpl)
                .dims(self.slots.iter().map(|s| (s.clone(), s.clone())).chain([("other".to_string(), "Andere".to_string())]));
            self.handle = Some(reg.chart(def));
        }
    }

    /// Werte je Platz und `other` = Summe minus Plätze.
    pub fn put(&self, frame: &mut crate::frame::Frame, values: &HashMap<&str, f64>, digits: u32) {
        let Some(h) = &self.handle else { return };
        let total: f64 = values.values().copied().map(nz).sum();
        let mut out = Vec::with_capacity(self.slots.len() + 1);
        let mut used = 0.0;
        for s in &self.slots {
            let v = values.get(s.as_str()).copied().map_or(0.0, nz);
            used += v;
            out.push(round(v, digits));
        }
        out.push(round((total - used).max(0.0), digits));
        frame.put(h, &out);
    }
}

struct AppCharts {
    cpu: ChartHandle,
    mem: ChartHandle,
    net: ChartHandle,
    io: ChartHandle,
}

pub struct Stats {
    core: Core,
    shared: Shared,
    version: u64,
    source: Box<dyn Source>,
    slots: Vec<Slot>,
    top: [TopN; 4],
    app_top: [TopN; 2],
    apps: HashMap<String, AppCharts>,
    states: ChartHandle,
    ticks: u64,
    raw: Raw,
    last_sync: u64,
}

impl Stats {
    pub fn new(core: &Core, shared: Shared, source: Box<dyn Source>) -> Self {
        Self {
            core: core.clone(),
            shared,
            version: u64::MAX,
            source,
            slots: Vec::new(),
            top: [
                TopN::new(&DOCKER_CPU_ALL, 12),
                TopN::new(&DOCKER_MEM_ALL, 12),
                TopN::new(&DOCKER_NET_RX_ALL, 12),
                TopN::new(&DOCKER_NET_TX_ALL, 12),
            ],
            app_top: [TopN::new(&APPS_CPU_ALL, 12), TopN::new(&APPS_MEM_ALL, 12)],
            apps: HashMap::new(),
            states: core.chart(ChartDef::tpl(&DOCKER_CONTAINERS)),
            ticks: 0,
            raw: Raw::default(),
            last_sync: 0,
        }
    }

    /// Bestand übernehmen: Plätze behalten (Zähler!), neue anlegen, entfernte stilllegen.
    fn resync(&mut self) {
        let (version, entries) = {
            let st = self.shared.read();
            (st.version, st.entries.clone())
        };
        let mut old: HashMap<String, Slot> = self.slots.drain(..).map(|s| (s.entry.full_id.clone(), s)).collect();
        let mut slots = Vec::with_capacity(entries.len());
        for e in entries {
            match old.remove(&e.full_id) {
                Some(mut s) => {
                    if s.entry.pid != e.pid {
                        // Neu gestartet: Zähler neu beginnen.
                        let charts = s.charts.take();
                        let name = std::mem::take(&mut s.chart_name);
                        s = Slot { charts, chart_name: name, oom_kills: s.oom_kills, rank: s.rank, ..Slot::new(e.clone()) };
                    }
                    s.entry = e;
                    slots.push(s);
                }
                None => slots.push(Slot::new(e)),
            }
        }
        let names: HashSet<String> = slots.iter().map(|s| s.entry.c.name.clone()).collect();
        for s in old.into_values() {
            if !s.chart_name.is_empty() && !names.contains(&s.chart_name) {
                retire(&self.core, &s.chart_name);
            }
        }
        // Umbenannt: alte Diagramme stilllegen.
        for s in &mut slots {
            if !s.chart_name.is_empty() && s.chart_name != s.entry.c.name {
                if !names.contains(&s.chart_name) {
                    retire(&self.core, &s.chart_name);
                }
                s.charts = None;
                s.chart_name.clear();
            }
        }
        let apps: HashSet<String> = slots.iter().map(|s| s.entry.c.app.clone()).collect();
        let gone: Vec<String> = self.apps.keys().filter(|a| !apps.contains(*a)).cloned().collect();
        for a in gone {
            self.apps.remove(&a);
            for t in PER_APP {
                self.core.registry.retire(&t.chart_id(Some(&a)));
            }
        }
        self.slots = slots;
        self.version = version;
        self.sync_source();
    }

    fn sync_source(&mut self) {
        let entries: Vec<Entry> = self.slots.iter().map(|s| s.entry.clone()).collect();
        let dirs = self.source.sync(&entries);
        self.last_sync = self.ticks;
        self.shared.write().cgroups = dirs;
    }

    fn charts_for(core: &Core, e: &Entry) -> Charts {
        let def = |t: &Tpl| ChartDef::tpl(t).instance(&e.c.name).label("app", e.c.app.clone()).label("image", e.c.image.clone());
        Charts {
            cpu: core.chart(def(&DOCKER_CPU)),
            mem: core.chart(def(&DOCKER_MEM)),
            net: core.chart(def(&DOCKER_NET)),
            io: core.chart(def(&DOCKER_IO)),
            pids: core.chart(def(&DOCKER_PIDS)),
            throttle: core.chart(def(&DOCKER_THROTTLE)),
        }
    }
}

fn retire(core: &Core, name: &str) {
    for t in PER_CONTAINER {
        core.registry.retire(&t.chart_id(Some(name)));
    }
}

impl Collector for Stats {
    fn name(&self) -> &'static str {
        "docker"
    }

    fn collect(&mut self, t: &mut Tick<'_>) -> anyhow::Result<()> {
        let started = Instant::now();
        self.ticks += 1;
        if self.shared.read().version != self.version {
            self.resync();
        } else if self.ticks - self.last_sync >= 30 {
            // Container ohne gefundene cgroup regelmässig erneut versuchen.
            self.sync_source();
        }
        let slow = t.due(5);
        let dt = t.dt;
        let mut found = 0u32;
        for s in &mut self.slots {
            let running = s.entry.c.running() || s.entry.c.state == "paused";
            if !running || !self.source.read(&s.entry.full_id, slow, dt, &mut self.raw) {
                s.cur = Cur::default();
                continue;
            }
            found += 1;
            let r = self.raw;
            let rate = |c: &mut Counter, v: Option<u64>| v.and_then(|v| c.rate(v, dt));
            let mut cur = Cur {
                user: nan(rate(&mut s.user, r.cpu_user_us).map(|v| v / 1e4)),
                sys: nan(rate(&mut s.sys, r.cpu_sys_us).map(|v| v / 1e4)),
                anon: r.mem_anon.map_or(f64::NAN, |v| v as f64),
                file: r.mem_file.map_or(f64::NAN, |v| v as f64),
                kernel: r.mem_kernel.map_or(f64::NAN, |v| v as f64),
                swap: r.mem_swap.map_or(f64::NAN, |v| v as f64),
                mem_total: r.mem_current.map_or(f64::NAN, |v| v as f64),
                mem: r.mem_current.map_or(f64::NAN, |v| v.saturating_sub(r.mem_inactive_file.unwrap_or(0)) as f64),
                rx: nan(rate(&mut s.rx, r.net_rx).map(|v| v * 8.0)),
                tx: nan(rate(&mut s.tx, r.net_tx).map(|v| v * 8.0)),
                read: nan(rate(&mut s.io_r, r.io_read)),
                write: nan(rate(&mut s.io_w, r.io_write)),
                pids: s.cur.pids,
                throttle: s.cur.throttle,
            };
            if slow {
                cur.pids = r.pids.map_or(f64::NAN, |v| v as f64);
                let dp = r.periods.and_then(|v| s.periods.delta(v));
                let dthr = r.throttled.and_then(|v| s.throttled.delta(v));
                cur.throttle = match (dp, dthr) {
                    (Some(p), Some(t)) if p > 0 => t as f64 / p as f64 * 100.0,
                    (Some(_), Some(_)) => 0.0,
                    _ => f64::NAN,
                };
                if let Some(o) = r.oom_kills {
                    s.oom_kills = o;
                }
            }
            s.cur = cur;
            if s.charts.is_none() {
                s.charts = Some(Self::charts_for(&self.core, &s.entry));
                s.chart_name = s.entry.c.name.clone();
            }
            let Some(ch) = &s.charts else { continue };
            t.put(&ch.cpu, &[round(cur.user, 2), round(cur.sys, 2)]);
            t.put(&ch.mem, &[round(cur.anon, 0), round(cur.file, 0), round(cur.kernel, 0), round(cur.swap, 0)]);
            if !s.entry.c.host_net {
                t.put(&ch.net, &[round(cur.rx, 0), round(cur.tx, 0)]);
            }
            t.put(&ch.io, &[round(cur.read, 0), round(cur.write, 0)]);
            if slow {
                t.put(&ch.pids, &[cur.pids]);
                t.put(&ch.throttle, &[round(cur.throttle, 2)]);
            }
            // Rangliste glätten (etwa eine Minute).
            let vals = [cur.cpu(), nz(cur.mem), nz(cur.rx), nz(cur.tx)];
            for (r, v) in s.rank.iter_mut().zip(vals) {
                *r = if *r == 0.0 { v } else { *r * 0.95 + v * 0.05 };
            }
        }
        let read_ms = started.elapsed().as_secs_f64() * 1000.0;

        // --- Summen je Container
        if self.ticks == 3 || self.ticks.is_multiple_of(60) || self.top[0].slots.is_empty() && self.ticks.is_multiple_of(5) {
            for (k, top) in self.top.iter_mut().enumerate() {
                let ranked: Vec<(String, f64)> = self.slots.iter().map(|s| (s.entry.c.name.clone(), s.rank[k])).collect();
                top.rebalance(&self.core.registry, &ranked);
            }
        }
        let pick = |f: fn(&Cur) -> f64| -> HashMap<&str, f64> {
            self.slots.iter().filter(|s| s.entry.c.running()).map(|s| (s.entry.c.name.as_str(), f(&s.cur))).collect()
        };
        if self.ticks >= 3 {
            self.top[0].put(&mut t.frame, &pick(Cur::cpu), 2);
            self.top[1].put(&mut t.frame, &pick(|c| nz(c.mem)), 0);
            self.top[2].put(&mut t.frame, &pick(|c| nz(c.rx)), 0);
            self.top[3].put(&mut t.frame, &pick(|c| nz(c.tx)), 0);
        }

        // --- je App
        let mut apps: Vec<App> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for s in &self.slots {
            let c = &s.entry.c;
            let k = *index.entry(c.app.clone()).or_insert_with(|| {
                apps.push(App {
                    name: c.app.clone(),
                    dir: s.entry.labels.get("com.docker.compose.project.working_dir").cloned().unwrap_or_default(),
                    ..App::default()
                });
                apps.len() - 1
            });
            let a = &mut apps[k];
            a.containers.push(c.name.clone());
            a.total += 1;
            a.restarts += c.restarts;
            if c.unhealthy() {
                a.unhealthy += 1;
            }
            if c.running() {
                a.running += 1;
                if c.started > 0 && (a.started == 0 || c.started < a.started) {
                    a.started = c.started;
                }
                a.cpu += s.cur.cpu();
                a.mem += nz(s.cur.mem);
                a.net_rx += nz(s.cur.rx);
                a.net_tx += nz(s.cur.tx);
                a.io_read += nz(s.cur.read);
                a.io_write += nz(s.cur.write);
            }
        }
        for a in &mut apps {
            a.cpu = round(a.cpu, 2);
            a.mem = round(a.mem, 0);
            a.net_rx = round(a.net_rx, 0);
            a.net_tx = round(a.net_tx, 0);
            a.io_read = round(a.io_read, 0);
            a.io_write = round(a.io_write, 0);
            if a.running == 0 {
                continue;
            }
            let ch = self.apps.entry(a.name.clone()).or_insert_with(|| {
                let def = |t: &Tpl| ChartDef::tpl(t).instance(&a.name);
                AppCharts {
                    cpu: self.core.chart(def(&APP_CPU)),
                    mem: self.core.chart(def(&APP_MEM)),
                    net: self.core.chart(def(&APP_NET)),
                    io: self.core.chart(def(&APP_IO)),
                }
            });
            if self.ticks >= 3 {
                t.put(&ch.cpu, &[a.cpu]);
                t.put(&ch.mem, &[a.mem]);
                t.put(&ch.net, &[a.net_rx, a.net_tx]);
                t.put(&ch.io, &[a.io_read, a.io_write]);
            }
        }
        if self.ticks == 3 || self.ticks.is_multiple_of(60) || self.app_top[0].slots.is_empty() && self.ticks.is_multiple_of(5) {
            // Rangliste der Apps aus den geglätteten Werten ihrer Container.
            let mut rank: HashMap<&str, [f64; 2]> = HashMap::new();
            for s in &self.slots {
                let r = rank.entry(s.entry.c.app.as_str()).or_default();
                r[0] += s.rank[0];
                r[1] += s.rank[1];
            }
            for (k, top) in self.app_top.iter_mut().enumerate() {
                let ranked: Vec<(String, f64)> = rank.iter().map(|(n, r)| ((*n).to_string(), r[k])).collect();
                top.rebalance(&self.core.registry, &ranked);
            }
        }
        if self.ticks >= 3 {
            let cpu: HashMap<&str, f64> = apps.iter().map(|a| (a.name.as_str(), a.cpu)).collect();
            let mem: HashMap<&str, f64> = apps.iter().map(|a| (a.name.as_str(), a.mem)).collect();
            self.app_top[0].put(&mut t.frame, &cpu, 2);
            self.app_top[1].put(&mut t.frame, &mem, 0);
        }

        // --- Zustände
        if slow {
            let mut n = [0f64; 5];
            for s in &self.slots {
                let c = &s.entry.c;
                match c.state.as_str() {
                    "running" => n[0] += 1.0,
                    "paused" => n[1] += 1.0,
                    "restarting" => n[2] += 1.0,
                    _ => n[3] += 1.0,
                }
                if c.unhealthy() {
                    n[4] += 1.0;
                }
            }
            t.put(&self.states, &n);
        }

        // --- Momentaufnahmen
        let list: Vec<Container> = self
            .slots
            .iter()
            .map(|s| {
                let mut c = s.entry.c.clone();
                let cur = &s.cur;
                c.oom_kills = c.oom_kills.max(s.oom_kills);
                c.cpu = round(cur.cpu(), 2);
                c.mem = round(nz(cur.mem), 0);
                c.mem_total = round(nz(cur.mem_total), 0);
                c.net_rx = round(nz(cur.rx), 0);
                c.net_tx = round(nz(cur.tx), 0);
                c.io_read = round(nz(cur.read), 0);
                c.io_write = round(nz(cur.write), 0);
                c.pids = nz(cur.pids) as u32;
                c
            })
            .collect();
        self.core.snapshot("docker.containers", &list);
        apps.sort_by(|a, b| a.name.cmp(&b.name));
        self.core.snapshot("docker.apps", &apps);

        if self.ticks % 10 == 2 {
            let total_ms = started.elapsed().as_secs_f64() * 1000.0;
            let info = {
                let mut st = self.shared.write();
                st.info.collect_ms = round(total_ms, 2);
                st.info.cgroups_found = found;
                st.info.clone()
            };
            self.core.snapshot("docker.info", &info);
            if self.ticks % 300 == 2 {
                tracing::debug!("Docker: {} Container, {found} mit cgroup, Lesen {read_ms:.2} ms, gesamt {total_ms:.2} ms", self.slots.len());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topn_keeps_slots_stable() {
        let core = crate::registry::Registry::load(None, 1).expect("registry");
        let mut top = TopN::new(&DOCKER_CPU_ALL, 2);
        let r = |v: &[(&str, f64)]| v.iter().map(|(n, x)| ((*n).to_string(), *x)).collect::<Vec<_>>();
        top.rebalance(&core, &r(&[("a", 5.0), ("b", 3.0), ("c", 1.0)]));
        assert_eq!(top.slots, vec!["a", "b"]);
        top.rebalance(&core, &r(&[("a", 1.0), ("b", 3.0), ("c", 9.0)]));
        assert_eq!(top.slots, vec!["c", "b"], "c übernimmt den Platz von a, b bleibt an seinem Platz");
        let mut f = crate::frame::Frame::new(0);
        let vals: HashMap<&str, f64> = [("a", 1.0), ("b", 3.0), ("c", 9.0)].into_iter().collect();
        top.put(&mut f, &vals, 2);
        assert_eq!(f.values, vec![9.0, 3.0, 1.0]);
    }
}
