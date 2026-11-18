//! Abfragen von Zeitreihen (`POST /api/v1/query`) und das Live-Protokoll (`/api/v1/live`, WebSocket).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::events::Event;

/// Wie mehrere Rohwerte zu einem Punkt verdichtet werden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agg {
    #[default]
    Avg,
    Min,
    Max,
    Sum,
    Last,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QueryReq {
    /// Diagramm-IDs.
    pub charts: Vec<String>,
    /// Beginn in Unix-Sekunden; `≤ 0` = relativ zu `before` (`-900` = die 15 Minuten davor).
    pub after: i64,
    /// Ende in Unix-Sekunden; `≤ 0` = relativ zu jetzt (`0` = jetzt).
    pub before: i64,
    /// Gewünschte Punkte je Reihe (0 = 300). Der Server wählt daraus Raster und Speicherstufe.
    pub points: u32,
    pub agg: Agg,
    /// Min/Max je Punkt mitliefern (für das Band bei verdichteten Daten).
    pub envelope: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct QueryResp {
    /// Serverzeit (Unix-Sekunden).
    pub now: i64,
    pub charts: Vec<ChartData>,
}

/// Werte eines Diagramms auf einem festen Raster: Punkt `i` liegt bei `start + i * step`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartData {
    pub id: String,
    /// Dimensionen in der Reihenfolge von `values`.
    pub dims: Vec<String>,
    pub start: i64,
    /// Sekunden je Punkt.
    pub step: u32,
    /// Quelle: 255 = Arbeitsspeicher, 0 = Rohdaten, 1 = Minuten, 2 = Stunden.
    pub tier: u8,
    /// Je Dimension eine Reihe, `null` = kein Wert.
    pub values: Vec<Vec<Option<f64>>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub min: Vec<Vec<Option<f64>>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub max: Vec<Vec<Option<f64>>>,
}

impl ChartData {
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.first().map_or(0, Vec::len)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Zeit des Punkts `i` (Unix-Sekunden).
    #[must_use]
    pub fn time(&self, i: usize) -> i64 {
        self.start + i as i64 * i64::from(self.step)
    }
}

/// Stufe `tier` in [`ChartData`] für Daten aus dem Arbeitsspeicher.
pub const TIER_RAM: u8 = 255;

/// Nachricht des Browsers an `/api/v1/live`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum LiveCmd {
    /// Abonnieren (ergänzt die bisherigen Abos).
    Sub {
        #[serde(default)]
        charts: Vec<String>,
        /// Schlüssel von Momentaufnahmen, z. B. `docker.containers`.
        #[serde(default)]
        snaps: Vec<String>,
        #[serde(default)]
        events: bool,
    },
    Unsub {
        #[serde(default)]
        charts: Vec<String>,
        #[serde(default)]
        snaps: Vec<String>,
        #[serde(default)]
        events: bool,
    },
    Ping,
}

/// Nachricht des Servers über `/api/v1/live`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "lowercase")]
pub enum LiveMsg {
    /// Neue Werte: je abonniertem Diagramm die Werte in der Reihenfolge seiner Dimensionen.
    Tick { t: i64, c: BTreeMap<String, Vec<Option<f64>>> },
    /// Eine Momentaufnahme hat sich geändert.
    Snap { key: String, ts: i64, data: serde_json::Value },
    Event { ev: Event },
    /// Der Katalog hat sich geändert (Diagramme oder Dimensionen neu/entfernt) → `/charts` neu laden.
    Catalog { generation: u64 },
    Pong,
}
