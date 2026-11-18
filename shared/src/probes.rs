//! Typen des Bereichs `probes`: Ziele (Einstellungen `probes`) und Momentaufnahme `probes.status`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeKind {
    #[default]
    Ping,
    Tcp,
    Http,
}

impl ProbeKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ping => "Ping",
            Self::Tcp => "TCP",
            Self::Http => "HTTP",
        }
    }
}

/// Ein Ziel.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeTarget {
    /// Stabil, klein (`gateway`, `cloudflare-dns` …); wird Diagramm-Instanz.
    pub id: String,
    pub name: String,
    pub kind: ProbeKind,
    /// Ping/TCP: Rechnername oder IP. HTTP: ganze Adresse (`https://…`).
    pub host: String,
    /// TCP: Port.
    pub port: u16,
    pub enabled: bool,
}

/// Einstellungen (`core.settings` Schlüssel `probes`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeSettings {
    /// Standardziele (Gateway, 1.1.1.1, 9.9.9.9) prüfen.
    pub defaults: bool,
    pub targets: Vec<ProbeTarget>,
}

impl Default for ProbeSettings {
    fn default() -> Self {
        Self { defaults: true, targets: Vec::new() }
    }
}

/// Zustand eines Ziels.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeState {
    pub id: String,
    pub name: String,
    pub kind: ProbeKind,
    /// Was geprüft wird (`192.168.1.1`, `1.1.1.1:443`, `https://…`).
    pub target: String,
    /// Standardziel (nicht löschbar, nur über `defaults` abschaltbar).
    pub builtin: bool,
    pub up: bool,
    /// Noch keine Messung.
    pub pending: bool,
    pub latency_ms: Option<f64>,
    /// Verlust in Prozent (letzte Messung).
    pub loss: f64,
    /// HTTP-Status.
    pub status: Option<u16>,
    pub error: Option<String>,
    /// Unix-Sekunden: seit wann im aktuellen Zustand.
    pub since: i64,
    /// Unix-Sekunden der letzten Messung.
    pub last_check: i64,
    /// Verfügbarkeit in Prozent seit dem Start.
    pub uptime_pct: f64,
}

/// Momentaufnahme `probes.status`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbesStatus {
    pub demo: bool,
    pub targets: Vec<ProbeState>,
    /// ICMP geht nicht (fehlendes Recht) – Ping fällt dann auf TCP zurück.
    pub icmp_error: Option<String>,
}
