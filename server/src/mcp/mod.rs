//! MCP-Server (Model Context Protocol, „Streamable HTTP“) unter `/mcp`.
//!
//! Damit fragen KI-Programme, Skripte und externe Dashboards alles ab, was nrzMonitor weiss:
//! Diagramme, Live-Werte, Zeitreihen aus DuckDB, Ereignisse, Container, Alarme, DNS, Strom …
//!
//! - `POST /mcp`   JSON-RPC (einzeln oder gebündelt); `initialize` vergibt eine Sitzung (`Mcp-Session-Id`).
//! - `GET /mcp`    Server-Sent Events der Sitzung: `notifications/resources/updated` für abonnierte
//!   Ressourcen, `notifications/message` für Ereignisse (nach `logging/setLevel`), Katalogänderungen.
//! - `DELETE /mcp` Sitzung beenden.
//! - `GET /mcp/stream` einfacher Live-Strom für Dashboards (siehe [`stream`]).
//! - `/mcp/api/v1/…` die ganze API mit dem MCP-Schlüssel (lesend; ändernd nur mit `allow_write`).
//! - `nrzmonitor mcp` Brücke stdin/stdout → HTTP, z. B. `docker exec -i nrzmonitor nrzmonitor mcp`.
//!
//! Zugang: `Authorization: Bearer <Schlüssel>` (auch `X-Api-Key` oder `?token=`). Der Schlüssel steht
//! in den Einstellungen (oder `NRZMON_MCP_TOKEN`). Mit `NRZMON_PASSWORD` gilt auch die Anmeldung der Oberfläche.

mod resources;
mod stdio;
mod stream;
mod tools;

use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{any, get},
};
use futures_util::stream::Stream;
use parking_lot::Mutex;
use serde_json::{Value, json};
use shared::{events::Severity, mcp::McpSettings};
use tokio::sync::{broadcast::error::RecvError, mpsc};

pub use self::{stdio::run as stdio, tools::catalog as tool_catalog};
use crate::{auth, config, core::Core, util};

/// Bevorzugte Protokollversion und alle, die der Server versteht.
pub const PROTOCOL: &str = "2025-06-18";
const SUPPORTED: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
/// Sitzungen ohne offenen Strom fallen nach so vielen Sekunden ohne Anfrage weg.
const SESSION_IDLE: i64 = 3600;

const SESSION_HEADER: &str = "mcp-session-id";

pub struct Hub {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// Die eigene API ohne Anmeldung – für Werkzeuge und `/mcp/api/v1/…`.
    api: Router,
    /// Offene Ströme (`GET /mcp` und `/mcp/stream`).
    pub streams: AtomicUsize,
}

pub struct Session {
    pub id: String,
    pub protocol: String,
    pub client: String,
    last_seen: AtomicI64,
    /// Abonnierte Ressourcen (URIs).
    subs: Mutex<HashSet<String>>,
    /// Ereignisse ab dieser Stufe als `notifications/message` (None = keine).
    log_level: Mutex<Option<u8>>,
    tx: Mutex<Option<mpsc::Sender<Value>>>,
}

impl Session {
    fn touch(&self) {
        self.last_seen.store(util::now(), Ordering::Relaxed);
    }

    /// Nachricht an den offenen Strom (ohne zu warten; voll = verwerfen).
    fn push(&self, msg: &Value) {
        let mut tx = self.tx.lock();
        if let Some(t) = tx.as_ref()
            && let Err(mpsc::error::TrySendError::Closed(_)) = t.try_send(msg.clone())
        {
            *tx = None;
        }
    }

    fn streaming(&self) -> bool {
        self.tx.lock().as_ref().is_some_and(|t| !t.is_closed())
    }
}

static HUB: OnceLock<Arc<Hub>> = OnceLock::new();

pub fn hub() -> Option<&'static Arc<Hub>> {
    HUB.get()
}

impl Hub {
    pub fn sessions(&self) -> usize {
        self.sessions.lock().len()
    }

    fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().get(id).cloned()
    }

    fn all(&self) -> Vec<Arc<Session>> {
        self.sessions.lock().values().cloned().collect()
    }
}

/// Einstellungen inkl. Umgebung; fehlt der Schlüssel, wird er jetzt erzeugt und gespeichert.
pub fn settings(core: &Core) -> McpSettings {
    let mut s: McpSettings = core.settings.get("mcp");
    if let Some(t) = config::var("NRZMON_MCP_TOKEN") {
        s.token = t;
    } else if s.token.is_empty() {
        s.token = util::random_hex(24);
        if let Err(e) = core.settings.set("mcp", &s) {
            tracing::warn!("MCP-Schlüssel nicht gespeichert: {e:#}");
        }
    }
    if config::flag("NRZMON_MCP_WRITE") {
        s.allow_write = true;
    }
    s
}

/// Routen unter `/mcp` (eigene Anmeldung, CORS für Dashboards im Browser).
pub fn routes(core: &Core, api: Router) -> Router<Core> {
    let _ = HUB.set(Arc::new(Hub { sessions: Mutex::new(HashMap::new()), api, streams: AtomicUsize::new(0) }));
    // Schlüssel gleich beim Start anlegen, damit er in der Oberfläche steht.
    let _ = settings(core);
    Router::new()
        .route("/mcp", get(open_stream).post(post).delete(close).options(preflight))
        .route("/mcp/stream", get(stream::sse).options(preflight))
        .route("/mcp/api/v1/{*path}", any(stream::proxy))
        .route_layer(middleware::from_fn_with_state(core.clone(), guard))
}

// --- Zugang -----------------------------------------------------------------------------------------

fn query_token(req: &Request) -> Option<String> {
    req.uri().query()?.split('&').find_map(|kv| kv.strip_prefix("token=")).map(|t| t.replace("%2B", "+"))
}

fn presented_token(headers: &HeaderMap) -> Option<String> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")).map(|t| t.trim().to_string()));
    bearer.or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()).map(|t| t.trim().to_string()))
}

fn authorized(core: &Core, s: &McpSettings, req: &Request) -> bool {
    let token = presented_token(req.headers()).or_else(|| query_token(req));
    if token.is_some_and(|t| !s.token.is_empty() && util::constant_eq(t.as_bytes(), s.token.as_bytes())) {
        return true;
    }
    // Ohne Passwort ist die Oberfläche offen – das soll den Schlüssel nicht ersetzen.
    core.cfg.password.is_some() && auth::is_authenticated(core, req.headers())
}

fn cors(res: &mut Response) {
    let h = res.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    h.insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue::from_static("Mcp-Session-Id, Mcp-Protocol-Version, WWW-Authenticate"));
}

async fn preflight() -> Response {
    let mut res = StatusCode::NO_CONTENT.into_response();
    cors(&mut res);
    let h = res.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"));
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Authorization, Content-Type, Accept, X-Api-Key, Mcp-Session-Id, Mcp-Protocol-Version, Last-Event-ID"),
    );
    h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("86400"));
    res
}

async fn guard(State(core): State<Core>, req: Request, next: Next) -> Response {
    if req.method() == Method::OPTIONS {
        return preflight().await;
    }
    let s = settings(&core);
    let mut res = if !s.enabled {
        (StatusCode::NOT_FOUND, axum::Json(json!({"error": "MCP ist in den Einstellungen ausgeschaltet."}))).into_response()
    } else if !authorized(&core, &s, &req) {
        let mut r =
            (StatusCode::UNAUTHORIZED, axum::Json(json!({"error": "MCP-Schlüssel fehlt oder stimmt nicht (Authorization: Bearer …)."})))
                .into_response();
        r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer realm=\"nrzMonitor\""));
        r
    } else {
        next.run(req).await
    };
    cors(&mut res);
    res
}

// --- JSON-RPC ---------------------------------------------------------------------------------------

/// Fehler eines Aufrufs: (Code, Text).
pub type RpcErr = (i64, String);

pub fn invalid(msg: impl Into<String>) -> RpcErr {
    (-32602, msg.into())
}

fn rpc_ok(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_err(id: &Value, (code, message): RpcErr) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Alles, was ein Aufruf braucht.
pub struct Ctx {
    pub core: Core,
    pub session: Option<Arc<Session>>,
    pub settings: McpSettings,
    /// Für Fortschrittsmeldungen während eines Aufrufs (nur mit SSE-Antwort).
    pub notify: Option<mpsc::Sender<Value>>,
    pub progress_token: Option<Value>,
}

impl Ctx {
    pub fn api(&self) -> Router {
        hub().map_or_else(Router::new, |h| h.api.clone())
    }

    /// Fortschritt melden (`notifications/progress`), sofern der Aufrufer darum gebeten hat.
    pub async fn progress(&self, n: u64, total: Option<u64>, message: String) -> bool {
        let (Some(tx), Some(token)) = (&self.notify, &self.progress_token) else {
            return false;
        };
        let mut params = json!({"progressToken": token, "progress": n, "message": message});
        if let Some(t) = total {
            params["total"] = json!(t);
        }
        tx.send(json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": params})).await.is_ok()
    }
}

async fn post(State(core): State<Core>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(hub) = hub() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, axum::Json(rpc_err(&Value::Null, (-32700, format!("JSON nicht lesbar: {e}")))))
                .into_response();
        }
    };
    let (batch, items) = match msg {
        Value::Array(a) => (true, a),
        v => (false, vec![v]),
    };
    if items.is_empty() {
        return (StatusCode::BAD_REQUEST, axum::Json(rpc_err(&Value::Null, (-32600, "Leere Anfrage.".into())))).into_response();
    }
    let is_init = items.iter().any(|m| m.get("method").and_then(Value::as_str) == Some("initialize"));
    let session = match headers.get(SESSION_HEADER).and_then(|v| v.to_str().ok()) {
        Some(id) if !is_init => match hub.get(id) {
            Some(s) => Some(s),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    axum::Json(rpc_err(&Value::Null, (-32001, "Sitzung unbekannt – bitte neu verbinden.".into()))),
                )
                    .into_response();
            }
        },
        _ => None,
    };
    if let Some(s) = &session {
        s.touch();
    }
    let settings = settings(&core);
    let accepts_sse = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("text/event-stream"));

    // Ein einzelner Aufruf mit Fortschritts-Token → Antwort als Strom (Fortschritt, dann Ergebnis).
    if !batch && accepts_sse {
        let m = &items[0];
        let token = m.pointer("/params/_meta/progressToken").cloned();
        if let (Some(id), Some(token)) = (m.get("id").cloned(), token)
            && m.get("method").and_then(Value::as_str) == Some("tools/call")
        {
            {
                let (tx, rx) = mpsc::channel::<Value>(256);
                let ctx = Ctx { core, session, settings, notify: Some(tx.clone()), progress_token: Some(token) };
                let m = m.clone();
                tokio::spawn(async move {
                    let res = match dispatch(&ctx, &m).await {
                        Ok(r) => rpc_ok(&id, r),
                        Err(e) => rpc_err(&id, e),
                    };
                    let _ = tx.send(res).await;
                });
                return sse_response(rx, None);
            }
        }
    }

    let mut new_session = None;
    let mut out = Vec::new();
    for m in &items {
        let Some(method) = m.get("method").and_then(Value::as_str) else {
            continue;
        }; // Antwort des Clients
        let Some(id) = m.get("id") else { continue }; // Benachrichtigung (initialized, cancelled …)
        if method == "initialize" {
            let (result, s) = initialize(m.get("params").unwrap_or(&Value::Null));
            new_session = Some(s);
            out.push(rpc_ok(id, result));
            continue;
        }
        let ctx = Ctx { core: core.clone(), session: session.clone(), settings: settings.clone(), notify: None, progress_token: None };
        out.push(match dispatch(&ctx, m).await {
            Ok(r) => rpc_ok(id, r),
            Err(e) => rpc_err(id, e),
        });
    }
    if out.is_empty() {
        let mut res = StatusCode::ACCEPTED.into_response();
        if let Some(s) = new_session {
            set_session(&mut res, &s);
        }
        return res;
    }
    let body = if batch { Value::Array(out) } else { out.remove(0) };
    let mut res = axum::Json(body).into_response();
    if let Some(s) = new_session {
        set_session(&mut res, &s);
    }
    res
}

fn set_session(res: &mut Response, s: &Session) {
    if let Ok(v) = HeaderValue::from_str(&s.id) {
        res.headers_mut().insert(SESSION_HEADER, v);
    }
}

fn initialize(params: &Value) -> (Value, Arc<Session>) {
    let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL);
    let protocol = if SUPPORTED.contains(&asked) { asked } else { PROTOCOL };
    let client = params.pointer("/clientInfo/name").and_then(Value::as_str).unwrap_or("unbekannt").to_string();
    let session = Arc::new(Session {
        id: util::random_hex(16),
        protocol: protocol.to_string(),
        client: client.clone(),
        last_seen: AtomicI64::new(util::now()),
        subs: Mutex::new(HashSet::new()),
        log_level: Mutex::new(None),
        tx: Mutex::new(None),
    });
    if let Some(hub) = hub() {
        hub.sessions.lock().insert(session.id.clone(), session.clone());
    }
    tracing::info!("MCP: neue Sitzung von {client} (Protokoll {protocol})");
    let result = json!({
        "protocolVersion": protocol,
        "capabilities": {
            "tools": {"listChanged": false},
            "resources": {"subscribe": true, "listChanged": true},
            "prompts": {"listChanged": false},
            "logging": {},
            "completions": {},
        },
        "serverInfo": {"name": "nrzmonitor", "title": "nrzMonitor", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    });
    (result, session)
}

const INSTRUCTIONS: &str = "nrzMonitor überwacht einen Linux-Host, alle Docker-Container und das Netzwerk (DNS, Cloudflare, \
NetBird, Erreichbarkeit, Strom). Einstieg: `overview` (Zustand auf einen Blick). Diagramme finden mit `list_charts` \
(Filter section/context/label/search), aktuelle Werte mit `live_values`, Verläufe mit `query_timeseries` \
(after/before relativ wie \"-1h\", \"-7d\"), Kennzahlen mit `chart_stats`, Ranglisten mit `top_instances`, \
Ursachen mit `what_changed`. Container: `docker_containers`, `docker_container`, `docker_logs`. Ereignisse: `events`. \
Alles andere: `get_snapshot` oder `api_request` (ganze REST-API). Live mitlesen: `watch_live` oder Ressourcen \
abonnieren (nrzmon://chart/{id}, nrzmon://snap/{key}). Werte: Bytes, Bit/s (Netzwerk), Prozent 0–100 \
(Container-CPU: 100 = ein Kern), Zeiten in Unix-Sekunden.";

async fn dispatch(ctx: &Ctx, m: &Value) -> Result<Value, RpcErr> {
    let method = m.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = m.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools::list(&ctx.settings)})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).ok_or_else(|| invalid("`name` fehlt."))?;
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            Ok(tools::call(ctx, name, &args).await)
        }
        "resources/list" => Ok(json!({"resources": resources::list(&ctx.core)})),
        "resources/templates/list" => Ok(json!({"resourceTemplates": resources::templates()})),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).ok_or_else(|| invalid("`uri` fehlt."))?;
            resources::read(ctx, uri).await.map(|c| json!({"contents": [c]}))
        }
        "resources/subscribe" | "resources/unsubscribe" => {
            let uri = params.get("uri").and_then(Value::as_str).ok_or_else(|| invalid("`uri` fehlt."))?;
            let s = ctx.session.as_ref().ok_or_else(|| invalid("Abonnieren braucht eine Sitzung (Mcp-Session-Id)."))?;
            if method == "resources/subscribe" {
                s.subs.lock().insert(uri.to_string());
                start_pump(&ctx.core);
            } else {
                s.subs.lock().remove(uri);
            }
            Ok(json!({}))
        }
        "prompts/list" => Ok(json!({"prompts": resources::prompts()})),
        "prompts/get" => {
            let name = params.get("name").and_then(Value::as_str).ok_or_else(|| invalid("`name` fehlt."))?;
            resources::prompt(name, params.get("arguments").unwrap_or(&Value::Null))
        }
        "completion/complete" => Ok(json!({"completion": resources::complete(&ctx.core, &params)})),
        "logging/setLevel" => {
            let level = params.get("level").and_then(Value::as_str).unwrap_or("info");
            let s = ctx.session.as_ref().ok_or_else(|| invalid("Braucht eine Sitzung (Mcp-Session-Id)."))?;
            *s.log_level.lock() = Some(level_rank(level));
            start_pump(&ctx.core);
            Ok(json!({}))
        }
        _ => Err((-32601, format!("Unbekannte Methode `{method}`."))),
    }
}

// --- Ströme -----------------------------------------------------------------------------------------

/// Zählt offene Ströme, solange er lebt.
pub struct StreamGuard;

impl StreamGuard {
    pub fn new() -> Self {
        if let Some(h) = hub() {
            h.streams.fetch_add(1, Ordering::Relaxed);
        }
        Self
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        if let Some(h) = hub() {
            h.streams.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

fn rx_stream(rx: mpsc::Receiver<Value>, guard: Option<StreamGuard>) -> impl Stream<Item = Result<SseEvent, Infallible>> {
    futures_util::stream::unfold((rx, guard), |(mut rx, guard)| async move {
        let v = rx.recv().await?;
        Some((Ok(SseEvent::default().event("message").data(v.to_string())), (rx, guard)))
    })
}

fn sse_response(rx: mpsc::Receiver<Value>, guard: Option<StreamGuard>) -> Response {
    let mut res = Sse::new(rx_stream(rx, guard)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response();
    res.headers_mut().insert("x-accel-buffering", HeaderValue::from_static("no"));
    res
}

/// `GET /mcp`: Strom der Sitzung für Benachrichtigungen.
async fn open_stream(State(core): State<Core>, headers: HeaderMap) -> Response {
    let Some(hub) = hub() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Some(id) = headers.get(SESSION_HEADER).and_then(|v| v.to_str().ok()) else {
        return (StatusCode::BAD_REQUEST, axum::Json(json!({"error": "Mcp-Session-Id fehlt (erst initialize)."}))).into_response();
    };
    let Some(session) = hub.get(id) else {
        return (StatusCode::NOT_FOUND, axum::Json(json!({"error": "Sitzung unbekannt – bitte neu verbinden."}))).into_response();
    };
    session.touch();
    let (tx, rx) = mpsc::channel(512);
    *session.tx.lock() = Some(tx);
    start_pump(&core);
    sse_response(rx, Some(StreamGuard::new()))
}

async fn close(headers: HeaderMap) -> Response {
    if let (Some(hub), Some(id)) = (hub(), headers.get(SESSION_HEADER).and_then(|v| v.to_str().ok())) {
        hub.sessions.lock().remove(id);
    }
    StatusCode::NO_CONTENT.into_response()
}

pub fn level_rank(level: &str) -> u8 {
    match level {
        "debug" => 0,
        "info" => 1,
        "notice" => 2,
        "warning" => 3,
        "error" => 4,
        "critical" => 5,
        "alert" => 6,
        "emergency" => 7,
        _ => 1,
    }
}

fn severity_level(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Ok => "notice",
        Severity::Warn => "warning",
        Severity::Crit => "critical",
    }
}

fn updated(uri: &str) -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/resources/updated", "params": {"uri": uri}})
}

/// Eine Aufgabe für alle Sitzungen: Takte, Momentaufnahmen, Ereignisse → Benachrichtigungen.
fn start_pump(core: &Core) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    let core = core.clone();
    tokio::spawn(async move {
        let mut ticks = core.subscribe_ticks();
        let mut snaps = core.snaps.subscribe();
        let mut events = core.subscribe_events();
        let mut generation = core.registry.generation();
        let mut check = tokio::time::interval(Duration::from_secs(2));
        let mut sweep = tokio::time::interval(Duration::from_secs(60));
        loop {
            let Some(hub) = hub() else { return };
            tokio::select! {
                frame = ticks.recv() => {
                    let frame = match frame { Ok(f) => f, Err(RecvError::Lagged(_)) => continue, Err(RecvError::Closed) => return };
                    for s in hub.all().into_iter().filter(|s| s.streaming()) {
                        let subs = s.subs.lock().clone();
                        if subs.is_empty() { continue; }
                        let mut sent = HashSet::new();
                        for row in &frame.rows {
                            let id = &*row.chart.id;
                            for uri in &subs {
                                let hit = match uri.strip_prefix("nrzmon://") {
                                    Some(rest) => match rest.split_once('/') {
                                        Some(("chart", c)) => c == id,
                                        Some(("context", c)) => id == c || id.strip_prefix(c).is_some_and(|r| r.starts_with('.')),
                                        Some(("section", sec)) => core.registry.get(id).is_some_and(|m| m.section == sec),
                                        _ => rest == "overview" && id == "system.cpu",
                                    },
                                    None => false,
                                };
                                if hit && sent.insert(uri.clone()) {
                                    s.push(&updated(uri));
                                }
                            }
                        }
                    }
                }
                snap = snaps.recv() => {
                    let snap = match snap { Ok(s) => s, Err(RecvError::Lagged(_)) => continue, Err(RecvError::Closed) => return };
                    let uri = format!("nrzmon://snap/{}", snap.key);
                    for s in hub.all().into_iter().filter(|s| s.streaming()) {
                        if s.subs.lock().contains(&uri) { s.push(&updated(&uri)); }
                    }
                }
                ev = events.recv() => {
                    let ev = match ev { Ok(e) => e, Err(RecvError::Lagged(_)) => continue, Err(RecvError::Closed) => return };
                    let level = severity_level(ev.severity);
                    for s in hub.all().into_iter().filter(|s| s.streaming()) {
                        if s.subs.lock().contains("nrzmon://events/recent") { s.push(&updated("nrzmon://events/recent")); }
                        let min = *s.log_level.lock();
                        if min.is_some_and(|m| level_rank(level) >= m) {
                            s.push(&json!({"jsonrpc": "2.0", "method": "notifications/message",
                                "params": {"level": level, "logger": ev.source, "data": &*ev}}));
                        }
                    }
                }
                _ = check.tick() => {
                    let g = core.registry.generation();
                    if g != generation {
                        generation = g;
                        let msg = json!({"jsonrpc": "2.0", "method": "notifications/resources/list_changed"});
                        for s in hub.all() { s.push(&msg); }
                    }
                }
                _ = sweep.tick() => {
                    let now = util::now();
                    hub.sessions.lock().retain(|_, s| s.streaming() || now - s.last_seen.load(Ordering::Relaxed) < SESSION_IDLE);
                }
            }
        }
    });
}

/// Hilfe für interne Aufrufe der eigenen API: (Status, Inhalt).
pub async fn call_api(api: Router, method: Method, path: &str, body: Option<Value>) -> Result<(StatusCode, Bytes), String> {
    use tower::ServiceExt;
    let mut req = Request::builder().method(method).uri(path);
    let body = match body {
        Some(v) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let req = req.body(body).map_err(|e| format!("Ungültiger Pfad: {e}"))?;
    let Ok(res) = api.oneshot(req).await;
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024 * 1024).await.map_err(|e| e.to_string())?;
    Ok((status, bytes))
}
