//! Vorlagen für Erreichbarkeits-Prüfungen (Ping, TCP, HTTP) – je Ziel ein Diagramm.

use super::{Tpl, d};
use crate::model::{ChartKind::*, Unit::*};

pub const PROBE_LATENCY: Tpl = Tpl::new(
    "probe.latency", "probes", "Antwortzeit", Millis, Line, &[d("latency", "Antwortzeit")], 100,
).per("target").every(10);
pub const PROBE_LOSS: Tpl = Tpl::new(
    "probe.loss", "probes", "Ausfälle", Percent, Area, &[d("loss", "Verlust")], 110,
).per("target").every(10);
pub const PROBE_UP: Tpl = Tpl::new(
    "probe.up", "probes", "Erreichbar", State, Area, &[d("up", "Erreichbar")], 120,
).per("target").every(10);
pub const PROBE_HTTP: Tpl = Tpl::new(
    "probe.http", "probes", "HTTP: Zeitanteile", Millis, Stacked,
    &[d("dns", "DNS"), d("connect", "Verbinden"), d("tls", "TLS"), d("wait", "Warten"), d("transfer", "Übertragen")], 130,
).per("target").every(30);

pub const ALL: &[Tpl] = &[PROBE_LATENCY, PROBE_LOSS, PROBE_UP, PROBE_HTTP];
