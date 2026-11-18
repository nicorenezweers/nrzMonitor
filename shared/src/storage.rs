//! Typen des Bereichs `storage`: gestufter Speicher, Einstellungen und Auswertungen.
//! Besitzer und Inhalt: siehe `ARCHITECTURE.md` (Bereich 1).

use serde::{Deserialize, Serialize};

use crate::query::Agg;

/// Stufen des Archivs: 0 = Sekunden, 1 = Minuten, 2 = Stunden.
pub const TIERS: usize = 3;
/// Auflösung je Stufe in Sekunden.
pub const TIER_RES: [u32; TIERS] = [1, 60, 3600];
/// Anzeigenamen je Stufe.
pub const TIER_NAMES: [&str; TIERS] = ["Sekunden", "Minuten", "Stunden"];

/// Einstellungen (`core.settings` Schlüssel `storage`, `GET/PUT /api/v1/storage/settings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageSettings {
    /// Aufbewahrung je Stufe in Sekunden (Sekunden-, Minuten-, Stundenwerte).
    pub retention: [u64; TIERS],
    /// Obergrenze für den ganzen Speicher in Bytes (0 = keine). Wird sie erreicht, fällt zuerst das
    /// Älteste der feinsten Stufe weg.
    pub max_bytes: u64,
    /// Aufbewahrung der Ereignisse in Sekunden.
    pub event_retention: u64,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            retention: [86_400, 30 * 86_400, 2 * 365 * 86_400],
            max_bytes: 2 * 1024 * 1024 * 1024,
            event_retention: 90 * 86_400,
        }
    }
}

impl StorageSettings {
    /// Werte in vernünftige Grenzen bringen (Sekunden mind. 1 h, Minuten mind. 1 Tag …).
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        let min = [3600, 86_400, 7 * 86_400];
        let max = [30 * 86_400, 2 * 365 * 86_400, 20 * 365 * 86_400];
        for i in 0..TIERS {
            self.retention[i] = self.retention[i].clamp(min[i], max[i]);
        }
        if self.max_bytes != 0 {
            self.max_bytes = self.max_bytes.max(64 * 1024 * 1024);
        }
        self.event_retention = self.event_retention.clamp(86_400, 10 * 365 * 86_400);
        self
    }
}

/// Belegung einer Stufe.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TierStats {
    pub tier: u8,
    /// Auflösung in Sekunden.
    pub resolution: u32,
    /// Aufbewahrung in Sekunden (aus den Einstellungen).
    pub retention: u64,
    /// Zeilen (= Werte bei Stufe 0; bei Stufe 1/2 je Zeile Mittel, Minimum, Maximum).
    pub rows: u64,
    /// Bytes der abgeschlossenen Parquet-Dateien.
    pub bytes: u64,
    pub files: u64,
    /// Noch offene Zeilen in DuckDB (werden später als Datei abgeschlossen).
    pub hot_rows: u64,
    /// Ältester und neuester Zeitpunkt (Unix-Sekunden, 0 = leer).
    pub oldest: i64,
    pub newest: i64,
    /// Bytes je Zeile in den Dateien.
    pub bytes_per_value: f64,
}

/// `GET /api/v1/storage/stats`, auch als Momentaufnahme `storage.stats`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageStats {
    /// Unix-Sekunden der Messung.
    pub ts: i64,
    pub tiers: Vec<TierStats>,
    /// Grösse der DuckDB-Datei (offene Abschnitte, Ereignisse, Katalog) in Bytes.
    pub db_bytes: u64,
    /// Alles zusammen (DuckDB + Dateien) in Bytes.
    pub total_bytes: u64,
    /// Obergrenze aus den Einstellungen (0 = keine).
    pub max_bytes: u64,
    /// Arbeitsspeicher des Ringpuffers in Bytes.
    pub ring_bytes: u64,
    /// Sekunden im Ringpuffer.
    pub ring_window: u32,
    /// Reihen insgesamt (aktive und stillgelegte).
    pub series: u64,
    /// Aktive Diagramme.
    pub charts: u64,
    /// Angenommene Werte je Sekunde (Mittel der letzten Minute).
    pub samples_per_sec: f64,
    /// Seit dem Start angenommene Werte.
    pub samples_total: u64,
    /// Verworfene Werte (zu spät, Abschnitt schon abgeschlossen).
    pub dropped: u64,
    /// Gespeicherte Ereignisse.
    pub events: u64,
    /// Freier Platz auf dem Datenträger des Datenordners (Bytes, 0 = unbekannt).
    pub disk_free: u64,
}

/// Zeitraum einer Auswertung: wie bei [`crate::query::QueryReq`] (`≤ 0` = relativ).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Window {
    pub after: i64,
    pub before: i64,
}

/// `POST /api/v1/analyze/stats`: Kennzahlen je Reihe der Diagramme.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StatsReq {
    pub charts: Vec<String>,
    pub after: i64,
    pub before: i64,
}

/// Kennzahlen einer Reihe (Dimension eines Diagramms). `None` = keine Werte im Zeitraum.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SeriesStats {
    pub chart: String,
    pub dim: String,
    /// Anzeigename der Dimension.
    pub name: String,
    pub unit: crate::Unit,
    pub count: u64,
    pub min: Option<f64>,
    pub avg: Option<f64>,
    pub max: Option<f64>,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
    pub last: Option<f64>,
    pub stddev: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StatsResp {
    /// Tatsächlicher Zeitraum (Unix-Sekunden).
    pub after: i64,
    pub before: i64,
    /// Benutzte Stufe (255 = Arbeitsspeicher).
    pub tier: u8,
    pub series: Vec<SeriesStats>,
}

/// `POST /api/v1/analyze/top`: Instanzen eines Kontexts ordnen (z. B. `docker.cpu` je Container).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TopReq {
    pub context: String,
    pub after: i64,
    pub before: i64,
    /// Kennzahl je Instanz über den Zeitraum (Standard Mittel).
    pub agg: Agg,
    /// Nur diese Dimension (leer = Summe aller Dimensionen der Instanz).
    pub dim: Option<String>,
    /// Höchstens so viele (Standard 20).
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TopItem {
    pub chart: String,
    /// Name der Instanz (Label `container`, `device` … oder die Diagramm-ID).
    pub instance: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TopResp {
    pub context: String,
    pub unit: crate::Unit,
    pub after: i64,
    pub before: i64,
    pub items: Vec<TopItem>,
}

/// `POST /api/v1/analyze/correlate` („Was hat sich geändert?“): das markierte Fenster
/// `[after, before)` wird mit dem Zeitraum davor verglichen (Standard: viermal so lang).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CorrelateReq {
    pub after: i64,
    pub before: i64,
    /// Beginn des Vergleichszeitraums (0 = automatisch: viermal das Fenster davor).
    pub baseline_after: i64,
    /// Nur Diagramme dieses Bereichs (leer = alle).
    pub section: Option<String>,
    /// Höchstens so viele Reihen (Standard 30).
    pub limit: Option<u32>,
    /// Punkte des Mini-Verlaufs je Reihe (Standard 60, 0 = keiner).
    pub points: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CorrelateItem {
    pub chart: String,
    pub dim: String,
    /// Diagrammtitel und Dimension für die Anzeige.
    pub title: String,
    pub name: String,
    pub unit: crate::Unit,
    /// Veränderung 0–1 (1 = stärkste), danach wird geordnet.
    pub score: f64,
    /// Mittel im Vergleichszeitraum und im Fenster.
    pub before_avg: f64,
    pub window_avg: f64,
    /// Mini-Verlauf über Vergleichszeitraum + Fenster (`spark_split` = erster Punkt im Fenster).
    pub spark: Vec<Option<f64>>,
    pub spark_split: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CorrelateResp {
    pub after: i64,
    pub before: i64,
    pub baseline_after: i64,
    /// Untersuchte Reihen.
    pub examined: u64,
    /// Rechenzeit in Millisekunden.
    pub took_ms: u64,
    pub items: Vec<CorrelateItem>,
}

/// Query-Parameter von `GET /api/v1/export.csv?charts=a,b&after=-3600&before=0&points=0`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportQuery {
    /// Diagramm-IDs, mit Komma getrennt.
    pub charts: String,
    pub after: i64,
    pub before: i64,
    /// Punkte (0 = volle Auflösung der gewählten Stufe, höchstens 100 000).
    pub points: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_limits() {
        let s = StorageSettings::default();
        assert_eq!(s.retention[0], 86_400);
        let bad = StorageSettings { retention: [1, 1, 1], max_bytes: 5, event_retention: 0 }.sanitized();
        assert_eq!(bad.retention, [3600, 86_400, 7 * 86_400]);
        assert_eq!(bad.max_bytes, 64 * 1024 * 1024);
        let parsed: StorageSettings = serde_json::from_str(r#"{"max_bytes":0}"#).unwrap_or_default();
        assert_eq!(parsed.max_bytes, 0);
        assert_eq!(parsed.retention, s.retention);
    }
}
