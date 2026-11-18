//! Live-Kanal (`GET /api/v1/live`, WebSocket): der Browser abonniert Diagramme, Momentaufnahmen und
//! Ereignisse und bekommt nur das. Format: `shared::query::{LiveCmd, LiveMsg}` als JSON-Text.

use std::{
    collections::{BTreeMap, HashSet},
    sync::atomic::Ordering,
    time::Duration,
};

use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use shared::query::{LiveCmd, LiveMsg};
use tokio::sync::broadcast::error::RecvError;

use crate::core::Core;

pub async fn upgrade(State(core): State<Core>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| async move {
        core.live_clients.fetch_add(1, Ordering::Relaxed);
        run(&core, socket).await;
        core.live_clients.fetch_sub(1, Ordering::Relaxed);
    })
}

async fn send(socket: &mut WebSocket, msg: &LiveMsg) -> bool {
    match serde_json::to_string(msg) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => true,
    }
}

async fn run(core: &Core, mut socket: WebSocket) {
    let mut charts: HashSet<String> = HashSet::new();
    let mut snaps: HashSet<String> = HashSet::new();
    let mut events = false;
    let mut ticks = core.subscribe_ticks();
    let mut snap_rx = core.snaps.subscribe();
    let mut event_rx = core.subscribe_events();
    let mut generation = core.registry.generation();
    let mut check = tokio::time::interval(Duration::from_secs(2));

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(msg)) = incoming else { break };
                let Message::Text(text) = msg else { continue };
                let Ok(cmd) = serde_json::from_str::<LiveCmd>(&text) else { continue };
                match cmd {
                    LiveCmd::Sub { charts: c, snaps: s, events: e } => {
                        charts.extend(c);
                        events |= e;
                        for key in s {
                            // Den aktuellen Stand gleich mitgeben.
                            if let Some(snap) = core.snaps.get(&key) {
                                let msg = LiveMsg::Snap { key: key.clone(), ts: snap.ts, data: snap.data.clone() };
                                if !send(&mut socket, &msg).await { return; }
                            }
                            snaps.insert(key);
                        }
                    }
                    LiveCmd::Unsub { charts: c, snaps: s, events: e } => {
                        for id in &c { charts.remove(id); }
                        for key in &s { snaps.remove(key); }
                        if e { events = false; }
                    }
                    LiveCmd::Ping => {
                        if !send(&mut socket, &LiveMsg::Pong).await { break; }
                    }
                }
            }
            frame = ticks.recv() => {
                let frame = match frame {
                    Ok(f) => f,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                };
                if charts.is_empty() { continue; }
                let mut c = BTreeMap::new();
                for row in &frame.rows {
                    if charts.contains(&*row.chart.id) {
                        let values = frame.values_of(row).iter().map(|v| v.is_finite().then_some(*v)).collect();
                        c.insert(row.chart.id.to_string(), values);
                    }
                }
                if !c.is_empty() && !send(&mut socket, &LiveMsg::Tick { t: frame.ts, c }).await { break; }
            }
            snap = snap_rx.recv() => {
                let snap = match snap {
                    Ok(s) => s,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                };
                if snaps.contains(&snap.key) {
                    let msg = LiveMsg::Snap { key: snap.key.clone(), ts: snap.ts, data: snap.data.clone() };
                    if !send(&mut socket, &msg).await { break; }
                }
            }
            ev = event_rx.recv() => {
                let ev = match ev {
                    Ok(e) => e,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                };
                if events && !send(&mut socket, &LiveMsg::Event { ev: (*ev).clone() }).await { break; }
            }
            _ = check.tick() => {
                let g = core.registry.generation();
                if g != generation {
                    generation = g;
                    if !send(&mut socket, &LiveMsg::Catalog { generation: g }).await { break; }
                }
            }
        }
    }
}
