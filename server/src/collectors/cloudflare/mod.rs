//! Cloudflare: Zonen, HTTP-Analytics je Minute (GraphQL), Firewall-Ereignisse und Tunnel (REST).
//!
//! - Zonen findet das Token selbst (`GET /zones`), Konto für Tunnel aus den Zonen.
//! - Welche GraphQL-Datensätze der Tarif erlaubt, fragt der Sammler je Zone über `settings` ab
//!   (Doku: „Discovery → Settings“). Reihenfolge: `httpRequests1mGroups` (alle Kennzahlen),
//!   sonst `httpRequestsAdaptiveGroups` (ohne Bedrohungen/Seitenaufrufe); Firewall:
//!   `firewallEventsAdaptiveGroups`, sonst die einzelnen Ereignisse `firewallEventsAdaptive`.
//!   Schlägt eine Abfrage mit „kein Zugriff“ fehl, fällt er auf den nächsten Datensatz zurück.
//! - Analytics treffen verzögert ein: jede Minute wird einmal abgegeben, sobald sie
//!   [`LAG`] Sekunden alt ist (`Frame::new(minute)`).
//! - Ratenbegrenzung (HTTP 429 / GraphQL-Kontingent 300 je 5 min): Pause nach `Retry-After`.

mod demo;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, VecDeque};

use serde_json::{Value, json};
use shared::{
    catalog::cloudflare::*,
    cloudflare::{CfStatus, CfTunnel, CfZone, CloudflareSettings},
    dns::TestResult,
    events::{Event, Severity},
};
use tokio::task::JoinHandle;

use crate::{
    collectors::dns::store::{format_time, parse_time},
    config,
    core::Core,
    frame::Frame,
    registry::{ChartDef, ChartHandle},
    util,
};

pub const API: &str = "https://api.cloudflare.com/client/v4";
/// Sekunden, die eine Minute alt sein muss, bevor sie abgegeben wird.
const LAG: i64 = 300;
/// So weit zurück wird beim Start nachgeholt (Ring/Speicher nehmen ≤ 15 min zurück).
const START_BACK: i64 = 13 * 60;
const MAX_ZONES: usize = 10;
const MAX_COUNTRIES: usize = 8;

/// Wirksame Einstellungen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    pub enabled: bool,
    pub token: String,
    pub token_from_env: bool,
    pub zones: Vec<String>,
    pub account_id: String,
}

impl Effective {
    pub fn configured(&self) -> bool {
        self.enabled && !self.token.is_empty()
    }
}

pub fn effective_from(s: &CloudflareSettings) -> Effective {
    let env = config::var("NRZMON_CLOUDFLARE_TOKEN");
    let from_env = s.token.trim().is_empty() && env.is_some();
    Effective {
        enabled: s.enabled,
        token: if from_env { env.unwrap_or_default() } else { s.token.trim().to_string() },
        token_from_env: from_env,
        zones: s.zones.iter().map(|z| z.trim().to_string()).filter(|z| !z.is_empty()).collect(),
        account_id: s.account_id.trim().to_string(),
    }
}

pub fn effective(core: &Core) -> Effective {
    effective_from(&core.settings.get::<CloudflareSettings>("cloudflare"))
}

pub fn start(core: &Core) {
    let core = core.clone();
    tokio::spawn(async move {
        let mut rx = core.settings.subscribe();
        let mut running: Option<(Option<Effective>, JoinHandle<()>)> = None;
        loop {
            let eff = effective(&core);
            let want = eff.configured().then_some(eff);
            if running.as_ref().map(|(e, _)| e) != Some(&want) {
                if let Some((_, h)) = running.take() {
                    h.abort();
                }
                let c = core.clone();
                let h = match want.clone() {
                    Some(e) => tokio::spawn(async move { Runner::new(c, API, e).run().await }),
                    None if core.cfg.demo => tokio::spawn(async move { demo::run(c).await }),
                    None => {
                        core.snapshot("cloudflare.status", &CfStatus::default());
                        tokio::spawn(async {})
                    }
                };
                running = Some((want, h));
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    });
}

// --- Client ------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfError {
    Unreachable(String),
    Auth(String),
    Forbidden(String),
    RateLimited(i64),
    Http(u16, String),
    Api(String),
}

impl std::fmt::Display for CfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(e) => write!(f, "Cloudflare nicht erreichbar: {e}"),
            Self::Auth(m) => write!(f, "Cloudflare lehnt das Token ab ({m}) – Token prüfen"),
            Self::Forbidden(m) => write!(f, "Recht fehlt: {m}"),
            Self::RateLimited(s) => write!(f, "Ratenbegrenzung – Pause für {s} s"),
            Self::Http(s, m) => write!(f, "Cloudflare antwortet mit HTTP {s}: {m}"),
            Self::Api(m) => write!(f, "{m}"),
        }
    }
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
    pub calls: u64,
}

impl Client {
    pub fn new(http: reqwest::Client, base: &str, token: &str) -> Self {
        Self { http, base: base.trim_end_matches('/').to_string(), token: token.to_string(), calls: 0 }
    }

    async fn send(&mut self, req: reqwest::RequestBuilder) -> Result<Value, CfError> {
        self.calls += 1;
        let res = req.bearer_auth(&self.token).send().await.map_err(|e| CfError::Unreachable(short(&e)))?;
        let status = res.status();
        let retry = res
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(60);
        let text = res.text().await.map_err(|e| CfError::Unreachable(short(&e)))?;
        if status.as_u16() == 429 {
            return Err(CfError::RateLimited(retry.clamp(5, 900)));
        }
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let msg = first_error(&v).unwrap_or_else(|| text.chars().take(160).collect());
        match status.as_u16() {
            200..=299 => {}
            401 => return Err(CfError::Auth(msg)),
            403 => return Err(CfError::Forbidden(msg)),
            s => return Err(CfError::Http(s, msg)),
        }
        if v.get("success").and_then(Value::as_bool) == Some(false) {
            let code = v.pointer("/errors/0/code").and_then(Value::as_i64).unwrap_or(0);
            return Err(if matches!(code, 9109 | 10000 | 1000) { CfError::Auth(msg) } else { CfError::Api(msg) });
        }
        Ok(v)
    }

    pub async fn get(&mut self, path: &str) -> Result<Value, CfError> {
        let req = self.http.get(format!("{}{path}", self.base));
        self.send(req).await
    }

    /// GraphQL-Abfrage; Fehler der Abfrage kommen als `Api`/`Forbidden` (Datensatz nicht erlaubt).
    pub async fn graphql(&mut self, query: &str, variables: Value) -> Result<Value, CfError> {
        let req = self.http.post(format!("{}/graphql", self.base)).json(&json!({ "query": query, "variables": variables }));
        let v = self.send(req).await?;
        if let Some(errs) = v.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty()) {
            let msg = errs.iter().filter_map(|e| e.get("message").and_then(Value::as_str)).collect::<Vec<_>>().join("; ");
            let lower = msg.to_lowercase();
            if lower.contains("rate limit") || lower.contains("quota") {
                return Err(CfError::RateLimited(60));
            }
            if lower.contains("not authorized") || lower.contains("does not have access") || lower.contains("permission") {
                return Err(CfError::Forbidden(msg));
            }
            return Err(CfError::Api(msg));
        }
        Ok(v.get("data").cloned().unwrap_or(Value::Null))
    }

    /// Alle Zonen (seitenweise).
    pub async fn zones(&mut self) -> Result<Vec<Value>, CfError> {
        let mut out = Vec::new();
        for page in 1u32..=20 {
            let v = self.get(&format!("/zones?per_page=50&page={page}")).await?;
            let list = v.get("result").and_then(Value::as_array).cloned().unwrap_or_default();
            let n = list.len();
            out.extend(list);
            let pages = v.pointer("/result_info/total_pages").and_then(Value::as_u64).unwrap_or(1);
            if n == 0 || u64::from(page) >= pages {
                break;
            }
        }
        Ok(out)
    }
}

fn first_error(v: &Value) -> Option<String> {
    let e = v.pointer("/errors/0")?;
    let msg = e.get("message").and_then(Value::as_str)?;
    Some(match e.get("code").and_then(Value::as_i64) {
        Some(c) => format!("{msg} (Code {c})"),
        None => msg.to_string(),
    })
}

fn short(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "Zeitüberschreitung".into()
    } else if e.is_connect() {
        "Verbindung fehlgeschlagen".into()
    } else {
        e.to_string().chars().take(160).collect()
    }
}

// --- Abfragen ----------------------------------------------------------------------------------

const Q_SETTINGS: &str = "query($zone: string!) { viewer { zones(filter: {zoneTag: $zone}) { settings {
  httpRequests1mGroups { enabled notOlderThan }
  httpRequestsAdaptiveGroups { enabled notOlderThan }
  firewallEventsAdaptiveGroups { enabled notOlderThan }
  firewallEventsAdaptive { enabled notOlderThan }
} } } }";

const Q_1M: &str = "query($zone: string!, $from: Time!, $to: Time!) { viewer { zones(filter: {zoneTag: $zone}) {
  httpRequests1mGroups(limit: 100, filter: {datetime_geq: $from, datetime_lt: $to}, orderBy: [datetimeMinute_ASC]) {
    dimensions { datetimeMinute }
    sum { requests cachedRequests bytes cachedBytes threats pageViews
          countryMap { clientCountryName requests }
          responseStatusMap { edgeResponseStatus requests } }
    uniq { uniques }
  } } } }";

const Q_ADAPTIVE: &str = "query($zone: string!, $from: Time!, $to: Time!) { viewer { zones(filter: {zoneTag: $zone}) {
  cache: httpRequestsAdaptiveGroups(limit: 2000, filter: {datetime_geq: $from, datetime_lt: $to}) {
    count sum { edgeResponseBytes visits } dimensions { datetimeMinute cacheStatus } }
  status: httpRequestsAdaptiveGroups(limit: 2000, filter: {datetime_geq: $from, datetime_lt: $to}) {
    count dimensions { datetimeMinute edgeResponseStatus } }
  country: httpRequestsAdaptiveGroups(limit: 2000, filter: {datetime_geq: $from, datetime_lt: $to}) {
    count dimensions { datetimeMinute clientCountryName } }
  } } }";

const Q_FW_GROUPS: &str = "query($zone: string!, $from: Time!, $to: Time!) { viewer { zones(filter: {zoneTag: $zone}) {
  firewallEventsAdaptiveGroups(limit: 1000, filter: {datetime_geq: $from, datetime_lt: $to}) {
    count dimensions { action datetimeMinute } } } } }";

const Q_FW_RAW: &str = "query($zone: string!, $from: Time!, $to: Time!) { viewer { zones(filter: {zoneTag: $zone}) {
  firewallEventsAdaptive(limit: 10000, filter: {datetime_geq: $from, datetime_lt: $to}, orderBy: [datetime_ASC]) {
    action datetime } } } }";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Http {
    OneMinute,
    Adaptive,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fw {
    Groups,
    Raw,
    None,
}

/// Werte einer Minute einer Zone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Minute {
    pub requests: f64,
    pub cached: f64,
    pub bytes: f64,
    pub cached_bytes: f64,
    pub threats: Option<f64>,
    pub visitors: Option<f64>,
    pub pageviews: Option<f64>,
    pub status: [f64; 4],
    pub countries: HashMap<String, f64>,
    /// block, challenge, log, other
    pub firewall: [f64; 4],
}

fn num(v: &Value, path: &str) -> f64 {
    v.pointer(path).and_then(Value::as_f64).unwrap_or(0.0)
}

fn zone_node<'a>(data: &'a Value, key: &str) -> &'a [Value] {
    data.pointer(&format!("/viewer/zones/0/{key}")).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn status_slot(code: i64) -> Option<usize> {
    match code {
        200..=299 => Some(0),
        300..=399 => Some(1),
        400..=499 => Some(2),
        500..=599 => Some(3),
        _ => None,
    }
}

fn fw_slot(action: &str) -> usize {
    match action {
        "block" | "drop" => 0,
        a if a.contains("challenge") => 1,
        "log" => 2,
        _ => 3,
    }
}

fn parse_1m(data: &Value, out: &mut BTreeMap<i64, Minute>) {
    for g in zone_node(data, "httpRequests1mGroups") {
        let Some(ts) = g.pointer("/dimensions/datetimeMinute").and_then(Value::as_str).and_then(parse_time) else { continue };
        let m = out.entry(ts).or_default();
        m.requests += num(g, "/sum/requests");
        m.cached += num(g, "/sum/cachedRequests");
        m.bytes += num(g, "/sum/bytes");
        m.cached_bytes += num(g, "/sum/cachedBytes");
        m.threats = Some(m.threats.unwrap_or(0.0) + num(g, "/sum/threats"));
        m.pageviews = Some(m.pageviews.unwrap_or(0.0) + num(g, "/sum/pageViews"));
        m.visitors = Some(m.visitors.unwrap_or(0.0) + num(g, "/uniq/uniques"));
        for c in g.pointer("/sum/countryMap").and_then(Value::as_array).into_iter().flatten() {
            let name = c.get("clientCountryName").and_then(Value::as_str).unwrap_or("").to_string();
            *m.countries.entry(name).or_default() += num(c, "/requests");
        }
        for s in g.pointer("/sum/responseStatusMap").and_then(Value::as_array).into_iter().flatten() {
            if let Some(i) = s.get("edgeResponseStatus").and_then(Value::as_i64).and_then(status_slot) {
                m.status[i] += num(s, "/requests");
            }
        }
    }
}

fn parse_adaptive(data: &Value, out: &mut BTreeMap<i64, Minute>) {
    let minute = |g: &Value| g.pointer("/dimensions/datetimeMinute").and_then(Value::as_str).and_then(parse_time);
    for g in zone_node(data, "cache") {
        let Some(ts) = minute(g) else { continue };
        let m = out.entry(ts).or_default();
        let n = num(g, "/count");
        let bytes = num(g, "/sum/edgeResponseBytes");
        m.requests += n;
        m.bytes += bytes;
        m.visitors = Some(m.visitors.unwrap_or(0.0) + num(g, "/sum/visits"));
        let cs = g.pointer("/dimensions/cacheStatus").and_then(Value::as_str).unwrap_or("");
        if matches!(cs, "hit" | "stale" | "updating" | "revalidated") {
            m.cached += n;
            m.cached_bytes += bytes;
        }
    }
    for g in zone_node(data, "status") {
        let Some(ts) = minute(g) else { continue };
        if let Some(i) = g.pointer("/dimensions/edgeResponseStatus").and_then(Value::as_i64).and_then(status_slot) {
            out.entry(ts).or_default().status[i] += num(g, "/count");
        }
    }
    for g in zone_node(data, "country") {
        let Some(ts) = minute(g) else { continue };
        let name = g.pointer("/dimensions/clientCountryName").and_then(Value::as_str).unwrap_or("").to_string();
        *out.entry(ts).or_default().countries.entry(name).or_default() += num(g, "/count");
    }
}

fn parse_fw(data: &Value, kind: Fw, out: &mut BTreeMap<i64, Minute>) {
    match kind {
        Fw::Groups => {
            for g in zone_node(data, "firewallEventsAdaptiveGroups") {
                let Some(ts) = g.pointer("/dimensions/datetimeMinute").and_then(Value::as_str).and_then(parse_time) else { continue };
                let a = g.pointer("/dimensions/action").and_then(Value::as_str).unwrap_or("");
                out.entry(ts).or_default().firewall[fw_slot(a)] += num(g, "/count");
            }
        }
        Fw::Raw => {
            for e in zone_node(data, "firewallEventsAdaptive") {
                let Some(ts) = e.get("datetime").and_then(Value::as_str).and_then(parse_time) else { continue };
                let a = e.get("action").and_then(Value::as_str).unwrap_or("");
                out.entry(ts.div_euclid(60) * 60).or_default().firewall[fw_slot(a)] += 1.0;
            }
        }
        Fw::None => {}
    }
}

// --- Diagramme ---------------------------------------------------------------------------------

/// Diagramme einer Zone.
pub struct ZoneCharts {
    requests: ChartHandle,
    bandwidth: ChartHandle,
    status: ChartHandle,
    threats: ChartHandle,
    visitors: ChartHandle,
    firewall: ChartHandle,
    countries: ChartHandle,
    country_names: Vec<String>,
    zone: String,
}

impl ZoneCharts {
    pub fn new(core: &Core, zone: &str) -> Self {
        let c = |t: &Tpl| core.chart(ChartDef::tpl(t).instance(zone));
        Self {
            requests: c(&CF_REQUESTS),
            bandwidth: c(&CF_BANDWIDTH),
            status: c(&CF_STATUS),
            threats: c(&CF_THREATS),
            visitors: c(&CF_VISITORS),
            firewall: c(&CF_FIREWALL),
            countries: core.chart(ChartDef::tpl(&CF_COUNTRIES).instance(zone).dims([("other", "Andere")])),
            country_names: Vec::new(),
            zone: zone.to_string(),
        }
    }

    /// Werte einer Minute in den Rahmen legen (`fw`: Firewall-Werte vorhanden).
    pub fn put(&mut self, core: &Core, f: &mut Frame, m: &Minute, fw: bool) {
        let r = |v: f64| util::round(v, 0);
        f.put(&self.requests, &[r(m.cached), r((m.requests - m.cached).max(0.0))]);
        f.put(&self.bandwidth, &[r(m.cached_bytes), r((m.bytes - m.cached_bytes).max(0.0))]);
        f.put(&self.status, &m.status.map(r));
        f.put(&self.threats, &[util::nan(m.threats.map(r))]);
        f.put(&self.visitors, &[util::nan(m.visitors.map(r)), util::nan(m.pageviews.map(r))]);
        if fw {
            f.put(&self.firewall, &m.firewall.map(r));
        }
        // Länder: häufigste bekommen einen festen Platz (stabil, neue hinten), Rest „Andere“.
        let mut order: Vec<(&String, &f64)> = m.countries.iter().filter(|(k, _)| !k.is_empty()).collect();
        order.sort_by(|a, b| b.1.total_cmp(a.1).then(a.0.cmp(b.0)));
        let mut changed = false;
        for (name, _) in &order {
            if !self.country_names.contains(name) && self.country_names.len() < MAX_COUNTRIES {
                self.country_names.push((*name).clone());
                changed = true;
            }
        }
        if changed {
            let mut dims: Vec<(String, String)> = self.country_names.iter().map(|n| (n.clone(), n.clone())).collect();
            dims.push(("other".into(), "Andere".into()));
            self.countries = core.chart(ChartDef::tpl(&CF_COUNTRIES).instance(&self.zone).dims(dims));
        }
        let mut vals = vec![0.0; self.country_names.len() + 1];
        for (name, n) in m.countries.iter() {
            match self.country_names.iter().position(|c| c == name) {
                Some(i) => vals[i] += n,
                None => *vals.last_mut().unwrap_or(&mut 0.0) += n,
            }
        }
        f.put(&self.countries, &vals.into_iter().map(r).collect::<Vec<_>>());
    }
}

/// Summen der letzten 24 Stunden je Zone.
#[derive(Default)]
pub struct Rolling {
    minutes: VecDeque<(i64, Minute)>,
}

impl Rolling {
    pub fn push(&mut self, ts: i64, m: Minute) {
        self.minutes.push_back((ts, m));
        while self.minutes.front().is_some_and(|(t, _)| *t < ts - 86_400) {
            self.minutes.pop_front();
        }
    }

    pub fn fill(&self, z: &mut CfZone) {
        let mut countries: HashMap<&str, f64> = HashMap::new();
        z.requests_24h = 0.0;
        z.cached_24h = 0.0;
        z.bytes_24h = 0.0;
        z.threats_24h = 0.0;
        z.firewall_24h = 0.0;
        for (_, m) in &self.minutes {
            z.requests_24h += m.requests;
            z.cached_24h += m.cached;
            z.bytes_24h += m.bytes;
            z.threats_24h += m.threats.unwrap_or(0.0);
            z.firewall_24h += m.firewall.iter().sum::<f64>();
            for (c, n) in &m.countries {
                *countries.entry(c.as_str()).or_default() += n;
            }
        }
        let mut top: Vec<(String, f64)> = countries.into_iter().filter(|(c, _)| !c.is_empty()).map(|(c, n)| (c.to_string(), n)).collect();
        top.sort_by(|a, b| b.1.total_cmp(&a.1));
        top.truncate(8);
        z.top_countries = top;
    }
}

struct TunnelCharts {
    conns: ChartHandle,
    up: ChartHandle,
}

// --- Sammler -----------------------------------------------------------------------------------

struct ZoneState {
    info: CfZone,
    http: Http,
    fw: Fw,
    discovered_at: i64,
    last: i64,
    charts: ZoneCharts,
    rolling: Rolling,
}

pub struct Runner {
    core: Core,
    client: Client,
    eff: Effective,
    zones: Vec<ZoneState>,
    zones_at: i64,
    tunnels: HashMap<String, (CfTunnel, TunnelCharts)>,
    status: CfStatus,
    backoff_until: i64,
}

impl Runner {
    pub fn new(core: Core, base: &str, eff: Effective) -> Self {
        let client = Client::new(core.http.clone(), base, &eff.token);
        Self {
            core,
            client,
            eff,
            zones: Vec::new(),
            zones_at: 0,
            tunnels: HashMap::new(),
            status: CfStatus { configured: true, ..CfStatus::default() },
            backoff_until: 0,
        }
    }

    pub async fn run(mut self) {
        loop {
            self.cycle(util::now()).await;
            // Jede Minute einmal, kurz nach dem Minutenwechsel.
            let now = util::now_ms();
            let next = (now.div_euclid(60_000) + 1) * 60_000 + 7_000;
            tokio::time::sleep(std::time::Duration::from_millis((next - now) as u64)).await;
        }
    }

    pub async fn cycle(&mut self, now: i64) {
        if now < self.backoff_until {
            self.publish();
            return;
        }
        match self.poll(now).await {
            Ok(()) => {
                self.status.ok = true;
                self.status.error = None;
                self.status.last_poll = util::now_ms();
            }
            Err(CfError::RateLimited(secs)) => {
                self.backoff_until = now + secs;
                self.status.error = Some(CfError::RateLimited(secs).to_string());
            }
            Err(e) => {
                self.status.ok = false;
                self.status.error = Some(e.to_string());
            }
        }
        self.publish();
    }

    fn publish(&mut self) {
        self.status.calls = self.client.calls;
        self.status.backoff_until = self.backoff_until;
        self.status.zones = self
            .zones
            .iter()
            .map(|z| {
                let mut info = z.info.clone();
                z.rolling.fill(&mut info);
                info
            })
            .collect();
        let mut tunnels: Vec<CfTunnel> = self.tunnels.values().map(|(t, _)| t.clone()).collect();
        tunnels.sort_by(|a, b| a.name.cmp(&b.name));
        self.status.tunnels = tunnels;
        self.core.snapshot("cloudflare.status", &self.status);
    }

    async fn poll(&mut self, now: i64) -> Result<(), CfError> {
        if self.zones.is_empty() || now - self.zones_at >= 1800 {
            self.load_zones(now).await?;
        }
        let mut hints = Vec::new();
        for i in 0..self.zones.len() {
            match self.zone_minutes(i, now).await {
                Ok(()) => {}
                Err(CfError::RateLimited(s)) => return Err(CfError::RateLimited(s)),
                Err(e) => self.zones[i].info.error = Some(e.to_string()),
            }
            if self.zones[i].http == Http::None {
                hints.push(format!("{}: keine HTTP-Analytics lesbar – Recht „Zone → Analytics → Lesen“ nötig.", self.zones[i].info.name));
            }
        }
        if let Err(e) = self.poll_tunnels(now).await {
            match e {
                CfError::RateLimited(s) => return Err(CfError::RateLimited(s)),
                CfError::Forbidden(_) | CfError::Auth(_) => {
                    hints.push("Tunnel: Recht „Konto → Cloudflare Tunnel → Lesen“ fehlt.".into());
                }
                e => hints.push(format!("Tunnel: {e}")),
            }
        }
        self.status.hints = hints;
        Ok(())
    }

    async fn load_zones(&mut self, now: i64) -> Result<(), CfError> {
        let list = self.client.zones().await?;
        self.zones_at = now;
        let want = &self.eff.zones;
        let mut found = Vec::new();
        for z in list {
            let id = z.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let name = z.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            if id.is_empty() || (!want.is_empty() && !want.iter().any(|w| w == &id || w.eq_ignore_ascii_case(&name))) {
                continue;
            }
            if self.status.account_id.is_empty() || !self.eff.account_id.is_empty() {
                self.status.account_id = if self.eff.account_id.is_empty() {
                    z.pointer("/account/id").and_then(Value::as_str).unwrap_or("").to_string()
                } else {
                    self.eff.account_id.clone()
                };
            }
            found.push(CfZone {
                id,
                name,
                plan: z.pointer("/plan/name").and_then(Value::as_str).unwrap_or("").to_string(),
                status: z.get("status").and_then(Value::as_str).unwrap_or("").to_string(),
                ..CfZone::default()
            });
        }
        if found.is_empty() {
            return Err(CfError::Api(
                "Das Token sieht keine Zone – Rechte „Zone → Zone → Lesen“ und „Zone → Analytics → Lesen“ vergeben.".into(),
            ));
        }
        found.truncate(MAX_ZONES);
        let mut old: HashMap<String, ZoneState> = self.zones.drain(..).map(|z| (z.info.id.clone(), z)).collect();
        for info in found {
            let state = match old.remove(&info.id) {
                Some(mut s) => {
                    s.info.plan = info.plan;
                    s.info.status = info.status;
                    s
                }
                None => ZoneState {
                    charts: ZoneCharts::new(&self.core, &info.name),
                    info,
                    http: Http::None,
                    fw: Fw::None,
                    discovered_at: 0,
                    last: (now - LAG - START_BACK).div_euclid(60) * 60,
                    rolling: Rolling::default(),
                },
            };
            self.zones.push(state);
        }
        for gone in old.into_values() {
            for t in ALL {
                self.core.registry.retire(&t.chart_id(Some(&gone.info.name)));
            }
        }
        Ok(())
    }

    /// Datensätze erkennen (alle 6 h): was erlaubt der Tarif dieser Zone?
    async fn discover(&mut self, i: usize, now: i64) -> Result<(), CfError> {
        let id = self.zones[i].info.id.clone();
        let data = self.client.graphql(Q_SETTINGS, json!({ "zone": id })).await;
        let z = &mut self.zones[i];
        z.discovered_at = now;
        match data {
            Ok(d) => {
                let on = |k: &str| d.pointer(&format!("/viewer/zones/0/settings/{k}/enabled")).and_then(Value::as_bool).unwrap_or(false);
                z.http = if on("httpRequests1mGroups") {
                    Http::OneMinute
                } else if on("httpRequestsAdaptiveGroups") {
                    Http::Adaptive
                } else {
                    Http::None
                };
                z.fw = if on("firewallEventsAdaptiveGroups") {
                    Fw::Groups
                } else if on("firewallEventsAdaptive") {
                    Fw::Raw
                } else {
                    Fw::None
                };
            }
            Err(CfError::RateLimited(s)) => return Err(CfError::RateLimited(s)),
            // `settings` nicht lesbar: der Reihe nach probieren.
            Err(_) => {
                z.http = Http::OneMinute;
                z.fw = Fw::Groups;
            }
        }
        Ok(())
    }

    async fn zone_minutes(&mut self, i: usize, now: i64) -> Result<(), CfError> {
        if now - self.zones[i].discovered_at >= 6 * 3600 {
            self.discover(i, now).await?;
        }
        let until = (now - LAG).div_euclid(60) * 60;
        let from = self.zones[i].last + 60;
        if until < from {
            return Ok(());
        }
        let id = self.zones[i].info.id.clone();
        let vars = json!({ "zone": id, "from": format_time(from), "to": format_time(until + 60) });
        let mut minutes: BTreeMap<i64, Minute> = BTreeMap::new();
        let mut zone_error = None;
        // HTTP-Analytics mit Rückfall.
        loop {
            let q = match self.zones[i].http {
                Http::OneMinute => Q_1M,
                Http::Adaptive => Q_ADAPTIVE,
                Http::None => break,
            };
            match self.client.graphql(q, vars.clone()).await {
                Ok(d) => {
                    if self.zones[i].http == Http::OneMinute {
                        parse_1m(&d, &mut minutes);
                    } else {
                        parse_adaptive(&d, &mut minutes);
                    }
                    break;
                }
                Err(CfError::Forbidden(m) | CfError::Api(m)) => {
                    zone_error = Some(m);
                    self.zones[i].http = if self.zones[i].http == Http::OneMinute { Http::Adaptive } else { Http::None };
                }
                Err(e) => return Err(e),
            }
        }
        loop {
            let q = match self.zones[i].fw {
                Fw::Groups => Q_FW_GROUPS,
                Fw::Raw => Q_FW_RAW,
                Fw::None => break,
            };
            match self.client.graphql(q, vars.clone()).await {
                Ok(d) => {
                    parse_fw(&d, self.zones[i].fw, &mut minutes);
                    break;
                }
                Err(CfError::Forbidden(_) | CfError::Api(_)) => {
                    self.zones[i].fw = if self.zones[i].fw == Fw::Groups { Fw::Raw } else { Fw::None };
                }
                Err(e) => return Err(e),
            }
        }
        let z = &mut self.zones[i];
        z.info.dataset = match z.http {
            Http::OneMinute => "httpRequests1mGroups",
            Http::Adaptive => "httpRequestsAdaptiveGroups",
            Http::None => "",
        }
        .into();
        z.info.firewall_dataset = match z.fw {
            Fw::Groups => "firewallEventsAdaptiveGroups",
            Fw::Raw => "firewallEventsAdaptive",
            Fw::None => "",
        }
        .into();
        z.info.error = if z.http == Http::None {
            Some(format!("Keine HTTP-Analytics: {}", zone_error.unwrap_or_else(|| "Datensatz im Tarif nicht verfügbar".into())))
        } else {
            None
        };
        if z.http == Http::None && z.fw == Fw::None {
            z.last = until;
            return Ok(());
        }
        // Jede Minute genau einmal, fehlende Minuten = keine Anfragen (0).
        let oldest = (now - 14 * 60).div_euclid(60) * 60;
        let mut m = from;
        while m <= until {
            let v = minutes.remove(&m).unwrap_or_default();
            if m >= oldest {
                let mut f = Frame::new(m);
                if z.http != Http::None {
                    z.charts.put(&self.core, &mut f, &v, z.fw != Fw::None);
                } else {
                    let r = |x: f64| util::round(x, 0);
                    f.put(&z.charts.firewall, &v.firewall.map(r));
                }
                self.core.submit(f);
            }
            z.rolling.push(m, v);
            m += 60;
        }
        z.last = until;
        z.info.last_minute = until;
        Ok(())
    }

    async fn poll_tunnels(&mut self, now: i64) -> Result<(), CfError> {
        if self.status.account_id.is_empty() {
            return Ok(());
        }
        let v = self.client.get(&format!("/accounts/{}/cfd_tunnel?is_deleted=false&per_page=100", self.status.account_id)).await?;
        let list = v.get("result").and_then(Value::as_array).cloned().unwrap_or_default();
        let ts = now.div_euclid(60) * 60;
        let mut f = Frame::new(ts);
        let mut seen = Vec::new();
        for t in list {
            let id = t.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let name = t.get("name").and_then(Value::as_str).unwrap_or(&id).to_string();
            let status = t.get("status").and_then(Value::as_str).unwrap_or("inactive").to_string();
            let conns = t.get("connections").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut colos: Vec<String> =
                conns.iter().filter_map(|c| c.get("colo_name").and_then(Value::as_str).map(str::to_string)).collect();
            colos.sort();
            colos.dedup();
            let tunnel = CfTunnel {
                id: id.clone(),
                name: name.clone(),
                status: status.clone(),
                connections: conns.len() as u32,
                colos,
                status_since: now,
                created_at: t.get("created_at").and_then(Value::as_str).unwrap_or("").to_string(),
            };
            seen.push(id.clone());
            let core = self.core.clone();
            let entry = self.tunnels.entry(id.clone()).or_insert_with(|| {
                let charts = TunnelCharts {
                    conns: core.chart(ChartDef::tpl(&CF_TUNNEL_CONNS).instance(&name)),
                    up: core.chart(ChartDef::tpl(&CF_TUNNEL_UP).instance(&name)),
                };
                (CfTunnel { status: String::new(), ..tunnel.clone() }, charts)
            });
            let before = std::mem::take(&mut entry.0.status);
            let since = entry.0.status_since;
            entry.0 = CfTunnel { status_since: if before == status { since } else { now }, ..tunnel };
            if !before.is_empty() && before != status {
                let up = matches!(status.as_str(), "healthy");
                let sev = match status.as_str() {
                    "healthy" => Severity::Ok,
                    "degraded" => Severity::Warn,
                    _ => Severity::Crit,
                };
                self.core.event(
                    Event::new("cloudflare", if up { "tunnel.up" } else { "tunnel.down" }, sev, &name, format!("Tunnel {name}: {}", tunnel_label(&status)))
                        .message(format!("Vorher: {}", tunnel_label(&before)))
                        .label("tunnel", id.clone()),
                );
            }
            let up = matches!(status.as_str(), "healthy" | "degraded");
            f.put(&entry.1.conns, &[conns.len() as f64]);
            f.put(&entry.1.up, &[if up { 1.0 } else { 0.0 }]);
        }
        let gone: Vec<String> = self.tunnels.keys().filter(|k| !seen.contains(k)).cloned().collect();
        for id in gone {
            if let Some((t, _)) = self.tunnels.remove(&id) {
                self.core.registry.retire(&CF_TUNNEL_CONNS.chart_id(Some(&t.name)));
                self.core.registry.retire(&CF_TUNNEL_UP.chart_id(Some(&t.name)));
            }
        }
        self.core.submit(f);
        Ok(())
    }
}

pub fn tunnel_label(s: &str) -> &str {
    match s {
        "healthy" => "verbunden",
        "degraded" => "eingeschränkt",
        "down" => "getrennt",
        "inactive" => "inaktiv",
        other => other,
    }
}

type Tpl = shared::catalog::Tpl;

/// „Verbindung testen“.
pub async fn test(core: &Core, s: &CloudflareSettings) -> TestResult {
    test_at(core, API, s).await
}

pub async fn test_at(core: &Core, base: &str, s: &CloudflareSettings) -> TestResult {
    let eff = effective_from(s);
    if eff.token.is_empty() {
        return TestResult { ok: false, message: "Kein Token hinterlegt.".into(), details: vec![] };
    }
    let mut c = Client::new(core.http.clone(), base, &eff.token);
    let mut details = Vec::new();
    match c.get("/user/tokens/verify").await {
        Ok(v) => details.push(format!("Token gültig ({})", v.pointer("/result/status").and_then(Value::as_str).unwrap_or("active"))),
        // Konto-Tokens lassen sich so nicht prüfen – die Zonenliste entscheidet.
        Err(CfError::Auth(_) | CfError::Forbidden(_)) => {}
        Err(e) => return TestResult { ok: false, message: e.to_string(), details },
    }
    let zones = match c.zones().await {
        Ok(z) => z,
        Err(e) => return TestResult { ok: false, message: e.to_string(), details },
    };
    if zones.is_empty() {
        return TestResult { ok: false, message: "Das Token sieht keine Zone (Recht „Zone → Zone → Lesen“).".into(), details };
    }
    let names: Vec<String> = zones
        .iter()
        .map(|z| {
            format!(
                "{} ({})",
                z.get("name").and_then(Value::as_str).unwrap_or("?"),
                z.pointer("/plan/name").and_then(Value::as_str).unwrap_or("?")
            )
        })
        .collect();
    details.push(format!("Zonen: {}", names.join(", ")));
    let first = zones[0].get("id").and_then(Value::as_str).unwrap_or("").to_string();
    match c.graphql(Q_SETTINGS, json!({ "zone": first })).await {
        Ok(d) => {
            let on = |k: &str| d.pointer(&format!("/viewer/zones/0/settings/{k}/enabled")).and_then(Value::as_bool).unwrap_or(false);
            let ds: Vec<&str> = ["httpRequests1mGroups", "httpRequestsAdaptiveGroups", "firewallEventsAdaptiveGroups", "firewallEventsAdaptive"]
                .into_iter()
                .filter(|k| on(k))
                .collect();
            details.push(format!("Analytics: {}", if ds.is_empty() { "keine Datensätze erlaubt".into() } else { ds.join(", ") }));
        }
        Err(e) => details.push(format!("Analytics: {e}")),
    }
    let account = if eff.account_id.is_empty() {
        zones[0].pointer("/account/id").and_then(Value::as_str).unwrap_or("").to_string()
    } else {
        eff.account_id.clone()
    };
    if !account.is_empty() {
        match c.get(&format!("/accounts/{account}/cfd_tunnel?is_deleted=false&per_page=100")).await {
            Ok(v) => details.push(format!("Tunnel: {}", v.get("result").and_then(Value::as_array).map_or(0, Vec::len))),
            Err(e) => details.push(format!("Tunnel nicht lesbar: {e}")),
        }
    }
    TestResult { ok: true, message: "Verbindung in Ordnung.".into(), details }
}
