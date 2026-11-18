//! Typen des Bereichs `cloudflare`: Zonen, Tunnel und die Momentaufnahme `cloudflare.status`.
//! Die Einstellungen stehen als [`crate::dns::TokenForm`] im Schlüssel `cloudflare`.

use serde::{Deserialize, Serialize};

/// Gespeicherte Einstellungen (`core.settings` Schlüssel `cloudflare`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudflareSettings {
    pub enabled: bool,
    /// API-Token (leer = `NRZMON_CLOUDFLARE_TOKEN`).
    pub token: String,
    /// Nur diese Zonen (Name oder ID); leer = alle, die das Token sieht.
    pub zones: Vec<String>,
    /// Konto für Tunnel; leer = aus den Zonen.
    pub account_id: String,
}

impl Default for CloudflareSettings {
    fn default() -> Self {
        Self { enabled: true, token: String::new(), zones: Vec::new(), account_id: String::new() }
    }
}

/// Eine Zone mit den Summen der letzten 24 Stunden (aus den abgegebenen Minuten).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CfZone {
    pub id: String,
    pub name: String,
    pub plan: String,
    /// `active`, `pending` …
    pub status: String,
    /// Benutzter Datensatz: `httpRequests1mGroups`, `httpRequestsAdaptiveGroups` oder leer.
    pub dataset: String,
    /// Firewall-Datensatz: `firewallEventsAdaptiveGroups`, `firewallEventsAdaptive` oder leer.
    pub firewall_dataset: String,
    /// Fehler oder fehlendes Recht dieser Zone (verständlich formuliert).
    pub error: Option<String>,
    /// Letzte abgegebene Minute (Unix-Sekunden).
    pub last_minute: i64,
    pub requests_24h: f64,
    pub cached_24h: f64,
    pub bytes_24h: f64,
    pub threats_24h: f64,
    pub firewall_24h: f64,
    pub top_countries: Vec<(String, f64)>,
}

/// Ein Cloudflare-Tunnel.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CfTunnel {
    pub id: String,
    pub name: String,
    /// `healthy`, `degraded`, `down`, `inactive`
    pub status: String,
    pub connections: u32,
    /// Rechenzentren der Verbindungen (z. B. `ZRH`, `FRA`).
    pub colos: Vec<String>,
    /// Unix-Sekunden.
    pub status_since: i64,
    pub created_at: String,
}

/// Momentaufnahme `cloudflare.status`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CfStatus {
    pub configured: bool,
    pub demo: bool,
    pub ok: bool,
    /// Gesamtfehler (Token ungültig, nicht erreichbar …).
    pub error: Option<String>,
    /// Hinweise zu fehlenden Rechten.
    pub hints: Vec<String>,
    /// Unix-Millisekunden.
    pub last_poll: i64,
    pub account_id: String,
    pub zones: Vec<CfZone>,
    pub tunnels: Vec<CfTunnel>,
    /// API-Aufrufe seit dem Start.
    pub calls: u64,
    /// Wartet wegen Ratenbegrenzung bis (Unix-Sekunden).
    pub backoff_until: i64,
}
