//! Typen des Bereichs `netbird`: Peers (`netbird.peers`), Zustand (`netbird.status`), Aktivität.

use serde::{Deserialize, Serialize};

/// Standard-Adresse der Management-API.
pub const DEFAULT_URL: &str = "https://api.netbird.io";

/// Gespeicherte Einstellungen (`core.settings` Schlüssel `netbird`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetbirdSettings {
    pub enabled: bool,
    /// Management-API (leer = `NRZMON_NETBIRD_URL` oder `https://api.netbird.io`).
    pub url: String,
    /// Persönliches Zugangs-Token (leer = `NRZMON_NETBIRD_TOKEN`).
    pub token: String,
}

impl Default for NetbirdSettings {
    fn default() -> Self {
        Self { enabled: true, url: String::new(), token: String::new() }
    }
}

/// Ein Peer (Momentaufnahme `netbird.peers`: `Vec<Peer>`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub ip: String,
    pub connection_ip: String,
    pub dns_label: String,
    pub connected: bool,
    /// Unix-Sekunden.
    pub last_seen: i64,
    pub os: String,
    pub version: String,
    pub groups: Vec<String>,
    pub country: String,
    pub city: String,
    pub user: String,
    pub ssh: bool,
    pub login_expired: bool,
    pub approval_required: bool,
}

/// Eintrag des Aktivitätsprotokolls.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Activity {
    pub id: String,
    /// Unix-Sekunden.
    pub ts: i64,
    pub activity: String,
    pub code: String,
    pub initiator: String,
    pub target: String,
}

/// Bestand (Anzahl je Art).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Inventory {
    pub users: u32,
    pub groups: u32,
    pub policies: u32,
    pub routes: u32,
    pub networks: u32,
    pub setup_keys: u32,
    pub nameservers: u32,
}

/// Momentaufnahme `netbird.status`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NetbirdStatus {
    pub configured: bool,
    pub demo: bool,
    pub ok: bool,
    pub error: Option<String>,
    /// Teile, die das Token nicht lesen darf.
    pub hints: Vec<String>,
    pub url: String,
    /// Unix-Millisekunden.
    pub last_poll: i64,
    pub peers: u32,
    pub connected: u32,
    pub inventory: Inventory,
    pub users: Vec<String>,
    pub groups: Vec<(String, u32)>,
    pub policies: Vec<(String, bool)>,
    pub routes: Vec<String>,
    pub networks: Vec<String>,
    pub setup_keys: Vec<(String, bool)>,
    pub nameservers: Vec<String>,
    /// Neueste zuerst (höchstens 100).
    pub activity: Vec<Activity>,
}
