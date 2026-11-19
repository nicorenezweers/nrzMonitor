//! Für Dashboards ohne MCP: ein einfacher Live-Strom (Server-Sent Events) und die REST-API mit dem
//! MCP-Schlüssel.
//!
//! `GET /mcp/stream?charts=system.cpu,docker.cpu.*&context=…&section=…&label=app=x&snaps=docker.containers&events=1&token=…`
//!
//! Ereignisse im Strom (`event:` + JSON in `data:`):
//! - `hello`   `{charts: [...], snaps: [...]}` – Auswahl und Beschreibung der Diagramme
//! - `tick`    `{t, c: {id: [werte je Dimension]}}` – jede neue Messung
//! - `snap`    `{key, ts, data}` – Momentaufnahme hat sich geändert (beim Start einmal alle)
//! - `event`   ein Ereignis (Container gestartet, Alarm …)
//! - `catalog` `{generation}` – Diagramme kamen dazu oder fielen weg (Auswahl wird neu gebildet)
//!
//! Im Browser: `new EventSource("/mcp/stream?section=system&token=…")`.

use std::{collections::HashMap, convert::Infallible, time::Duration};

use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
};
use serde_json::{Map, Value, json};
use tokio::sync::{broadcast::error::RecvError, mpsc};

use super::{StreamGuard, hub, settings, tools};
use crate::core::Core;

fn args_from(q: &HashMap<String, String>) -> Value {
    let mut a = Map::new();
    for k in ["charts", "section", "context", "family", "label", "search"] {
        if let Some(v) = q.get(k).filter(|v| !v.is_empty()) {
            a.insert(k.to_string(), json!(v));
        }
    }
    Value::Object(a)
}

pub async fn sse(State(core): State<Core>, Query(q): Query<HashMap<String, String>>) -> Response {
    let mut args = args_from(&q);
    let snaps: Vec<String> =
        q.get("snaps").map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()).unwrap_or_default();
    let events = q.get("events").is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes"));
    if args.as_object().is_some_and(Map::is_empty) && snaps.is_empty() && !events {
        args = json!({"section": "system"});
    }
    let limit = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(500usize).clamp(1, 5000);
    let (tx, rx) = mpsc::channel::<SseEvent>(512);
    let guard = StreamGuard::new();
    tokio::spawn(async move {
        let _guard = guard;
        let pick = |core: &Core| -> Vec<shared::ChartMeta> {
            if args.as_object().is_some_and(Map::is_empty) {
                return Vec::new();
            }
            let mut l = tools::select(core, &args);
            l.truncate(limit);
            l
        };
        let send = |name: &str, v: Value| SseEvent::default().event(name).data(v.to_string());
        let mut charts = pick(&core);
        let mut wanted: std::collections::HashSet<String> = charts.iter().map(|c| c.id.clone()).collect();
        let hello = json!({"charts": charts.iter().map(tools::compact).collect::<Vec<_>>(), "snaps": snaps, "events": events});
        if tx.send(send("hello", hello)).await.is_err() {
            return;
        }
        // Aktueller Stand gleich zu Beginn.
        let mut init = Map::new();
        for c in &charts {
            init.insert(c.id.clone(), tools::latest(&core, c));
        }
        if !init.is_empty() && tx.send(send("latest", Value::Object(init))).await.is_err() {
            return;
        }
        for k in &snaps {
            if let Some(s) = core.snaps.get(k)
                && tx.send(send("snap", json!({"key": k, "ts": s.ts, "data": s.data}))).await.is_err()
            {
                return;
            }
        }
        let mut ticks = core.subscribe_ticks();
        let mut snap_rx = core.snaps.subscribe();
        let mut ev_rx = core.subscribe_events();
        let mut generation = core.registry.generation();
        let mut check = tokio::time::interval(Duration::from_secs(2));
        loop {
            let msg = tokio::select! {
                () = tx.closed() => return,
                f = ticks.recv() => match f {
                    Ok(frame) => {
                        if wanted.is_empty() { continue; }
                        let mut c = Map::new();
                        for row in &frame.rows {
                            if wanted.contains(&*row.chart.id) {
                                let vals: Vec<Option<f64>> = frame.values_of(row).iter().map(|v| v.is_finite().then_some(*v)).collect();
                                c.insert(row.chart.id.to_string(), json!(vals));
                            }
                        }
                        if c.is_empty() { continue; }
                        send("tick", json!({"t": frame.ts, "c": c}))
                    }
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                },
                s = snap_rx.recv() => match s {
                    Ok(s) if snaps.contains(&s.key) => send("snap", json!({"key": s.key, "ts": s.ts, "data": s.data})),
                    Ok(_) | Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                },
                e = ev_rx.recv() => match e {
                    Ok(e) if events => send("event", json!(&*e)),
                    Ok(_) | Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                },
                _ = check.tick() => {
                    let g = core.registry.generation();
                    if g == generation { continue; }
                    generation = g;
                    charts = pick(&core);
                    wanted = charts.iter().map(|c| c.id.clone()).collect();
                    send("catalog", json!({"generation": g, "charts": charts.iter().map(tools::compact).collect::<Vec<_>>()}))
                }
            };
            if tx.send(msg).await.is_err() {
                return;
            }
        }
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (Ok::<_, Infallible>(e), rx)) });
    let mut res = Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response();
    res.headers_mut().insert("x-accel-buffering", HeaderValue::from_static("no"));
    res
}

/// `/mcp/api/v1/{path}` → eigene API (ohne zweite Anmeldung). Ändernd nur mit `allow_write`.
pub async fn proxy(
    State(core): State<Core>,
    method: Method,
    Path(path): Path<String>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let bare = format!("/{}", path.trim_start_matches('/'));
    if bare.contains("..") || ["/live", "/login", "/logout"].contains(&bare.as_str()) {
        return (StatusCode::FORBIDDEN, axum::Json(json!({"error": format!("`{bare}` ist über MCP nicht erreichbar.")}))).into_response();
    }
    if method != Method::GET && method != Method::HEAD && !settings(&core).allow_write {
        return (StatusCode::FORBIDDEN, axum::Json(json!({"error": "Ändernde Aufrufe sind aus (Einstellungen → MCP)."}))).into_response();
    }
    let Some(hub) = hub() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let target = match uri.query() {
        Some(q) => format!("/api/v1{bare}?{q}"),
        None => format!("/api/v1{bare}"),
    };
    let mut req = axum::http::Request::builder().method(method).uri(target);
    if let Some(ct) = headers.get(header::CONTENT_TYPE) {
        req = req.header(header::CONTENT_TYPE, ct);
    }
    let Ok(req) = req.body(Body::from(body)) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    use tower::ServiceExt;
    match hub.api.clone().oneshot(req).await {
        Ok(r) => r.into_response(),
        Err(e) => match e {},
    }
}
