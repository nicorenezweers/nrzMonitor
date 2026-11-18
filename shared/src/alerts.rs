//! Typen des Bereichs `alerts` (Momentaufnahmen, Einstellungen, Listen).
//! Besitzer und Inhalt: siehe `ARCHITECTURE.md`.
//!
//! - Momentaufnahme `alerts.active` → `Vec<ActiveAlert>`
//! - Einstellungen `alerts` → [`AlertConfig`] (Regeln, Kanäle, Wiederholung)

use serde::{Deserialize, Serialize};

use crate::events::Severity;

/// Eintrag der Momentaufnahme `alerts.active`: ein gerade laufender Alarm.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ActiveAlert {
    /// Eindeutig je Regel und Objekt.
    pub id: String,
    /// Name der Regel.
    pub rule: String,
    pub severity: Severity,
    pub title: String,
    pub message: String,
    /// Betroffenes Objekt (Container, Gerät, Ziel).
    pub subject: String,
    /// Seit wann (Unix-Millisekunden).
    pub since: i64,
    /// Auslösender Wert, schon formatiert (z. B. `93.4 %`).
    pub value: String,
    /// Diagramm, das den Alarm zeigt.
    pub chart: Option<String>,
    /// ID der Regel.
    pub rule_id: String,
    /// Gruppe der Regel (`System`, `Container` …).
    pub group: String,
    /// Zuletzt geprüft (Unix-Millisekunden).
    pub updated: i64,
}

// ------------------------------------------------------------------ Regeln

/// Vergleich: Alarm, wenn der Wert über / unter der Schwelle liegt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cmp {
    #[default]
    Above,
    Below,
}

impl Cmp {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Above => ">",
            Self::Below => "<",
        }
    }

    /// Wert verletzt die Schwelle.
    pub fn hit(self, v: f64, threshold: f64) -> bool {
        match self {
            Self::Above => v > threshold,
            Self::Below => v < threshold,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    /// Messwerte eines Diagramm-Kontexts.
    #[default]
    Metric,
    /// Zustand von Containern oder Datenquellen.
    State,
}

/// Zustandsprüfungen (für [`RuleKind::State`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateCheck {
    /// Container mit Fehlercode beendet (nicht durch `docker stop`).
    #[default]
    ContainerExited,
    /// Healthcheck meldet „ungesund“.
    ContainerUnhealthy,
    /// Neustarts im Fenster: Warn-/Kritisch-Schwelle = Anzahl Neustarts.
    RestartLoop,
    /// Vom Kernel wegen Speichermangel beendet – bleibt `window` Sekunden stehen.
    ContainerOom,
    /// Speicher in Prozent der Grenze des Containers.
    MemLimit,
    /// Diagramme liefern keine Werte mehr: Schwellen = Sekunden ohne Wert.
    NoData,
}

impl StateCheck {
    pub const ALL: [Self; 6] =
        [Self::ContainerExited, Self::ContainerUnhealthy, Self::RestartLoop, Self::ContainerOom, Self::MemLimit, Self::NoData];

    pub fn label(self) -> &'static str {
        match self {
            Self::ContainerExited => "Container unerwartet beendet",
            Self::ContainerUnhealthy => "Container ungesund",
            Self::RestartLoop => "Neustart-Schleife",
            Self::ContainerOom => "Speichermangel (OOM)",
            Self::MemLimit => "Speicher nahe Grenze",
            Self::NoData => "Daten bleiben aus",
        }
    }

    /// Die Prüfung nutzt Schwellen (sonst nur die Schwere der Regel).
    pub fn uses_thresholds(self) -> bool {
        matches!(self, Self::RestartLoop | Self::MemLimit | Self::NoData)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::ContainerExited => "container_exited",
            Self::ContainerUnhealthy => "container_unhealthy",
            Self::RestartLoop => "restart_loop",
            Self::ContainerOom => "container_oom",
            Self::MemLimit => "mem_limit",
            Self::NoData => "no_data",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.key() == s)
    }
}

/// Eine Alarmregel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    /// Eindeutig; Standardregeln haben feste IDs.
    pub id: String,
    pub name: String,
    /// Gruppe für die Anzeige (`System`, `Datenträger`, `Container` …).
    pub group: String,
    pub description: String,
    pub enabled: bool,
    /// Standardregel (nicht löschbar, zurücksetzbar).
    pub builtin: bool,
    pub kind: RuleKind,
    /// Kontext(e) des Katalogs, mit Komma getrennt; `*` am Ende als Platzhalter (`disk.*`).
    pub context: String,
    /// Wert aus den Dimensionen, siehe [`Expr`]: `used`, `rx+tx`, `*`, `max:*`, `used/used+avail`.
    pub expr: String,
    /// Label-Filter: `app=nrz-git, device!=loop*` – leer = alle Instanzen.
    pub filter: String,
    pub cmp: Cmp,
    pub warn: Option<f64>,
    pub crit: Option<f64>,
    /// Mittel über so viele Sekunden (0 = neuester Wert).
    pub window: u32,
    /// So lange muss die Schwelle verletzt sein, bevor der Alarm ausgelöst wird (Sekunden).
    pub duration: u32,
    /// Abstand zur Schwelle, ab dem der Alarm wieder endet (gleiche Einheit wie der Wert).
    pub hysteresis: f64,
    /// Für Zustandsregeln.
    pub check: StateCheck,
    /// Schwere bei Zustandsregeln ohne Schwellen.
    pub severity: Severity,
    /// Benachrichtigungen senden.
    pub notify: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            group: "Eigene".to_string(),
            description: String::new(),
            enabled: true,
            builtin: false,
            kind: RuleKind::Metric,
            context: String::new(),
            expr: "*".to_string(),
            filter: String::new(),
            cmp: Cmp::Above,
            warn: None,
            crit: None,
            window: 60,
            duration: 60,
            hysteresis: 0.0,
            check: StateCheck::ContainerExited,
            severity: Severity::Warn,
            notify: true,
        }
    }
}

impl Rule {
    /// Kontext passt zu einem der Muster der Regel.
    pub fn matches_context(&self, context: &str) -> bool {
        self.context.split(',').map(str::trim).filter(|p| !p.is_empty()).any(|p| glob(p, context))
    }

    /// Kurzbeschreibung der Bedingung, z. B. `> 85 / 95`.
    pub fn condition(&self) -> String {
        let f = |v: Option<f64>| v.map_or_else(|| "–".to_string(), |v| crate::model::trim(v, 2));
        format!("{} {} / {}", self.cmp.symbol(), f(self.warn), f(self.crit))
    }
}

/// Einfacher Platzhalter-Vergleich: `*` steht für beliebig viele Zeichen.
pub fn glob(pattern: &str, s: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == s;
    }
    let mut rest = s;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(p) => rest = &rest[p + part.len()..],
                None => return false,
            }
        }
    }
    true
}

/// Ein Label-Filter: `key=wert`, `key!=wert` (Platzhalter `*` erlaubt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelFilter {
    pub key: String,
    pub value: String,
    pub negate: bool,
}

/// `app=nrz-git, device!=loop*` → Filter. Unverständliche Teile werden übergangen.
pub fn parse_filter(s: &str) -> Vec<LabelFilter> {
    s.split(',')
        .filter_map(|part| {
            let part = part.trim();
            if let Some((k, v)) = part.split_once("!=") {
                Some(LabelFilter { key: k.trim().to_string(), value: v.trim().to_string(), negate: true })
            } else {
                part.split_once('=').map(|(k, v)| LabelFilter { key: k.trim().to_string(), value: v.trim().to_string(), negate: false })
            }
        })
        .filter(|f| !f.key.is_empty())
        .collect()
}

/// Wert aus den Dimensionen eines Diagramms.
///
/// - `used` – eine Dimension
/// - `rx+tx` – Summe, `*` = alle Dimensionen
/// - `max:*` – grösster Wert der genannten Dimensionen
/// - `used/used+avail` – Anteil in Prozent (Zähler / Nenner × 100); `@cores` = Anzahl CPU-Kerne
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Expr {
    pub max: bool,
    pub num: Vec<String>,
    /// Leer = kein Anteil.
    pub den: Vec<String>,
}

impl Expr {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let (max, s) = match s.strip_prefix("max:") {
            Some(r) => (true, r.trim()),
            None => (false, s),
        };
        let list = |p: &str| -> Result<Vec<String>, String> {
            let items: Vec<String> = p.split('+').map(|x| x.trim().to_string()).collect();
            if items.iter().any(String::is_empty) {
                return Err(format!("Leerer Teil in `{p}`"));
            }
            Ok(items)
        };
        if s.is_empty() {
            return Err("Ausdruck fehlt".to_string());
        }
        let (num, den) = match s.split_once('/') {
            Some((a, b)) => (list(a)?, list(b)?),
            None => (list(s)?, Vec::new()),
        };
        Ok(Self { max, num, den })
    }

    pub fn is_ratio(&self) -> bool {
        !self.den.is_empty()
    }

    /// Wert ausrechnen. `get(dim)` liefert den Wert einer Dimension, `all()` alle Werte,
    /// `cores` die Anzahl Kerne. `None`, wenn Werte fehlen oder der Nenner 0 ist.
    pub fn eval(&self, get: &dyn Fn(&str) -> Option<f64>, all: &dyn Fn() -> Vec<f64>, cores: f64) -> Option<f64> {
        let side = |items: &[String], max: bool| -> Option<f64> {
            let mut vals = Vec::new();
            for it in items {
                match it.as_str() {
                    "*" => vals.extend(all()),
                    "@cores" => vals.push(cores),
                    d => vals.push(get(d)?),
                }
            }
            if vals.is_empty() {
                return None;
            }
            Some(if max { vals.into_iter().fold(f64::MIN, f64::max) } else { vals.into_iter().sum() })
        };
        let n = side(&self.num, self.max)?;
        if self.den.is_empty() {
            return Some(n);
        }
        let d = side(&self.den, false)?;
        (d > 0.0).then(|| n / d * 100.0)
    }
}

// ------------------------------------------------------------------ Benachrichtigungen

/// Maske für Geheimnisse in Antworten.
pub const MASK: &str = "••••";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    #[default]
    Ntfy,
    Webhook,
    Discord,
    Telegram,
}

impl ChannelKind {
    pub const ALL: [Self; 4] = [Self::Ntfy, Self::Webhook, Self::Discord, Self::Telegram];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ntfy => "ntfy",
            Self::Webhook => "Webhook",
            Self::Discord => "Discord",
            Self::Telegram => "Telegram",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Ntfy => "ntfy",
            Self::Webhook => "webhook",
            Self::Discord => "discord",
            Self::Telegram => "telegram",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.key() == s)
    }
}

/// Ein Benachrichtigungskanal. `token` (und bei Discord/Webhook die URL) kommen nur maskiert zurück.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Channel {
    pub id: String,
    pub kind: ChannelKind,
    pub name: String,
    pub enabled: bool,
    /// ntfy-Server, Webhook- oder Discord-URL.
    pub url: String,
    /// ntfy-Topic.
    pub topic: String,
    /// ntfy-Zugangstoken oder Telegram-Bot-Token.
    pub token: String,
    pub chat_id: String,
    /// Erst ab dieser Schwere senden.
    pub min_severity: Severity,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: ChannelKind::Ntfy,
            name: String::new(),
            enabled: true,
            url: String::new(),
            topic: String::new(),
            token: String::new(),
            chat_id: String::new(),
            min_severity: Severity::Warn,
        }
    }
}

/// Einstellungen unter dem Schlüssel `alerts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertConfig {
    /// Eigene Regeln und geänderte Standardregeln.
    pub rules: Vec<Rule>,
    pub channels: Vec<Channel>,
    /// Bei laufendem Alarm erneut erinnern (Minuten, 0 = nie).
    pub repeat_minutes: u32,
    /// Höchstens so viele Erinnerungen je Alarm.
    pub max_repeats: u32,
    /// „Behoben“ melden.
    pub notify_resolved: bool,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self { rules: Vec::new(), channels: Vec::new(), repeat_minutes: 60, max_repeats: 3, notify_resolved: true }
    }
}

/// Regel mit aktuellem Zustand (`GET /api/v1/alerts/rules`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RuleStatus {
    pub rule: Rule,
    /// Laufende Alarme dieser Regel.
    pub firing: u32,
    /// Geprüfte Objekte (Diagramme, Container).
    pub checked: u32,
    /// Einheit für Schwellen (bei Anteilen Prozent).
    pub unit: crate::model::Unit,
    /// Fehler im Ausdruck.
    pub error: Option<String>,
}

/// Einstellungen ohne Regeln (`GET/PUT /api/v1/alerts/config`), Geheimnisse maskiert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyConfig {
    pub channels: Vec<Channel>,
    pub repeat_minutes: u32,
    pub max_repeats: u32,
    pub notify_resolved: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        let d = AlertConfig::default();
        Self { channels: d.channels, repeat_minutes: d.repeat_minutes, max_repeats: d.max_repeats, notify_resolved: d.notify_resolved }
    }
}

/// Antwort auf den Test-Knopf.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exprs() {
        let get = |d: &str| match d {
            "used" => Some(30.0),
            "avail" => Some(70.0),
            "a" => Some(5.0),
            _ => None,
        };
        let all = || vec![30.0, 70.0, 5.0];
        let e = Expr::parse("used/used+avail").unwrap();
        assert_eq!(e.eval(&get, &all, 4.0), Some(30.0));
        assert_eq!(Expr::parse("*").unwrap().eval(&get, &all, 4.0), Some(105.0));
        assert_eq!(Expr::parse("max:*").unwrap().eval(&get, &all, 4.0), Some(70.0));
        assert_eq!(Expr::parse("a/@cores").unwrap().eval(&get, &all, 4.0), Some(125.0));
        assert_eq!(Expr::parse("x").unwrap().eval(&get, &all, 4.0), None);
        assert!(Expr::parse("a+").is_err());
    }

    #[test]
    fn globs_and_filters() {
        assert!(glob("disk.*", "disk.space"));
        assert!(glob("loop*", "loop3"));
        assert!(!glob("loop*", "nvme0"));
        assert!(glob("*cache*", "build-cache-1"));
        let f = parse_filter("app=nrz-git, device!=loop*");
        assert_eq!(f.len(), 2);
        assert!(f[1].negate);
        let r = Rule { context: "cpu.temp, disk.temp".into(), ..Rule::default() };
        assert!(r.matches_context("disk.temp"));
        assert!(!r.matches_context("disk.io"));
    }
}
