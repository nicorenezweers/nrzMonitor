//! DNS über nrzNetwork (MCP): jede Anfrage im Netzwerk mitschreiben.
//!
//! Ablauf alle 10 s:
//! 1. `get_query_log` für das Fenster `[synced_until + 1 − Überlappung, jetzt]` abholen (alle Seiten,
//!    höchstens 500 je Seite). Nach einem Ausfall wird in Zeitscheiben aufgeholt (älteste zuerst,
//!    Scheibe halbiert sich, wenn sie zu voll ist). Doppelte erkennt die Nummer aus nrzNetwork.
//! 2. Neue Anfragen + Merker in einer Transaktion speichern ([`store`]).
//! 3. Vollständige 10-s-Abschnitte als Diagrammwerte abgeben ([`agg`]).
//! 4. Jede Minute `get_server_status` (Cache, Weiterleitungen), alle 5 min `get_blocking_status`.
//! 5. Alle 10 min abgeschlossene Tage als Parquet ablegen, Aufbewahrung durchsetzen.
//!
//! Ohne Zugangsdaten und mit `core.cfg.demo` laufen Beispieldaten durch dieselbe Strecke.

pub mod agg;
mod demo;
pub mod mcp;
pub mod store;
#[cfg(test)]
mod tests;

use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::RwLock;
use serde::Deserialize;
use serde_json::json;
use shared::{
    catalog::dns::*,
    dns::{BlocklistInfo, DEFAULT_MCP_URL, DnsSettings, DnsStatus, QueryEntry, QueryFilter, TestResult, UpstreamInfo},
    events::{Event, Severity},
};
use tokio::task::JoinHandle;

use self::{mcp::Mcp, store::DnsStore};
use crate::{
    config,
    core::Core,
    frame::Frame,
    registry::{ChartDef, ChartHandle},
    util::{self, Counter},
};

/// Überlappung beim Abholen (Sekunden): nrzNetwork schreibt sein Protokoll gebündelt.
pub const OVERLAP: i64 = 20;
const PER_PAGE: u32 = 500;
/// Mehr Seiten je Zeitscheibe → Scheibe halbieren.
const MAX_PAGES: u64 = 20;
/// Höchstens so viele Aufrufe je Durchgang (der Rest folgt im nächsten).
const MAX_CALLS: u32 = 80;
/// Beim allerersten Start so weit zurück holen (Sekunden).
const BACKFILL: i64 = 86_400;

static CURRENT: RwLock<Option<Arc<DnsStore>>> = parking_lot::const_rwlock(None);
static FILE_STORE: std::sync::OnceLock<Arc<DnsStore>> = std::sync::OnceLock::new();

/// Speicher, aus dem die API liest (Beispieldaten oder echte).
pub fn current_store() -> Option<Arc<DnsStore>> {
    CURRENT.read().clone()
}

fn file_store(core: &Core) -> anyhow::Result<Arc<DnsStore>> {
    if let Some(s) = FILE_STORE.get() {
        return Ok(s.clone());
    }
    let s = Arc::new(DnsStore::open(core.db.conn()?, core.cfg.data_dir.join("dns"))?);
    Ok(FILE_STORE.get_or_init(|| s).clone())
}

/// Wirksame Einstellungen: gespeichert, sonst aus der Umgebung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    pub enabled: bool,
    pub url: String,
    pub token: String,
    pub token_from_env: bool,
    pub retention_days: u32,
}

impl Effective {
    pub fn configured(&self) -> bool {
        self.enabled && !self.token.is_empty() && !self.url.is_empty()
    }
}

pub fn effective_from(s: &DnsSettings) -> Effective {
    let env_token = config::var("NRZMON_DNS_MCP_TOKEN");
    let token_from_env = s.token.trim().is_empty() && env_token.is_some();
    let url = Some(s.url.trim().to_string())
        .filter(|u| !u.is_empty())
        .or_else(|| config::var("NRZMON_DNS_MCP_URL"))
        .unwrap_or_else(|| DEFAULT_MCP_URL.to_string());
    Effective {
        enabled: s.enabled,
        url,
        token: if token_from_env { env_token.unwrap_or_default() } else { s.token.trim().to_string() },
        token_from_env,
        retention_days: if s.retention_days == 0 { shared::dns::DEFAULT_RETENTION_DAYS } else { s.retention_days.min(3650) },
    }
}

pub fn effective(core: &Core) -> Effective {
    effective_from(&core.settings.get::<DnsSettings>("dns"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Real(Effective),
    Demo,
    Off(u32),
}

pub fn start(core: &Core) {
    let core = core.clone();
    tokio::spawn(async move {
        let mut rx = core.settings.subscribe();
        let mut running: Option<(Mode, JoinHandle<()>)> = None;
        loop {
            let eff = effective(&core);
            let mode = if eff.configured() {
                Mode::Real(eff.clone())
            } else if core.cfg.demo {
                Mode::Demo
            } else {
                Mode::Off(eff.retention_days)
            };
            if running.as_ref().map(|(m, _)| m) != Some(&mode) {
                if let Some((_, h)) = running.take() {
                    h.abort();
                }
                match launch(&core, &mode) {
                    Ok(h) => running = Some((mode, h)),
                    Err(e) => {
                        tracing::warn!("DNS: {e:#}");
                        running = Some((mode, tokio::spawn(async {})));
                    }
                }
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    });
}

fn launch(core: &Core, mode: &Mode) -> anyhow::Result<JoinHandle<()>> {
    let (store, src, eff) = match mode {
        Mode::Real(eff) => (file_store(core)?, Source::Mcp(Mcp::new(core.http.clone(), &eff.url, &eff.token)), eff.clone()),
        Mode::Demo => (Arc::new(DnsStore::memory()?), Source::Demo(demo::Gen::new()), effective(core)),
        Mode::Off(_) => {
            // Nichts eingerichtet: alte Daten bleiben lesbar, Zustand sagt „nicht eingerichtet“.
            let store = file_store(core)?;
            *CURRENT.write() = Some(store);
            let eff = effective(core);
            core.snapshot("dns.status", &DnsStatus { url: eff.url, retention_days: eff.retention_days, ..DnsStatus::default() });
            return Ok(tokio::spawn(async {}));
        }
    };
    *CURRENT.write() = Some(store.clone());
    let core = core.clone();
    Ok(tokio::spawn(async move {
        match Runner::new(&core, store, src, eff).await {
            Ok(mut r) => r.run().await,
            Err(e) => tracing::warn!("DNS-Sammler nicht gestartet: {e:#}"),
        }
    }))
}

enum Source {
    Mcp(Mcp),
    Demo(demo::Gen),
}

/// Eine Seite von `get_query_log` (Felder wie `QueryLogEntry` in nrzNetwork).
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct LogPage {
    entries: Vec<LogEntry>,
    total: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct LogEntry {
    id: i64,
    at_ms: i64,
    client: String,
    protocol: String,
    qname: String,
    qtype: String,
    rcode: String,
    kind: String,
    answer: String,
    duration_ms: u32,
    upstream: String,
}

impl From<LogEntry> for QueryEntry {
    fn from(e: LogEntry) -> Self {
        Self {
            ts: e.at_ms,
            id: e.id,
            client: e.client,
            proto: e.protocol,
            name: e.qname,
            qtype: e.qtype,
            rcode: e.rcode,
            kind: e.kind,
            answer: e.answer,
            ms: e.duration_ms,
            upstream: e.upstream,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ServerStatus {
    version: String,
    uptime_secs: u64,
    dns_listen: Vec<String>,
    listen_errors: Vec<String>,
    queries_per_second: f64,
    cache_entries: u64,
    zones: u32,
    blocked_domains: u64,
    blocking_enabled: bool,
    blocking_paused_until: Option<i64>,
    upstreams: Vec<UpstreamHealth>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct UpstreamHealth {
    upstream: String,
    name: String,
    ok: u64,
    failed: u64,
    avg_ms: f64,
    last_error: Option<String>,
    last_error_at: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct BlockingStatus {
    enabled: bool,
    paused_until: Option<i64>,
    total_domains: u64,
    lists: Vec<BlockList>,
    custom_blocked: u32,
    allowed: u32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct BlockList {
    name: String,
    url: String,
    enabled: bool,
    domain_count: u64,
    last_updated_at: Option<i64>,
    last_error: Option<String>,
}

/// Fehler beim Abholen eines Fensters.
enum Fetch {
    TooMany,
    Err(mcp::McpError),
}

struct Upstream {
    latency: ChartHandle,
    answers: ChartHandle,
    ok: Counter,
    failed: Counter,
}

struct Runner {
    core: Core,
    store: Arc<DnsStore>,
    src: Source,
    eff: Effective,
    agg: agg::Agg,
    /// Nummer → Zeit (ms) der zuletzt gesehenen Anfragen (Fenster der Überlappung).
    seen: HashMap<i64, i64>,
    synced: i64,
    status: DnsStatus,
    cache: ChartHandle,
    blocklist: ChartHandle,
    upstreams: HashMap<String, Upstream>,
    last_server: i64,
    last_blocking: i64,
    last_maint: i64,
    charts_saved: i64,
    failures: u32,
}

impl Runner {
    async fn new(core: &Core, store: Arc<DnsStore>, src: Source, eff: Effective) -> anyhow::Result<Self> {
        let st = store.clone();
        let (synced, charts_until, seen, qtypes, clients, backlog) = blocking(move || {
            let synced = st.state("synced_until")?.unwrap_or(0);
            let charts_until = st.state("charts_until")?.unwrap_or(0);
            let seen: HashMap<i64, i64> = st.ids_since((synced - OVERLAP - 5) * 1000)?.into_iter().collect();
            let now = util::now();
            let f = QueryFilter { from: Some(now - 86_400), ..QueryFilter::default() };
            let top = st.top(&f, 11)?;
            // Was nach dem letzten abgegebenen Abschnitt schon gespeichert ist, kommt wieder in die Abschnitte.
            let since = charts_until.max(now - 15 * 60);
            let backlog = st.page(&QueryFilter { from: Some(since), per_page: 500, ..QueryFilter::default() })?;
            let mut rows = backlog.entries;
            if backlog.total > 500 {
                rows.clear();
                let mut page = 1;
                loop {
                    let p = st.page(&QueryFilter { from: Some(since), per_page: 500, page, ..QueryFilter::default() })?;
                    let done = p.entries.len() < 500;
                    rows.extend(p.entries);
                    if done {
                        break;
                    }
                    page += 1;
                }
            }
            Ok((
                synced,
                charts_until,
                seen,
                top.qtypes.into_iter().map(|t| t.name).collect::<Vec<_>>(),
                top.clients.into_iter().map(|t| t.name).collect::<Vec<_>>(),
                rows,
            ))
        })
        .await?;
        let mut agg = agg::Agg::new(core, charts_until, qtypes, clients);
        agg.add(&backlog);
        let status = DnsStatus {
            configured: matches!(src, Source::Mcp(_)),
            demo: matches!(src, Source::Demo(_)),
            url: if matches!(src, Source::Mcp(_)) { eff.url.clone() } else { String::new() },
            retention_days: eff.retention_days,
            synced_until: synced,
            ..DnsStatus::default()
        };
        Ok(Self {
            core: core.clone(),
            store,
            src,
            eff,
            agg,
            seen,
            synced,
            status,
            cache: core.chart(ChartDef::tpl(&DNS_CACHE)),
            blocklist: core.chart(ChartDef::tpl(&DNS_BLOCKLIST)),
            upstreams: HashMap::new(),
            last_server: 0,
            last_blocking: 0,
            last_maint: 0,
            charts_saved: charts_until,
            failures: 0,
        })
    }

    async fn run(&mut self) {
        loop {
            self.cycle().await;
            // Auf den nächsten 10-s-Takt (+2 s, damit nrzNetwork seinen Puffer geschrieben hat);
            // nach Fehlern länger warten (bis +50 s).
            let now = util::now_ms();
            let step = i64::from(STEP) * 1000;
            let mut next = (now.div_euclid(step) + 1) * step + 2000;
            if next - now < 1000 {
                next += step;
            }
            next += step * i64::from(self.failures.min(5));
            tokio::time::sleep(Duration::from_millis((next - now).max(500) as u64)).await;
        }
    }

    async fn cycle(&mut self) {
        let now = util::now();
        match self.sync_log(now).await {
            Ok(()) => {
                if self.failures >= 3 {
                    self.core.event(
                        Event::new("dns", "dns.online", Severity::Ok, "nrzNetwork", "nrzNetwork wieder erreichbar")
                            .message("Das Abfrageprotokoll wird lückenlos nachgeholt."),
                    );
                }
                self.failures = 0;
                self.status.connected = true;
                self.status.error = None;
                self.status.last_poll = util::now_ms();
            }
            Err(e) => {
                self.failures += 1;
                self.status.connected = false;
                self.status.error = Some(e.clone());
                if self.failures == 3 {
                    self.core.event(Event::new("dns", "dns.offline", Severity::Warn, "nrzNetwork", "nrzNetwork nicht erreichbar").message(e));
                }
            }
        }
        for f in self.agg.flush(self.synced - OVERLAP, util::now()) {
            self.core.submit(f);
        }
        if self.agg.emitted_until != self.charts_saved {
            self.charts_saved = self.agg.emitted_until;
            let (st, v) = (self.store.clone(), self.charts_saved);
            let _ = blocking(move || st.set_state("charts_until", v)).await;
        }
        if self.status.connected && now - self.last_server >= 60 {
            self.last_server = now;
            if let Err(e) = self.server_status(now).await {
                self.status.error = Some(e);
            }
        }
        if self.status.connected && now - self.last_blocking >= 300 {
            self.last_blocking = now;
            if let Err(e) = self.blocking_status(now).await {
                self.status.error = Some(e);
            }
        }
        if now - self.last_maint >= 600 {
            self.last_maint = now;
            self.maintain(now).await;
        }
        self.status.synced_until = self.synced;
        self.status.retention_days = self.eff.retention_days;
        self.core.snapshot("dns.status", &self.status);
    }

    /// Protokoll bis jetzt abholen und speichern.
    async fn sync_log(&mut self, now: i64) -> Result<(), String> {
        let floor = now - i64::from(self.eff.retention_days) * 86_400;
        let mut slice: i64 = 3600;
        let mut calls = 0u32;
        self.status.catching_up = false;
        loop {
            let start = if self.synced == 0 { now - BACKFILL.min(i64::from(self.eff.retention_days) * 86_400) } else { self.synced + 1 - OVERLAP };
            let from = start.max(floor);
            let to = (from + slice).min(now);
            if to < from {
                return Ok(());
            }
            let entries = match &mut self.src {
                Source::Demo(g) => {
                    let first = self.synced == 0;
                    g.generate(if first { from } else { self.synced + 1 }, to)
                }
                Source::Mcp(m) => match fetch(m, from, to, &mut calls, &mut self.status).await {
                    Ok(e) => e,
                    Err(Fetch::TooMany) if to > from => {
                        slice = ((to - from) / 2).max(1);
                        continue;
                    }
                    Err(Fetch::TooMany) => return Err("zu viele Anfragen in einer Sekunde".into()),
                    Err(Fetch::Err(e)) => return Err(e.to_string()),
                },
            };
            let fresh = self.dedupe(entries);
            let st = self.store.clone();
            let rows = fresh.clone();
            blocking(move || st.insert(&rows, Some(to))).await.map_err(|e| format!("Speichern: {e:#}"))?;
            self.agg.add(&fresh);
            self.synced = to;
            let keep = (self.synced - OVERLAP - 5) * 1000;
            self.seen.retain(|_, ts| *ts >= keep);
            if to >= now {
                return Ok(());
            }
            slice = (slice * 2).min(6 * 3600);
            if calls >= MAX_CALLS {
                self.status.catching_up = true;
                return Ok(());
            }
        }
    }

    fn dedupe(&mut self, entries: Vec<QueryEntry>) -> Vec<QueryEntry> {
        let mut out = Vec::with_capacity(entries.len());
        for e in entries {
            if e.id != 0 && self.seen.insert(e.id, e.ts).is_some() {
                continue;
            }
            out.push(e);
        }
        out.sort_by_key(|e| (e.ts, e.id));
        out
    }

    async fn server_status(&mut self, now: i64) -> Result<(), String> {
        let s: ServerStatus = match &mut self.src {
            Source::Mcp(m) => {
                let (s, size) = m.call_as::<ServerStatus>("get_server_status", json!({})).await.map_err(|e| e.to_string())?;
                self.status.calls += 1;
                self.status.rx_bytes += size as u64;
                s
            }
            Source::Demo(g) => g.server_status(),
        };
        let st = &mut self.status;
        st.version = s.version;
        st.uptime_secs = s.uptime_secs;
        st.queries_per_second = s.queries_per_second;
        st.cache_entries = s.cache_entries;
        st.zones = s.zones;
        st.blocked_domains = s.blocked_domains;
        st.blocking_enabled = s.blocking_enabled;
        st.paused_until = s.blocking_paused_until;
        st.listen = s.dns_listen;
        st.listen_errors = s.listen_errors;
        st.upstreams = s
            .upstreams
            .iter()
            .map(|u| UpstreamInfo {
                upstream: u.upstream.clone(),
                name: u.name.clone(),
                ok: u.ok,
                failed: u.failed,
                avg_ms: u.avg_ms,
                last_error: u.last_error.clone(),
                last_error_at: u.last_error_at,
            })
            .collect();
        let ts = now.div_euclid(60) * 60;
        let mut f = Frame::new(ts);
        f.put(&self.cache, &[s.cache_entries as f64]);
        for u in &s.upstreams {
            let label = if u.upstream.is_empty() { u.name.clone() } else { u.upstream.clone() };
            let up = self.upstreams.entry(label.clone()).or_insert_with(|| {
                let title = if u.name.is_empty() { label.clone() } else { u.name.clone() };
                Upstream {
                    latency: self.core.chart(ChartDef::tpl(&DNS_UPSTREAM_LATENCY).instance(&label).family(&title)),
                    answers: self.core.chart(ChartDef::tpl(&DNS_UPSTREAM_ANSWERS).instance(&label).family(&title)),
                    ok: Counter::new(),
                    failed: Counter::new(),
                }
            });
            f.put(&up.latency, &[util::round(u.avg_ms, 2)]);
            let ok = up.ok.rate(u.ok, 60.0).map(|v| util::round(v, 2));
            let failed = up.failed.rate(u.failed, 60.0).map(|v| util::round(v, 2));
            f.put(&up.answers, &[util::nan(ok), util::nan(failed)]);
        }
        self.core.submit(f);
        Ok(())
    }

    async fn blocking_status(&mut self, now: i64) -> Result<(), String> {
        let b: BlockingStatus = match &mut self.src {
            Source::Mcp(m) => {
                let (b, size) = m.call_as::<BlockingStatus>("get_blocking_status", json!({})).await.map_err(|e| e.to_string())?;
                self.status.calls += 1;
                self.status.rx_bytes += size as u64;
                b
            }
            Source::Demo(g) => g.blocking_status(),
        };
        let st = &mut self.status;
        st.blocking_enabled = b.enabled;
        st.paused_until = b.paused_until;
        st.blocked_domains = b.total_domains;
        st.custom_blocked = b.custom_blocked;
        st.allowed = b.allowed;
        st.blocklists = b
            .lists
            .into_iter()
            .map(|l| BlocklistInfo {
                name: l.name,
                url: l.url,
                enabled: l.enabled,
                domains: l.domain_count,
                updated_at: l.last_updated_at,
                error: l.last_error,
            })
            .collect();
        let mut f = Frame::new(now.div_euclid(300) * 300);
        f.put(&self.blocklist, &[b.total_domains as f64]);
        self.core.submit(f);
        Ok(())
    }

    /// Abgeschlossene Tage archivieren, Aufbewahrung, Belegung messen.
    async fn maintain(&mut self, now: i64) {
        let st = self.store.clone();
        let cutoff = (self.synced - OVERLAP - 60) * 1000;
        let days = self.eff.retention_days;
        match blocking(move || {
            st.archive(cutoff)?;
            st.retain(days, util::now_ms())?;
            st.stats()
        })
        .await
        {
            Ok(s) => {
                self.status.stored = s.rows;
                self.status.stored_bytes = s.bytes;
                self.status.bytes_per_query = s.bytes_per_query;
                self.status.oldest = s.oldest;
            }
            Err(e) => tracing::warn!("DNS-Speicher pflegen: {e:#}"),
        }
        let _ = now;
    }
}

/// Ein Zeitfenster `[from, to]` vollständig abholen (alle Seiten).
async fn fetch(m: &Mcp, from: i64, to: i64, calls: &mut u32, status: &mut DnsStatus) -> Result<Vec<QueryEntry>, Fetch> {
    let mut out = Vec::new();
    let mut page = 1u32;
    loop {
        let args = json!({ "from": from, "to": to, "limit": PER_PAGE, "page": page });
        let (p, size) = m.call_as::<LogPage>("get_query_log", args).await.map_err(Fetch::Err)?;
        *calls += 1;
        status.calls += 1;
        status.rx_bytes += size as u64;
        let pages = p.total.div_ceil(u64::from(PER_PAGE));
        if page == 1 && pages > MAX_PAGES {
            return Err(Fetch::TooMany);
        }
        let n = p.entries.len();
        out.extend(p.entries.into_iter().map(QueryEntry::from));
        if n == 0 || u64::from(page) >= pages {
            return Ok(out);
        }
        page += 1;
    }
}

/// Blockierende Arbeit (DuckDB) auslagern.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| anyhow::anyhow!("Aufgabe abgebrochen: {e}"))?
}

/// „Verbindung testen“: MCP aufbauen, Serverzustand und eine Protokollseite lesen.
pub async fn test(core: &Core, s: &DnsSettings) -> TestResult {
    let eff = effective_from(s);
    if eff.token.is_empty() {
        return TestResult { ok: false, message: "Kein Token hinterlegt.".into(), details: vec![] };
    }
    let m = Mcp::new(core.http.clone(), &eff.url, &eff.token);
    let server = match m.initialize().await {
        Ok(s) => s,
        Err(e) => return TestResult { ok: false, message: e.to_string(), details: vec![] },
    };
    let mut details = vec![format!("Verbunden mit {server}")];
    match m.call_as::<ServerStatus>("get_server_status", json!({})).await {
        Ok((s, _)) => details.push(format!(
            "Version {}, {:.1} Anfragen/s, {} Cache-Einträge, {} Weiterleitungen",
            s.version,
            s.queries_per_second,
            s.cache_entries,
            s.upstreams.len()
        )),
        Err(e) => return TestResult { ok: false, message: e.to_string(), details },
    }
    let now = util::now();
    match m.call_as::<LogPage>("get_query_log", json!({ "from": now - 3600, "to": now, "limit": 1 })).await {
        Ok((p, size)) => details.push(format!("Abfrageprotokoll: {} Anfragen in der letzten Stunde ({size} Bytes Antwort)", p.total)),
        Err(e) => return TestResult { ok: false, message: format!("Abfrageprotokoll: {e}"), details },
    }
    TestResult { ok: true, message: "Verbindung in Ordnung.".into(), details }
}
