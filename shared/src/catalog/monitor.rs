//! Vorlagen für nrzMonitor selbst.

use super::{Tpl, d};
use crate::model::{ChartKind::*, Unit::*};

/// Dimensionen: je Sammler eine.
pub const MONITOR_COLLECTORS: Tpl = Tpl::new("monitor.collectors", "monitor", "Dauer je Sammler", Millis, Stacked, &[], 100);
pub const MONITOR_SERIES: Tpl = Tpl::new(
    "monitor.series", "monitor", "Diagramme und Reihen", Count, Line, &[d("charts", "Diagramme"), d("series", "Reihen")], 110,
).every(10);
pub const MONITOR_INGEST: Tpl = Tpl::new(
    "monitor.ingest", "monitor", "Gespeicherte Werte", PerSec, Area, &[d("samples", "Werte")], 120,
);
pub const MONITOR_CPU: Tpl = Tpl::new(
    "monitor.cpu", "monitor", "Eigene CPU-Last", Percent, Stacked, &[d("user", "Benutzer"), d("system", "System")], 130,
);
pub const MONITOR_MEM: Tpl = Tpl::new(
    "monitor.mem", "monitor", "Eigener Arbeitsspeicher", Bytes, Area, &[d("rss", "Belegt")], 140,
).every(5);
pub const MONITOR_DISK: Tpl = Tpl::new(
    "monitor.disk", "monitor", "Belegter Speicherplatz", Bytes, Stacked,
    &[d("hot", "Aktuell (DuckDB)"), d("raw", "Sekunden"), d("minute", "Minuten"), d("hour", "Stunden"), d("logs", "Protokolle")], 150,
).every(60);
pub const MONITOR_CLIENTS: Tpl = Tpl::new(
    "monitor.clients", "monitor", "Verbundene Browser", Count, Line, &[d("clients", "Verbindungen")], 160,
).every(10);

pub const ALL: &[Tpl] = &[
    MONITOR_COLLECTORS, MONITOR_SERIES, MONITOR_INGEST, MONITOR_CPU, MONITOR_MEM, MONITOR_DISK, MONITOR_CLIENTS,
];
