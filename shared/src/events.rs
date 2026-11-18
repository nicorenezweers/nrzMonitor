//! Ereignisse: alles, was zu einem Zeitpunkt passiert ist (Container gestartet, Alarm, Peer offline …).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    #[default]
    Info,
    /// Etwas ist wieder in Ordnung.
    Ok,
    Warn,
    Crit,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Event {
    /// Vom Speicher vergeben (0 = noch nicht gespeichert).
    #[serde(default)]
    pub id: u64,
    /// Unix-Millisekunden.
    pub ts: i64,
    /// Herkunft: `docker`, `alert`, `system`, `dns`, `cloudflare`, `netbird`, `probe`, `monitor`.
    pub source: String,
    /// Art, z. B. `container.start`, `container.oom`, `alert.raised`, `peer.offline`.
    pub kind: String,
    pub severity: Severity,
    /// Betroffenes Objekt (Containername, Peer, Gerät).
    #[serde(default)]
    pub subject: String,
    pub title: String,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
}

impl Event {
    #[must_use]
    pub fn new(source: &str, kind: &str, severity: Severity, subject: &str, title: impl Into<String>) -> Self {
        Self {
            source: source.to_string(),
            kind: kind.to_string(),
            severity,
            subject: subject.to_string(),
            title: title.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn message(mut self, m: impl Into<String>) -> Self {
        self.message = m.into();
        self
    }

    #[must_use]
    pub fn label(mut self, k: &str, v: impl Into<String>) -> Self {
        self.labels.insert(k.to_string(), v.into());
        self
    }
}

/// Filter für `GET /api/v1/events` (alle Felder optional, als Query-Parameter).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EventQuery {
    /// Unix-Millisekunden.
    pub after: Option<i64>,
    pub before: Option<i64>,
    pub source: Option<String>,
    pub kind: Option<String>,
    pub subject: Option<String>,
    /// Mindest-Schwere.
    pub severity: Option<Severity>,
    /// Freitext (Titel, Meldung, Objekt).
    pub q: Option<String>,
    /// Höchstens so viele (Standard 200, neueste zuerst).
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EventPage {
    /// Neueste zuerst.
    pub events: Vec<Event>,
    pub total: u64,
}
