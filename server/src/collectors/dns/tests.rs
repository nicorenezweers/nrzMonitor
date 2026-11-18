//! Tests gegen einen nachgebauten MCP-Server von nrzNetwork (lokaler HTTP-Server).

use std::sync::Arc;

use axum::{Json, Router, extract::State, http::HeaderMap, response::IntoResponse, routing::post};
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::*;
use crate::config::Config;

#[derive(Default)]
struct Mock {
    /// (id, at_ms, client, qname, kind)
    log: Vec<(i64, i64, String, String, String)>,
    /// Statuscode für alle Antworten (Ausfall).
    fail: Option<u16>,
    calls: u32,
    sse: bool,
    max_limit_seen: u64,
}

type Shared = Arc<Mutex<Mock>>;

async fn handler(State(m): State<Shared>, headers: HeaderMap, Json(req): Json<Value>) -> axum::response::Response {
    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    if auth != "Bearer geheim" {
        return (axum::http::StatusCode::UNAUTHORIZED, Json(json!({"error": {"code": 401, "message": "Token fehlt oder ist falsch."}})))
            .into_response();
    }
    let mut m = m.lock();
    m.calls += 1;
    if let Some(code) = m.fail {
        return (axum::http::StatusCode::from_u16(code).unwrap(), "kaputt").into_response();
    }
    let id = req["id"].clone();
    let result = match req["method"].as_str().unwrap_or("") {
        "initialize" => json!({"protocolVersion": "2025-06-18", "serverInfo": {"name": "nrzDNS", "title": "nrzNetwork", "version": "9.9"}}),
        "tools/call" => {
            let args = &req["params"]["arguments"];
            match req["params"]["name"].as_str().unwrap_or("") {
                "get_query_log" => {
                    let from = args["from"].as_i64().unwrap_or(0) * 1000;
                    let to = (args["to"].as_i64().unwrap_or(i64::MAX / 2000) + 1) * 1000;
                    let limit = args["limit"].as_u64().unwrap_or(50);
                    m.max_limit_seen = m.max_limit_seen.max(limit);
                    let limit = limit.clamp(1, 500) as usize;
                    let page = args["page"].as_u64().unwrap_or(1).max(1) as usize;
                    let mut rows: Vec<_> = m.log.iter().filter(|e| e.1 >= from && e.1 < to).cloned().collect();
                    rows.sort_by(|a, b| (b.1, b.0).cmp(&(a.1, a.0)));
                    let total = rows.len();
                    let entries: Vec<Value> = rows
                        .into_iter()
                        .skip((page - 1) * limit)
                        .take(limit)
                        .map(|(id, at, client, qname, kind)| {
                            json!({"id": id, "atMs": at, "client": client, "protocol": "udp", "qname": qname, "qtype": "A",
                                   "rcode": "NOERROR", "kind": kind, "answer": "1.2.3.4", "durationMs": 7, "upstream": ""})
                        })
                        .collect();
                    let page_json = json!({"entries": entries, "total": total, "page": page, "perPage": limit});
                    json!({"content": [{"type": "text", "text": page_json.to_string()}], "isError": false})
                }
                "get_server_status" => {
                    let s = json!({"version": "9.9", "cacheEntries": 42, "queriesPerSecond": 1.5,
                                   "upstreams": [{"upstream": "tls://1.1.1.1:853", "name": "Cloudflare", "ok": 10, "failed": 1, "avgMs": 12.5}]});
                    json!({"content": [{"type": "text", "text": s.to_string()}], "structuredContent": s, "isError": false})
                }
                "get_blocking_status" => {
                    let s = json!({"enabled": true, "totalDomains": 1000, "lists": [{"name": "L", "url": "u", "enabled": true, "domainCount": 1000}]});
                    json!({"content": [{"type": "text", "text": s.to_string()}], "isError": false})
                }
                _ => json!({"content": [{"type": "text", "text": "Unbekannt"}], "isError": true}),
            }
        }
        _ => return Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}})).into_response(),
    };
    let msg = json!({"jsonrpc": "2.0", "id": id, "result": result});
    if m.sse {
        return ([("content-type", "text/event-stream")], format!("event: message\ndata: {msg}\n\n")).into_response();
    }
    Json(msg).into_response()
}

async fn mock() -> (String, Shared) {
    let state: Shared = Arc::default();
    let app = Router::new().route("/mcp", post(handler)).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/mcp"), state)
}

fn test_core(dir: &std::path::Path) -> Core {
    let cfg = Config {
        bind: vec!["127.0.0.1:0".parse().unwrap()],
        data_dir: dir.to_path_buf(),
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
    Core::new(cfg).unwrap()
}

fn tmpdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("nrzmon-dns-{name}-{}-{}", std::process::id(), fastrand::u32(..)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn add(m: &Shared, from_id: i64, n: i64, start_ms: i64, step_ms: i64) {
    let mut m = m.lock();
    for i in 0..n {
        let id = from_id + i;
        m.log.push((id, start_ms + i * step_ms, format!("192.168.1.{}", id % 7), format!("d{}.example", id % 13), "forwarded".into()));
    }
}

fn eff(url: &str) -> Effective {
    Effective { enabled: true, url: url.into(), token: "geheim".into(), token_from_env: false, retention_days: 30 }
}

async fn runner(core: &Core, store: Arc<DnsStore>, url: &str) -> Runner {
    let src = Source::Mcp(Mcp::new(core.http.clone(), url, "geheim"));
    Runner::new(core, store, src, eff(url)).await.unwrap()
}

fn count(store: &DnsStore) -> (u64, u64) {
    let p = store.page(&QueryFilter { per_page: 1, ..QueryFilter::default() }).unwrap();
    let distinct = store.clients(&QueryFilter::default(), 1000).unwrap().iter().map(|c| c.queries).sum();
    (p.total, distinct)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pages_merker_duplicates_restart() {
    let dir = tmpdir("a");
    let core = test_core(&dir);
    let (url, m) = mock().await;
    let now = util::now();
    // 1234 Anfragen in der letzten Stunde → mehrere Seiten zu 500.
    add(&m, 1, 1234, (now - 3500) * 1000, 2000);
    let store = Arc::new(DnsStore::open(core.db.conn().unwrap(), dir.join("dns")).unwrap());
    let mut r = runner(&core, store.clone(), &url).await;
    r.sync_log(now).await.unwrap();
    assert_eq!(count(&store).0, 1234);
    assert_eq!(m.lock().max_limit_seen, 500, "nie mehr als 500 je Seite");
    assert_eq!(store.state("synced_until").unwrap(), Some(now));

    // Nochmals ohne neue Einträge: Überlappung bringt nichts doppelt.
    r.sync_log(now + 5).await.unwrap();
    assert_eq!(count(&store).0, 1234);

    // Neue Einträge, auch verspätete innerhalb der Überlappung.
    add(&m, 2000, 10, (now - 10) * 1000, 100);
    add(&m, 3000, 20, (now + 6) * 1000, 100);
    r.sync_log(now + 10).await.unwrap();
    assert_eq!(count(&store).0, 1264);

    // Neustart: Merker aus der Datenbank, keine Doppelten.
    drop(r);
    let mut r = runner(&core, store.clone(), &url).await;
    assert_eq!(r.synced, now + 10);
    add(&m, 4000, 5, (now + 12) * 1000, 100);
    r.sync_log(now + 15).await.unwrap();
    assert_eq!(count(&store).0, 1269);
    let p = store.page(&QueryFilter { per_page: 500, ..QueryFilter::default() }).unwrap();
    let mut ids: Vec<i64> = p.entries.iter().map(|e| e.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 500);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outage_and_catch_up_with_slices() {
    let dir = tmpdir("b");
    let core = test_core(&dir);
    let (url, m) = mock().await;
    let now = util::now();
    let store = Arc::new(DnsStore::open(core.db.conn().unwrap(), dir.join("dns")).unwrap());
    store.insert(&[], Some(now - 6 * 3600)).unwrap();
    // Sechs Stunden Ausfall mit 30 000 Anfragen (zu viel für eine Scheibe von 1 h → halbieren).
    add(&m, 1, 30_000, (now - 6 * 3600 + 1) * 1000, 700);
    let mut r = runner(&core, store.clone(), &url).await;
    m.lock().fail = Some(503);
    let e = r.sync_log(now).await.unwrap_err();
    assert!(e.contains("503"), "{e}");
    assert_eq!(store.state("synced_until").unwrap(), Some(now - 6 * 3600), "Merker bleibt beim Ausfall stehen");
    m.lock().fail = None;
    // Aufholen über mehrere Durchgänge (höchstens MAX_CALLS Aufrufe je Durchgang).
    for i in 0..20 {
        r.sync_log(now).await.unwrap();
        if !r.status.catching_up {
            break;
        }
        assert!(i < 19, "holt nicht auf");
    }
    let (total, _) = count(&store);
    let expected = m.lock().log.iter().filter(|e| e.1 <= (now + 1) * 1000 - 1).count() as u64;
    assert_eq!(total, expected);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn errors_status_sse_and_test() {
    let (url, m) = mock().await;
    let http = reqwest::Client::new();
    let bad = Mcp::new(http.clone(), &url, "falsch");
    assert!(matches!(bad.initialize().await, Err(mcp::McpError::Auth(_))));
    let good = Mcp::new(http.clone(), &url, "geheim");
    assert_eq!(good.initialize().await.unwrap(), "nrzNetwork 9.9");
    assert!(matches!(good.call("gibts_nicht", json!({})).await, Err(mcp::McpError::Tool(_))));
    assert!(matches!(good.rpc("foo/bar", json!({})).await, Err(mcp::McpError::Rpc(_))));
    m.lock().sse = true;
    let (s, _) = good.call_as::<ServerStatus>("get_server_status", json!({})).await.unwrap();
    assert_eq!(s.cache_entries, 42);
    assert_eq!(s.upstreams[0].avg_ms, 12.5);
    let unreachable = Mcp::new(http, "http://127.0.0.1:1/mcp", "geheim");
    assert!(matches!(unreachable.initialize().await, Err(mcp::McpError::Unreachable(_))));

    let dir = tmpdir("c");
    let core = test_core(&dir);
    let res = test(&core, &DnsSettings { url: url.clone(), token: "geheim".into(), ..DnsSettings::default() }).await;
    assert!(res.ok, "{res:?}");
    let res = test(&core, &DnsSettings { url, token: "nein".into(), ..DnsSettings::default() }).await;
    assert!(!res.ok && res.message.contains("Token"), "{res:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn charts_status_and_archive() {
    let dir = tmpdir("d");
    let core = test_core(&dir);
    let (url, m) = mock().await;
    let now = util::now();
    add(&m, 1, 300, (now - 300) * 1000, 1000);
    let store = Arc::new(DnsStore::open(core.db.conn().unwrap(), dir.join("dns")).unwrap());
    let mut r = runner(&core, store.clone(), &url).await;
    r.cycle().await;
    assert!(r.status.connected, "{:?}", r.status.error);
    assert_eq!(r.status.cache_entries, 42);
    assert_eq!(r.status.blocklists.len(), 1);
    let snap = core.snaps.get("dns.status").expect("Momentaufnahme");
    assert_eq!(snap.data["cache_entries"], 42);
    // Diagramme: Abschnitte bis synced − Überlappung abgegeben.
    let h = core.registry.handle("dns.queries").expect("Diagramm");
    assert!(core.ring.last_ts(h.series[0]).is_some());
    assert!(core.registry.handle("dns.upstream.latency.tls___1.1.1.1_853").is_some());

    // Archiv: alte Tage als Parquet, danach weiter abfragbar.
    let old = (now - 3 * 86_400) * 1000;
    let rows: Vec<QueryEntry> = (0..5000)
        .map(|i| QueryEntry {
            ts: old + i * 10,
            id: 100_000 + i,
            client: format!("192.168.1.{}", i % 9),
            proto: "udp".into(),
            name: format!("host{}.example.com", i % 50),
            qtype: "A".into(),
            rcode: "NOERROR".into(),
            kind: if i % 7 == 0 { "blocked" } else { "cached" }.into(),
            answer: "1.2.3.4".into(),
            ms: (i % 30) as u32,
            upstream: String::new(),
        })
        .collect();
    store.insert(&rows, None).unwrap();
    let moved = store.archive(now * 1000).unwrap();
    assert!(moved >= 5000);
    assert!(std::fs::read_dir(dir.join("dns")).unwrap().count() >= 1);
    let stats = store.stats().unwrap();
    assert_eq!(stats.rows, 5300);
    assert!(stats.bytes_per_query > 0.0 && stats.bytes_per_query < 20.0, "{}", stats.bytes_per_query);
    let top = store.top(&QueryFilter { from: Some(now - 4 * 86_400), ..QueryFilter::default() }, 5).unwrap();
    assert_eq!(top.total, 5300);
    let d = store.domain("host1.example.com", &QueryFilter { from: Some(now - 4 * 86_400), ..QueryFilter::default() }).unwrap();
    assert_eq!(d.queries, 100);
    store.retain(2, now * 1000).unwrap();
    assert_eq!(store.stats().unwrap().rows, 300);
    let _ = std::fs::remove_dir_all(dir);
}

/// Bytes je Anfrage bei realistischen Daten (Beispielgenerator, ein Tag).
#[test]
fn bytes_per_query_realistic() {
    let dir = tmpdir("e");
    let store = DnsStore::open(duckdb::Connection::open(dir.join("t.duckdb")).unwrap(), dir.join("dns")).unwrap();
    let mut g = demo::Gen::new();
    let day = util::now().div_euclid(86_400) * 86_400 - 2 * 86_400;
    let rows = g.generate(day, day + 86_399);
    let n = rows.len();
    store.insert(&rows, None).unwrap();
    store.archive(util::now_ms()).unwrap();
    let s = store.stats().unwrap();
    eprintln!("{n} Anfragen → {:.1} Bytes je Anfrage ({} Bytes)", s.bytes_per_query, s.bytes);
    assert_eq!(s.rows as usize, n);
    assert!(s.bytes_per_query < 16.0);
    let _ = std::fs::remove_dir_all(dir);
}
