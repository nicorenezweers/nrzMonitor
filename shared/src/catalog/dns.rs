//! Vorlagen für DNS (nrzNetwork über MCP): alle Anfragen im Netzwerk.

use super::{Tpl, d};
use crate::model::{ChartKind::*, Unit::*};

/// Takt der DNS-Diagramme in Sekunden.
pub const STEP: u32 = 10;

pub const DNS_QUERIES: Tpl = Tpl::new(
    "dns.queries", "dns", "Anfragen nach Antwortquelle", PerSec, Stacked,
    &[
        d("forwarded", "Weitergeleitet"), d("cached", "Cache"), d("blocked", "Gesperrt"),
        d("authoritative", "Eigene Zonen"), d("local", "Lokal"), d("failed", "Fehlgeschlagen"), d("dropped", "Verworfen"),
    ],
    100,
).every(STEP);
pub const DNS_RCODES: Tpl = Tpl::new(
    "dns.rcodes", "dns", "Antwortcodes", PerSec, Stacked,
    &[d("noerror", "NOERROR"), d("nxdomain", "NXDOMAIN"), d("servfail", "SERVFAIL"), d("refused", "REFUSED"), d("other", "Andere")],
    110,
).every(STEP);
/// Dimensionen: je Abfragetyp eine (`A`, `AAAA`, `HTTPS` …).
pub const DNS_QTYPES: Tpl = Tpl::new("dns.qtypes", "dns", "Abfragetypen", PerSec, Stacked, &[], 120).every(STEP);
pub const DNS_LATENCY: Tpl = Tpl::new(
    "dns.latency", "dns", "Antwortzeit", Millis, Line, &[d("avg", "Mittel"), d("p95", "95 %"), d("max", "Maximum")], 130,
).every(STEP);
pub const DNS_BLOCKED_PCT: Tpl = Tpl::new(
    "dns.blocked_pct", "dns", "Anteil gesperrt", Percent, Area, &[d("blocked", "Gesperrt")], 140,
).every(STEP);
pub const DNS_CLIENTS: Tpl = Tpl::new(
    "dns.clients", "dns", "Aktive Clients", Count, Line, &[d("clients", "Clients")], 150,
).every(60);
/// Dimensionen: je Client eine (aktivste zuerst, Rest als `other`).
pub const DNS_CLIENT_QUERIES: Tpl = Tpl::new("dns.client_queries", "dns", "Anfragen je Client", PerSec, Stacked, &[], 160).every(STEP);
pub const DNS_CACHE: Tpl = Tpl::new(
    "dns.cache", "dns", "Cache-Einträge", Count, Line, &[d("entries", "Einträge")], 170,
).every(60);
pub const DNS_BLOCKLIST: Tpl = Tpl::new(
    "dns.blocklist", "dns", "Gesperrte Domains", Count, Line, &[d("domains", "Domains")], 180,
).every(300);
pub const DNS_UPSTREAM_LATENCY: Tpl = Tpl::new(
    "dns.upstream.latency", "dns", "Weiterleitung: Antwortzeit", Millis, Line, &[d("avg", "Mittel")], 200,
).per("upstream").every(60);
pub const DNS_UPSTREAM_ANSWERS: Tpl = Tpl::new(
    "dns.upstream.answers", "dns", "Weiterleitung: Antworten", PerSec, Stacked, &[d("ok", "In Ordnung"), d("failed", "Fehler")], 210,
).per("upstream").every(60);

pub const ALL: &[Tpl] = &[
    DNS_QUERIES, DNS_RCODES, DNS_QTYPES, DNS_LATENCY, DNS_BLOCKED_PCT, DNS_CLIENTS, DNS_CLIENT_QUERIES,
    DNS_CACHE, DNS_BLOCKLIST, DNS_UPSTREAM_LATENCY, DNS_UPSTREAM_ANSWERS,
];
