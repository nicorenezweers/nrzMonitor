//! Gemeinsame Typen von nrzMonitor: API-Formate, Metrik-Modell und der Katalog der bekannten
//! Diagramme. Wird vom Server (`server/`) und der Oberfläche (`web/`, WASM) benutzt – darum hier
//! nur `serde`, keine schweren Abhängigkeiten.
//!
//! Wem welche Datei gehört, steht in `ARCHITECTURE.md`.

pub mod api;
pub mod catalog;
pub mod events;
pub mod model;
pub mod query;

// Bereichs-Typen (Momentaufnahmen, Einstellungen, Listen) – je Bereich eine Datei.
pub mod alerts;
pub mod cloudflare;
pub mod dns;
pub mod docker;
pub mod netbird;
pub mod power;
pub mod probes;
pub mod storage;
pub mod system;

pub use model::{ChartKind, ChartMeta, DimMeta, SeriesId, Unit};
