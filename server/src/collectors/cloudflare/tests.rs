//! Tests gegen eine nachgebaute Cloudflare-API (REST + GraphQL).

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::*;
use crate::config::Config;

#[derive(Default)]
struct Mock {
    tunnel_status: String,
    rate_limit: bool,
    graphql_calls: Vec<String>,
}

type Shared = Arc<Mutex<Mock>>;

fn ok(result: Value) -> Json<Value> {
    Json(json!({"success": true, "errors": [], "result": result}))
}

fn authed(h: &HeaderMap) -> bool {
    h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer cf-token")
}

async fn zones(State(m): State<Shared>, h: HeaderMap, Query(q): Query<std::collections::HashMap<String, String>>) -> axum::response::Response {
    if !authed(&h) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"success": false, "errors": [{"code": 10000, "message": "Authentication error"}]}))).into_response();
    }
    if m.lock().rate_limit {
        return (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "120")], "slow down").into_response();
    }
    let page: u32 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let z = if page == 1 {
        json!([{"id": "z1", "name": "free.example", "status": "active", "plan": {"name": "Free Website"}, "account": {"id": "acc1"}}])
    } else {
        json!([{"id": "z2", "name": "pro.example", "status": "active", "plan": {"name": "Pro Website"}, "account": {"id": "acc1"}}])
    };
    Json(json!({"success": true, "errors": [], "result": z, "result_info": {"page": page, "total_pages": 2}})).into_response()
}

async fn tunnels(State(m): State<Shared>) -> Json<Value> {
    let s = m.lock().tunnel_status.clone();
    ok(json!([{"id": "t1", "name": "home", "status": s, "created_at": "2025-01-01T00:00:00Z",
               "connections": [{"colo_name": "ZRH"}, {"colo_name": "FRA"}]}]))
}

async fn verify() -> Json<Value> {
    ok(json!({"status": "active"}))
}

async fn graphql(State(m): State<Shared>, Json(req): Json<Value>) -> Json<Value> {
    let q = req["query"].as_str().unwrap_or("").to_string();
    let zone = req["variables"]["zone"].as_str().unwrap_or("").to_string();
    let from = req["variables"]["from"].as_str().and_then(parse_time).unwrap_or(0);
    let to = req["variables"]["to"].as_str().and_then(parse_time).unwrap_or(0);
    let minutes: Vec<String> = (from..to).step_by(60).map(format_time).collect();
    let kind = if q.contains("settings") {
        "settings"
    } else if q.contains("httpRequests1mGroups") {
        "1m"
    } else if q.contains("httpRequestsAdaptiveGroups") {
        "adaptive"
    } else if q.contains("firewallEventsAdaptiveGroups") {
        "fwgroups"
    } else {
        "fwraw"
    };
    m.lock().graphql_calls.push(format!("{zone}:{kind}"));
    let zone_data = match (kind, zone.as_str()) {
        // Free-Zone: `settings` sagt 1m erlaubt, die Abfrage scheitert trotzdem → Rückfall auf adaptive.
        ("settings", _) => json!({"settings": {
            "httpRequests1mGroups": {"enabled": true}, "httpRequestsAdaptiveGroups": {"enabled": true},
            "firewallEventsAdaptiveGroups": {"enabled": zone == "z2"}, "firewallEventsAdaptive": {"enabled": true}}}),
        ("1m", "z1") => return Json(json!({"data": null, "errors": [{"message": "zone 'z1' does not have access to the path"}]})),
        ("1m", _) => json!({"httpRequests1mGroups": minutes.iter().map(|m| json!({
            "dimensions": {"datetimeMinute": m},
            "sum": {"requests": 100, "cachedRequests": 60, "bytes": 1000, "cachedBytes": 600, "threats": 2, "pageViews": 30,
                    "countryMap": [{"clientCountryName": "CH", "requests": 70}, {"clientCountryName": "DE", "requests": 30}],
                    "responseStatusMap": [{"edgeResponseStatus": 200, "requests": 90}, {"edgeResponseStatus": 404, "requests": 10}]},
            "uniq": {"uniques": 9}})).collect::<Vec<_>>()}),
        ("adaptive", _) => json!({
            "cache": minutes.iter().flat_map(|m| [
                json!({"count": 40, "sum": {"edgeResponseBytes": 400, "visits": 5}, "dimensions": {"datetimeMinute": m, "cacheStatus": "hit"}}),
                json!({"count": 10, "sum": {"edgeResponseBytes": 100, "visits": 1}, "dimensions": {"datetimeMinute": m, "cacheStatus": "dynamic"}})]).collect::<Vec<_>>(),
            "status": minutes.iter().map(|m| json!({"count": 50, "dimensions": {"datetimeMinute": m, "edgeResponseStatus": 200}})).collect::<Vec<_>>(),
            "country": minutes.iter().map(|m| json!({"count": 50, "dimensions": {"datetimeMinute": m, "clientCountryName": "CH"}})).collect::<Vec<_>>()}),
        ("fwgroups", _) => json!({"firewallEventsAdaptiveGroups": minutes.iter().map(|m| json!({"count": 3, "dimensions": {"datetimeMinute": m, "action": "block"}})).collect::<Vec<_>>()}),
        _ => json!({"firewallEventsAdaptive": minutes.iter().map(|m| json!({"action": "managed_challenge", "datetime": m})).collect::<Vec<_>>()}),
    };
    Json(json!({"data": {"viewer": {"zones": [zone_data]}}, "errors": null}))
}

async fn mock() -> (String, Shared) {
    let state: Shared = Arc::new(Mutex::new(Mock { tunnel_status: "healthy".into(), ..Mock::default() }));
    let app = Router::new()
        .route("/zones", get(zones))
        .route("/accounts/acc1/cfd_tunnel", get(tunnels))
        .route("/user/tokens/verify", get(verify))
        .route("/graphql", post(graphql))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), state)
}

fn test_core() -> (Core, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("nrzmon-cf-{}-{}", std::process::id(), fastrand::u32(..)));
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

fn eff(token: &str) -> Effective {
    Effective { enabled: true, token: token.into(), token_from_env: false, zones: vec![], account_id: String::new() }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zones_fallback_minutes_and_tunnel_events() {
    let (core, dir) = test_core();
    let (base, m) = mock().await;
    let mut events = core.subscribe_events();
    let mut r = Runner::new(core.clone(), &base, eff("cf-token"));
    let now = util::now();
    r.cycle(now).await;
    assert!(r.status.ok, "{:?}", r.status.error);
    assert_eq!(r.zones.len(), 2, "Zonen über zwei Seiten");
    assert_eq!(r.status.account_id, "acc1");
    let z1 = &r.zones[0];
    assert_eq!(z1.http, Http::Adaptive, "Free-Zone fällt auf adaptive zurück");
    assert_eq!(z1.fw, Fw::Raw);
    assert_eq!(r.zones[1].http, Http::OneMinute);
    assert_eq!(r.zones[1].fw, Fw::Groups);
    // Minuten bis jetzt − LAG je einmal abgegeben.
    let until = (now - LAG).div_euclid(60) * 60;
    assert_eq!(r.zones[1].last, until);
    let h = core.registry.handle("cf.requests.pro.example").expect("Diagramm");
    let (ts, v) = core.ring.latest(h.series[0]).expect("Wert");
    assert_eq!(ts, until);
    assert_eq!(v, 60.0);
    let fw = core.registry.handle("cf.firewall.free.example").expect("Firewall");
    assert_eq!(core.ring.latest(fw.series[1]).map(|x| x.1), Some(1.0), "Challenge aus Einzelereignissen");
    let countries = core.registry.get("cf.countries.pro.example").unwrap();
    assert_eq!(countries.dims.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["CH", "DE", "other"]);
    let snap = core.snaps.get("cloudflare.status").unwrap();
    assert_eq!(snap.data["tunnels"][0]["connections"], 2);
    assert!(snap.data["zones"][1]["requests_24h"].as_f64().unwrap() > 0.0);

    // Gleiche Minute nochmals: keine weitere GraphQL-Datenabfrage.
    let calls = m.lock().graphql_calls.len();
    r.cycle(now + 10).await;
    assert_eq!(m.lock().graphql_calls.len(), calls);

    // Tunnel fällt aus → Ereignis.
    m.lock().tunnel_status = "down".into();
    r.cycle(now + 60).await;
    let ev = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv()).await.unwrap().unwrap();
    assert_eq!(ev.kind, "tunnel.down");
    assert_eq!(ev.severity, Severity::Crit);

    // Ratenbegrenzung → Pause nach Retry-After.
    m.lock().rate_limit = true;
    r.zones_at = 0;
    r.cycle(now + 120).await;
    assert_eq!(r.backoff_until, now + 240);
    let before = m.lock().graphql_calls.len();
    r.cycle(now + 180).await;
    assert_eq!(m.lock().graphql_calls.len(), before, "während der Pause keine Aufrufe");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_token_and_test() {
    let (core, dir) = test_core();
    let (base, _m) = mock().await;
    let mut r = Runner::new(core.clone(), &base, eff("falsch"));
    r.cycle(util::now()).await;
    assert!(!r.status.ok);
    assert!(r.status.error.as_deref().unwrap_or("").contains("Token"), "{:?}", r.status.error);
    let res = test_at(&core, &base, &CloudflareSettings { token: "cf-token".into(), ..CloudflareSettings::default() }).await;
    assert!(res.ok, "{res:?}");
    assert!(res.details.iter().any(|d| d.contains("free.example")), "{res:?}");
    assert!(res.details.iter().any(|d| d.starts_with("Tunnel: 1")), "{res:?}");
    let res = test_at(&core, &base, &CloudflareSettings { token: "nein".into(), ..CloudflareSettings::default() }).await;
    assert!(!res.ok);
    let _ = std::fs::remove_dir_all(dir);
}
