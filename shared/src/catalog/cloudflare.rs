//! Vorlagen für Cloudflare (GraphQL-Analytics und REST). Werte je Minute, treffen verzögert ein.

use super::{Tpl, d};
use crate::model::{ChartKind::*, Unit::*};

pub const STEP: u32 = 60;

pub const CF_REQUESTS: Tpl = Tpl::new(
    "cf.requests", "cloudflare", "Anfragen", PerMin, Stacked, &[d("cached", "Aus dem Cache"), d("uncached", "Vom Ursprung")], 100,
).per("zone").every(STEP);
pub const CF_BANDWIDTH: Tpl = Tpl::new(
    "cf.bandwidth", "cloudflare", "Datenmenge je Minute", Bytes, Stacked, &[d("cached", "Aus dem Cache"), d("uncached", "Vom Ursprung")], 110,
).per("zone").every(STEP);
pub const CF_STATUS: Tpl = Tpl::new(
    "cf.status", "cloudflare", "Antworten nach Status", PerMin, Stacked,
    &[d("s2xx", "2xx"), d("s3xx", "3xx"), d("s4xx", "4xx"), d("s5xx", "5xx")], 120,
).per("zone").every(STEP);
pub const CF_THREATS: Tpl = Tpl::new(
    "cf.threats", "cloudflare", "Abgewehrte Bedrohungen", PerMin, Area, &[d("threats", "Bedrohungen")], 130,
).per("zone").every(STEP);
pub const CF_VISITORS: Tpl = Tpl::new(
    "cf.visitors", "cloudflare", "Besucher und Seitenaufrufe", PerMin, Line, &[d("visitors", "Besucher"), d("pageviews", "Seitenaufrufe")], 140,
).per("zone").every(STEP);
/// Dimensionen: je Land eine (häufigste zuerst, Rest als `other`).
pub const CF_COUNTRIES: Tpl = Tpl::new("cf.countries", "cloudflare", "Anfragen nach Land", PerMin, Stacked, &[], 150).per("zone").every(STEP);
pub const CF_FIREWALL: Tpl = Tpl::new(
    "cf.firewall", "cloudflare", "Firewall-Ereignisse", PerMin, Stacked,
    &[d("block", "Blockiert"), d("challenge", "Prüfung"), d("log", "Protokolliert"), d("other", "Andere")], 160,
).per("zone").every(STEP);
pub const CF_TUNNEL_CONNS: Tpl = Tpl::new(
    "cf.tunnel.conns", "cloudflare", "Tunnel: Verbindungen", Count, Line, &[d("connections", "Verbindungen")], 200,
).per("tunnel").every(STEP);
pub const CF_TUNNEL_UP: Tpl = Tpl::new(
    "cf.tunnel.up", "cloudflare", "Tunnel: erreichbar", State, Area, &[d("up", "Verbunden")], 210,
).per("tunnel").every(STEP);

pub const ALL: &[Tpl] = &[
    CF_REQUESTS, CF_BANDWIDTH, CF_STATUS, CF_THREATS, CF_VISITORS, CF_COUNTRIES, CF_FIREWALL, CF_TUNNEL_CONNS, CF_TUNNEL_UP,
];
