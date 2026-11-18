//! Allgemeine API-Formate.

use serde::{Deserialize, Serialize};

/// `GET /api/v1/info`
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Info {
    pub version: String,
    pub hostname: String,
    /// Beispieldaten statt echter Sammler (`NRZMON_DEMO=1` oder kein Linux).
    pub demo: bool,
    /// Unix-Sekunden.
    pub started_at: i64,
    pub now: i64,
    /// Sekunden zwischen zwei Messungen der lokalen Sammler.
    pub interval: u32,
    /// So viele Sekunden liegen in voller Auflösung im Arbeitsspeicher.
    pub live_window: u32,
    /// Anmeldung nötig (`NRZMON_PASSWORD` gesetzt).
    pub auth: bool,
    /// Diese Sitzung ist angemeldet (oder es braucht keine Anmeldung).
    pub authenticated: bool,
    /// Zählt bei jeder Katalogänderung hoch.
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LoginReq {
    pub password: String,
}

/// Fehlerantwort aller Endpunkte.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}

/// Momentaufnahme (`GET /api/v1/snap/{key}`): letzter bekannter Stand einer Liste oder eines Zustands.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Snapshot<T> {
    /// Unix-Millisekunden der Aufnahme (0 = noch keine).
    pub ts: i64,
    pub data: T,
}
