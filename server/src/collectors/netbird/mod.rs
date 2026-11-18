//! NetBird über die Management-API (`Authorization: Token <PAT>`).
//!
//! - alle 30 s `GET /api/peers` → `netbird.peers`, Diagramme, Ereignis bei Peer online/offline
//! - jede Minute `GET /api/events/audit` → neue Einträge als `core.event`, Diagramm „Aktivität“
//! - alle 5 min Bestand: Benutzer, Gruppen, Regeln, Routen, Netzwerke, Setup-Schlüssel, DNS
//!
//! Der Merker des Aktivitätsprotokolls (`<data>/netbird-state.json`) verhindert, dass nach einem
//! Neustart alte Einträge noch einmal als Ereignis erscheinen.

mod demo;
#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared::{
    catalog::netbird::*,
    dns::TestResult,
    events::{Event, Severity},
    netbird::{Activity, DEFAULT_URL, Inventory, NetbirdSettings, NetbirdStatus, Peer},
};
use tokio::task::JoinHandle;

use crate::{
    collectors::dns::store::parse_time,
    config,
    core::Core,
    frame::Frame,
    registry::{ChartDef, ChartHandle},
    util,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    pub enabled: bool,
    pub url: String,
    pub token: String,
    pub token_from_env: bool,
}

impl Effective {
    pub fn configured(&self) -> bool {
        self.enabled && !self.token.is_empty()
    }
}

pub fn effective_from(s: &NetbirdSettings) -> Effective {
    let env = config::var("NRZMON_NETBIRD_TOKEN");
    let from_env = s.token.trim().is_empty() && env.is_some();
    let url = Some(s.url.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
        .or_else(|| config::var("NRZMON_NETBIRD_URL").map(|u| u.trim_end_matches('/').to_string()))
        .unwrap_or_else(|| DEFAULT_URL.to_string());
    Effective {
        enabled: s.enabled,
        url,
        token: if from_env { env.unwrap_or_default() } else { s.token.trim().to_string() },
        token_from_env: from_env,
    }
}

pub fn effective(core: &Core) -> Effective {
    effective_from(&core.settings.get::<NetbirdSettings>("netbird"))
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
                    Some(e) => tokio::spawn(async move { Runner::new(c, e).run().await }),
                    None if core.cfg.demo => tokio::spawn(async move { demo::run(c).await }),
                    None => {
                        core.snapshot("netbird.status", &NetbirdStatus::default());
                        core.snapshot("netbird.peers", &Vec::<Peer>::new());
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
pub enum NbError {
    Unreachable(String),
    Auth(String),
    Forbidden(String),
    Http(u16, String),
}

impl std::fmt::Display for NbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(e) => write!(f, "NetBird nicht erreichbar: {e}"),
            Self::Auth(m) => write!(f, "NetBird lehnt das Token ab ({m}) – Token prüfen"),
            Self::Forbidden(m) => write!(f, "keine Berechtigung ({m})"),
            Self::Http(s, m) => write!(f, "NetBird antwortet mit HTTP {s}: {m}"),
        }
    }
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Client {
    pub fn new(http: reqwest::Client, base: &str, token: &str) -> Self {
        Self { http, base: base.trim_end_matches('/').to_string(), token: token.to_string() }
    }

    pub async fn get(&self, path: &str) -> Result<Value, NbError> {
        let res = self
            .http
            .get(format!("{}{path}", self.base))
            .header("Authorization", format!("Token {}", self.token))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| NbError::Unreachable(if e.is_timeout() { "Zeitüberschreitung".into() } else { e.to_string().chars().take(160).collect() }))?;
        let status = res.status().as_u16();
        let text = res.text().await.map_err(|e| NbError::Unreachable(e.to_string()))?;
        if (200..300).contains(&status) {
            return serde_json::from_str(&text).map_err(|e| NbError::Http(status, format!("Antwort nicht lesbar: {e}")));
        }
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| text.chars().take(160).collect());
        Err(match status {
            401 => NbError::Auth(msg),
            403 => NbError::Forbidden(msg),
            s => NbError::Http(s, msg),
        })
    }

    async fn list(&self, path: &str) -> Result<Vec<Value>, NbError> {
        Ok(self.get(path).await?.as_array().cloned().unwrap_or_default())
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn id_of(v: &Value) -> String {
    match v.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

pub fn parse_peer(v: &Value) -> Peer {
    Peer {
        id: s(v, "id"),
        name: s(v, "name"),
        hostname: s(v, "hostname"),
        ip: s(v, "ip"),
        connection_ip: s(v, "connection_ip"),
        dns_label: s(v, "dns_label"),
        connected: v.get("connected").and_then(Value::as_bool).unwrap_or(false),
        last_seen: v.get("last_seen").and_then(Value::as_str).and_then(parse_time).unwrap_or(0),
        os: s(v, "os"),
        version: s(v, "version"),
        groups: v
            .get("groups")
            .and_then(Value::as_array)
            .map(|g| g.iter().map(|g| s(g, "name")).filter(|n| !n.is_empty()).collect())
            .unwrap_or_default(),
        country: s(v, "country_code"),
        city: s(v, "city_name"),
        user: s(v, "user_id"),
        ssh: v.get("ssh_enabled").and_then(Value::as_bool).unwrap_or(false),
        login_expired: v.get("login_expired").and_then(Value::as_bool).unwrap_or(false),
        approval_required: v.get("approval_required").and_then(Value::as_bool).unwrap_or(false),
    }
}

pub fn parse_activity(v: &Value) -> Activity {
    let meta_name = v.pointer("/meta/name").and_then(Value::as_str).unwrap_or("");
    let initiator = [s(v, "initiator_name"), s(v, "initiator_email"), s(v, "initiator_id")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
    Activity {
        id: id_of(v),
        ts: v.get("timestamp").and_then(Value::as_str).and_then(parse_time).unwrap_or(0),
        activity: s(v, "activity"),
        code: s(v, "activity_code"),
        initiator,
        target: if meta_name.is_empty() { s(v, "target_id") } else { meta_name.to_string() },
    }
}

/// Merker des Aktivitätsprotokolls.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Merker {
    last_ts: i64,
    ids: Vec<String>,
}

// --- Sammler -----------------------------------------------------------------------------------

pub struct Runner {
    core: Core,
    client: Client,
    status: NetbirdStatus,
    peers_chart: ChartHandle,
    inventory_chart: ChartHandle,
    events_chart: ChartHandle,
    online: HashMap<String, (bool, ChartHandle, String)>,
    merker: Option<Merker>,
    merker_path: PathBuf,
    last_inventory: i64,
    last_events: i64,
    events_minute: i64,
}

impl Runner {
    pub fn new(core: Core, eff: Effective) -> Self {
        let client = Client::new(core.http.clone(), &eff.url, &eff.token);
        let merker_path = core.cfg.data_dir.join("netbird-state.json");
        let merker = std::fs::read_to_string(&merker_path).ok().and_then(|t| serde_json::from_str(&t).ok());
        Self {
            peers_chart: core.chart(ChartDef::tpl(&NETBIRD_PEERS)),
            inventory_chart: core.chart(ChartDef::tpl(&NETBIRD_INVENTORY)),
            events_chart: core.chart(ChartDef::tpl(&NETBIRD_EVENTS)),
            status: NetbirdStatus { configured: true, url: eff.url.clone(), ..NetbirdStatus::default() },
            core,
            client,
            online: HashMap::new(),
            merker,
            merker_path,
            last_inventory: 0,
            last_events: 0,
            events_minute: util::now().div_euclid(60) * 60 - 60,
        }
    }

    pub async fn run(mut self) {
        loop {
            self.cycle(util::now()).await;
            let now = util::now_ms();
            let step = i64::from(STEP) * 1000;
            let next = (now.div_euclid(step) + 1) * step + 3000;
            tokio::time::sleep(std::time::Duration::from_millis((next - now) as u64)).await;
        }
    }

    pub async fn cycle(&mut self, now: i64) {
        match self.peers(now).await {
            Ok(()) => {
                self.status.ok = true;
                self.status.error = None;
                self.status.last_poll = util::now_ms();
            }
            Err(e) => {
                self.status.ok = false;
                self.status.error = Some(e.to_string());
                self.core.snapshot("netbird.status", &self.status);
                return;
            }
        }
        if now - self.last_inventory >= 300 {
            self.last_inventory = now;
            self.inventory(now).await;
        }
        if now - self.last_events >= 60 {
            self.last_events = now;
            if let Err(e) = self.activity(now).await
                && !self.status.hints.iter().any(|h| h.starts_with("Aktivität"))
            {
                self.status.hints.push(format!("Aktivität: {e}"));
            }
        }
        self.core.snapshot("netbird.status", &self.status);
    }

    async fn peers(&mut self, now: i64) -> Result<(), NbError> {
        let list: Vec<Peer> = self.client.list("/api/peers").await?.iter().map(parse_peer).collect();
        let connected = list.iter().filter(|p| p.connected).count();
        self.status.peers = list.len() as u32;
        self.status.connected = connected as u32;
        let ts = now.div_euclid(i64::from(STEP)) * i64::from(STEP);
        let mut f = Frame::new(ts);
        f.put(&self.peers_chart, &[connected as f64, (list.len() - connected) as f64]);
        let mut seen = HashSet::new();
        for p in &list {
            let name = if p.name.is_empty() { p.id.clone() } else { p.name.clone() };
            seen.insert(p.id.clone());
            match self.online.get_mut(&p.id) {
                Some((was, chart, _)) => {
                    if *was != p.connected {
                        let ev = if p.connected {
                            Event::new("netbird", "peer.online", Severity::Ok, &name, format!("Peer {name} verbunden"))
                        } else {
                            Event::new("netbird", "peer.offline", Severity::Warn, &name, format!("Peer {name} getrennt"))
                        };
                        self.core.event(ev.message(format!("{} · {}", p.ip, p.os)).label("peer", p.id.clone()));
                        *was = p.connected;
                    }
                    f.put(chart, &[if p.connected { 1.0 } else { 0.0 }]);
                }
                None => {
                    let chart = self.core.chart(ChartDef::tpl(&NETBIRD_PEER_ONLINE).instance(&name).label("ip", p.ip.clone()));
                    f.put(&chart, &[if p.connected { 1.0 } else { 0.0 }]);
                    self.online.insert(p.id.clone(), (p.connected, chart, name));
                }
            }
        }
        let gone: Vec<String> = self.online.keys().filter(|k| !seen.contains(*k)).cloned().collect();
        for id in gone {
            if let Some((_, _, name)) = self.online.remove(&id) {
                self.core.registry.retire(&NETBIRD_PEER_ONLINE.chart_id(Some(&name)));
            }
        }
        self.core.submit(f);
        let mut list = list;
        list.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        self.core.snapshot("netbird.peers", &list);
        Ok(())
    }

    async fn inventory(&mut self, now: i64) {
        let mut hints = Vec::new();
        let parts = [
            ("/api/users", "Benutzer"),
            ("/api/groups", "Gruppen"),
            ("/api/policies", "Regeln"),
            ("/api/routes", "Routen"),
            ("/api/networks", "Netzwerke"),
            ("/api/setup-keys", "Setup-Schlüssel"),
            ("/api/dns/nameservers", "DNS-Server"),
        ];
        let mut counts = [f64::NAN; 7];
        let mut inv = Inventory::default();
        for (i, (path, label)) in parts.iter().enumerate() {
            let list = match self.client.list(path).await {
                Ok(l) => l,
                Err(NbError::Forbidden(_) | NbError::Auth(_)) => {
                    hints.push(format!("{label}: nicht lesbar – das Token braucht Administrator-Rechte."));
                    continue;
                }
                Err(e) => {
                    hints.push(format!("{label}: {e}"));
                    continue;
                }
            };
            counts[i] = list.len() as f64;
            let n = list.len() as u32;
            let st = &mut self.status;
            match i {
                0 => {
                    inv.users = n;
                    st.users = list.iter().map(|u| [s(u, "name"), s(u, "email")].into_iter().find(|x| !x.is_empty()).unwrap_or_default()).collect();
                }
                1 => {
                    inv.groups = n;
                    st.groups = list.iter().map(|g| (s(g, "name"), g.get("peers_count").and_then(Value::as_u64).unwrap_or(0) as u32)).collect();
                }
                2 => {
                    inv.policies = n;
                    st.policies = list.iter().map(|p| (s(p, "name"), p.get("enabled").and_then(Value::as_bool).unwrap_or(true))).collect();
                }
                3 => {
                    inv.routes = n;
                    st.routes = list
                        .iter()
                        .map(|r| {
                            let net = [s(r, "network"), s(r, "domains")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
                            format!("{} {}", s(r, "network_id"), net).trim().to_string()
                        })
                        .collect();
                }
                4 => {
                    inv.networks = n;
                    st.networks = list.iter().map(|x| s(x, "name")).collect();
                }
                5 => {
                    inv.setup_keys = n;
                    st.setup_keys = list.iter().map(|k| (s(k, "name"), k.get("valid").and_then(Value::as_bool).unwrap_or(false))).collect();
                }
                _ => {
                    inv.nameservers = n;
                    st.nameservers = list.iter().map(|x| s(x, "name")).collect();
                }
            }
        }
        self.status.inventory = inv;
        self.status.hints = hints;
        let mut f = Frame::new(now.div_euclid(300) * 300);
        f.put(&self.inventory_chart, &counts);
        self.core.submit(f);
    }

    async fn activity(&mut self, now: i64) -> Result<(), NbError> {
        let mut list: Vec<Activity> = self.client.list("/api/events/audit").await?.iter().map(parse_activity).collect();
        list.sort_by(|a, b| b.ts.cmp(&a.ts).then(b.id.cmp(&a.id)));
        // Neue Einträge seit dem Merker als Ereignis (beim allerersten Lauf nur merken).
        let first = self.merker.is_none();
        let merker = self.merker.get_or_insert_with(Merker::default);
        let mut fresh: Vec<&Activity> =
            list.iter().filter(|a| a.ts > merker.last_ts || (a.ts == merker.last_ts && !merker.ids.contains(&a.id))).collect();
        fresh.reverse();
        if !first {
            for a in &fresh {
                let ev = Event::new("netbird", "activity", Severity::Info, &a.target, a.activity.clone())
                    .message(if a.initiator.is_empty() { String::new() } else { format!("von {}", a.initiator) })
                    .label("code", a.code.clone());
                self.core.event(Event { ts: a.ts * 1000, ..ev });
            }
        }
        if let Some(newest) = list.first() {
            let ts = newest.ts;
            merker.ids = list.iter().filter(|a| a.ts == ts).map(|a| a.id.clone()).collect();
            merker.last_ts = ts;
            if let Ok(t) = serde_json::to_string(&*merker) {
                let _ = std::fs::write(&self.merker_path, t);
            }
        }
        // Aktivität je Minute (abgeschlossene Minuten, jede einmal).
        let current = now.div_euclid(60) * 60;
        let oldest = current - 14 * 60;
        let mut m = self.events_minute.max(oldest);
        while m < current {
            let n = list.iter().filter(|a| a.ts >= m && a.ts < m + 60).count();
            let mut f = Frame::new(m);
            f.put(&self.events_chart, &[n as f64]);
            self.core.submit(f);
            m += 60;
        }
        self.events_minute = current;
        list.truncate(100);
        self.status.activity = list;
        Ok(())
    }
}

/// „Verbindung testen“.
pub async fn test(core: &Core, s: &NetbirdSettings) -> TestResult {
    let eff = effective_from(s);
    if eff.token.is_empty() {
        return TestResult { ok: false, message: "Kein Token hinterlegt.".into(), details: vec![] };
    }
    let c = Client::new(core.http.clone(), &eff.url, &eff.token);
    let peers = match c.list("/api/peers").await {
        Ok(p) => p,
        Err(e) => return TestResult { ok: false, message: e.to_string(), details: vec![] },
    };
    let connected = peers.iter().filter(|p| p.get("connected").and_then(Value::as_bool) == Some(true)).count();
    let mut details = vec![format!("{} Peers, {connected} verbunden", peers.len())];
    for (path, label) in [("/api/users", "Benutzer"), ("/api/events/audit", "Aktivitätsprotokoll")] {
        match c.list(path).await {
            Ok(l) => details.push(format!("{label}: {} Einträge", l.len())),
            Err(e) => details.push(format!("{label}: {e}")),
        }
    }
    TestResult { ok: true, message: "Verbindung in Ordnung.".into(), details }
}
