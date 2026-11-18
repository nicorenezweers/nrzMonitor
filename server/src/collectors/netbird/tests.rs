//! Tests gegen eine nachgebaute NetBird-Management-API.

use std::sync::Arc;

use axum::{Json, Router, extract::State, http::HeaderMap, http::StatusCode, response::IntoResponse, routing::get};
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::*;
use crate::config::Config;

#[derive(Default)]
struct Mock {
    laptop_connected: bool,
    events: Vec<Value>,
}

type Shared = Arc<Mutex<Mock>>;

fn authed(h: &HeaderMap) -> bool {
    h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Token nb-pat")
}

async fn peers(State(m): State<Shared>, h: HeaderMap) -> axum::response::Response {
    if !authed(&h) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"message": "token invalid", "code": 401}))).into_response();
    }
    let c = m.lock().laptop_connected;
    Json(json!([
        {"id": "p1", "name": "server", "ip": "100.64.0.1", "connected": true, "last_seen": "2026-11-15T10:00:00Z", "os": "Linux",
         "version": "0.31.0", "groups": [{"id": "g1", "name": "All"}], "country_code": "CH", "city_name": "Zürich"},
        {"id": "p2", "name": "laptop", "ip": "100.64.0.2", "connected": c, "last_seen": "2026-11-15T09:00:00.123Z", "os": "Darwin",
         "version": "0.30.0", "groups": []}
    ]))
    .into_response()
}

async fn list(h: HeaderMap) -> axum::response::Response {
    if !authed(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!([{"id": "1", "name": "eins", "peers_count": 2, "enabled": true, "valid": true}])).into_response()
}

async fn forbidden() -> axum::response::Response {
    (StatusCode::FORBIDDEN, Json(json!({"message": "user is not admin", "code": 403}))).into_response()
}

async fn audit(State(m): State<Shared>) -> Json<Value> {
    Json(Value::Array(m.lock().events.clone()))
}

async fn mock() -> (String, Shared) {
    let state: Shared = Arc::new(Mutex::new(Mock { laptop_connected: true, events: vec![ev(1, 100)] }));
    let app = Router::new()
        .route("/api/peers", get(peers))
        .route("/api/users", get(list))
        .route("/api/groups", get(list))
        .route("/api/policies", get(list))
        .route("/api/routes", get(list))
        .route("/api/networks", get(list))
        .route("/api/setup-keys", get(forbidden))
        .route("/api/dns/nameservers", get(list))
        .route("/api/events/audit", get(audit))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), state)
}

fn ev(id: u32, ago: i64) -> Value {
    json!({"id": id.to_string(), "timestamp": crate::collectors::dns::store::format_time(util::now() - ago), "activity": format!("Ereignis {id}"),
           "activity_code": "peer.login", "initiator_name": "Nico", "target_id": "p1", "meta": {"name": "server"}})
}

fn test_core() -> (Core, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("nrzmon-nb-{}-{}", std::process::id(), fastrand::u32(..)));
    let cfg = Config {
        bind: vec!["127.0.0.1:0".parse().unwrap()],
        data_dir: dir.clone(),
        proc: "/nonexistent".into(),
        sys: "/nonexistent".into(),
        root: "/".into(),
        docker_sock: "/nonexistent".into(),
        interval: 1,
        live_window: 900,
        demo: false,
        password: None,
        web_dir: None,
        db_memory: "128MB".into(),
        db_threads: 1,
    };
    (Core::new(cfg).unwrap(), dir)
}

fn eff(url: &str, token: &str) -> Effective {
    Effective { enabled: true, url: url.into(), token: token.into(), token_from_env: false }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peers_events_inventory_merker() {
    let (core, dir) = test_core();
    let (url, m) = mock().await;
    let mut events = core.subscribe_events();
    let mut r = Runner::new(core.clone(), eff(&url, "nb-pat"));
    let now = util::now();
    r.cycle(now).await;
    assert!(r.status.ok, "{:?}", r.status.error);
    assert_eq!((r.status.peers, r.status.connected), (2, 2));
    assert_eq!(r.status.inventory.users, 1);
    assert_eq!(r.status.inventory.setup_keys, 0);
    assert!(r.status.hints.iter().any(|h| h.contains("Setup-Schlüssel")), "{:?}", r.status.hints);
    let peers: Vec<Peer> = serde_json::from_value(core.snaps.get("netbird.peers").unwrap().data.clone()).unwrap();
    assert_eq!(peers[1].country, "CH");
    assert_eq!(peers[0].last_seen, parse_time("2026-11-15T09:00:00Z").unwrap());
    assert!(core.registry.handle("netbird.peer.online.laptop").is_some());

    // Erster Lauf: alte Aktivität nur gemerkt, kein Ereignis. Peer geht offline → Ereignis.
    m.lock().laptop_connected = false;
    m.lock().events.push(ev(2, 10));
    r.last_events = 0;
    r.cycle(now + 30).await;
    let e1 = events.recv().await.unwrap();
    assert_eq!(e1.kind, "peer.offline");
    let e2 = events.recv().await.unwrap();
    assert_eq!((e2.kind.as_str(), e2.title.as_str()), ("activity", "Ereignis 2"));

    // Neustart: Merker aus der Datei, nichts doppelt.
    let mut r = Runner::new(core.clone(), eff(&url, "nb-pat"));
    m.lock().events.push(ev(3, 5));
    r.cycle(now + 60).await;
    let e = events.recv().await.unwrap();
    assert_eq!(e.title, "Ereignis 3");
    assert!(events.try_recv().is_err(), "keine weiteren Ereignisse");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_token_and_test() {
    let (core, dir) = test_core();
    let (url, _m) = mock().await;
    let mut r = Runner::new(core.clone(), eff(&url, "falsch"));
    r.cycle(util::now()).await;
    assert!(!r.status.ok);
    assert!(r.status.error.as_deref().unwrap_or("").contains("Token"));
    let res = test(&core, &NetbirdSettings { url: url.clone(), token: "nb-pat".into(), ..NetbirdSettings::default() }).await;
    assert!(res.ok && res.details[0].starts_with("2 Peers"), "{res:?}");
    let res = test(&core, &NetbirdSettings { url, token: "x".into(), ..NetbirdSettings::default() }).await;
    assert!(!res.ok);
    let _ = std::fs::remove_dir_all(dir);
}
