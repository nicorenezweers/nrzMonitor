//! Live-Kanal zum Server: **eine** WebSocket-Verbindung für die ganze Oberfläche, verbindet sich
//! selbst neu. Komponenten abonnieren, was sie zeigen, und lösen das Abo beim Verschwinden.
//!
//! - [`use_tick`]: neueste Werte eines Diagramms (für eigene Anzeigen; `ChartCard` nutzt das selbst).
//! - [`use_snap`]: Momentaufnahme (Liste, Zustand) – immer der neueste Stand.
//! - [`use_events`]: laufende Ereignisse.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use dioxus::prelude::*;
use futures::{SinkExt, StreamExt, channel::mpsc};
use gloo_net::websocket::{Message, futures::WebSocket};
use serde::de::DeserializeOwned;
use shared::{
    events::Event,
    query::{LiveCmd, LiveMsg},
};

use crate::state::App;

/// Neueste Werte eines Diagramms: (Unix-Sekunden, Werte in der Reihenfolge der Dimensionen).
pub type Tick = Option<(i64, Rc<Vec<Option<f64>>>)>;
/// Neueste Momentaufnahme: (Unix-Millisekunden, Daten).
pub type Snap = Option<(i64, Rc<serde_json::Value>)>;

const EVENTS_KEPT: usize = 300;

struct Slot<T: 'static> {
    signal: Signal<T>,
    refs: usize,
}

struct Inner {
    charts: RefCell<HashMap<String, Slot<Tick>>>,
    snaps: RefCell<HashMap<String, Slot<Snap>>>,
    event_refs: Cell<usize>,
    tx: RefCell<Option<mpsc::UnboundedSender<LiveCmd>>>,
}

#[derive(Clone)]
pub struct Live {
    inner: Rc<Inner>,
    /// Verbindung zum Server steht.
    pub connected: Signal<bool>,
    /// Neueste Ereignisse zuerst (nur solange jemand [`use_events`] nutzt).
    pub events: Signal<Vec<Event>>,
}

impl Live {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(Inner {
                charts: RefCell::new(HashMap::new()),
                snaps: RefCell::new(HashMap::new()),
                event_refs: Cell::new(0),
                tx: RefCell::new(None),
            }),
            connected: Signal::new_in_scope(false, ScopeId::ROOT),
            events: Signal::new_in_scope(Vec::new(), ScopeId::ROOT),
        }
    }

    fn send(&self, cmd: LiveCmd) {
        if let Some(tx) = self.inner.tx.borrow().as_ref() {
            let _ = tx.unbounded_send(cmd);
        }
    }

    fn acquire_chart(&self, id: &str) -> Signal<Tick> {
        let mut map = self.inner.charts.borrow_mut();
        let slot = map.entry(id.to_string()).or_insert_with(|| Slot { signal: Signal::new_in_scope(None, ScopeId::ROOT), refs: 0 });
        slot.refs += 1;
        let (signal, first) = (slot.signal, slot.refs == 1);
        drop(map);
        if first {
            self.send(LiveCmd::Sub { charts: vec![id.to_string()], snaps: vec![], events: false });
        }
        signal
    }

    fn release_chart(&self, id: &str) {
        let mut map = self.inner.charts.borrow_mut();
        let Some(slot) = map.get_mut(id) else { return };
        slot.refs = slot.refs.saturating_sub(1);
        let last = slot.refs == 0;
        drop(map);
        if last {
            self.send(LiveCmd::Unsub { charts: vec![id.to_string()], snaps: vec![], events: false });
        }
    }

    fn acquire_snap(&self, key: &str) -> Signal<Snap> {
        let mut map = self.inner.snaps.borrow_mut();
        let slot = map.entry(key.to_string()).or_insert_with(|| Slot { signal: Signal::new_in_scope(None, ScopeId::ROOT), refs: 0 });
        slot.refs += 1;
        let (signal, first) = (slot.signal, slot.refs == 1);
        drop(map);
        if first {
            self.send(LiveCmd::Sub { charts: vec![], snaps: vec![key.to_string()], events: false });
        }
        signal
    }

    fn release_snap(&self, key: &str) {
        let mut map = self.inner.snaps.borrow_mut();
        let Some(slot) = map.get_mut(key) else { return };
        slot.refs = slot.refs.saturating_sub(1);
        let last = slot.refs == 0;
        drop(map);
        if last {
            self.send(LiveCmd::Unsub { charts: vec![], snaps: vec![key.to_string()], events: false });
        }
    }

    /// Alles, was gerade jemand braucht (nach einem Neuverbinden wieder abonnieren).
    fn wanted(&self) -> LiveCmd {
        LiveCmd::Sub {
            charts: self.inner.charts.borrow().iter().filter(|(_, s)| s.refs > 0).map(|(k, _)| k.clone()).collect(),
            snaps: self.inner.snaps.borrow().iter().filter(|(_, s)| s.refs > 0).map(|(k, _)| k.clone()).collect(),
            events: self.inner.event_refs.get() > 0,
        }
    }

    fn handle(&self, text: &str, app: App) {
        let Ok(msg) = serde_json::from_str::<LiveMsg>(text) else { return };
        let mut connected = self.connected;
        if !*connected.peek() {
            connected.set(true);
        }
        match msg {
            LiveMsg::Tick { t, c } => {
                for (id, values) in c {
                    let signal = self.inner.charts.borrow().get(&id).map(|s| s.signal);
                    if let Some(mut signal) = signal {
                        signal.set(Some((t, Rc::new(values))));
                    }
                }
            }
            LiveMsg::Snap { key, ts, data } => {
                let signal = self.inner.snaps.borrow().get(&key).map(|s| s.signal);
                if let Some(mut signal) = signal {
                    signal.set(Some((ts, Rc::new(data))));
                }
            }
            LiveMsg::Event { ev } => {
                let mut events = self.events;
                let mut list = events.write();
                list.insert(0, ev);
                list.truncate(EVENTS_KEPT);
            }
            LiveMsg::Catalog { .. } => {
                spawn(async move { app.reload_charts().await });
            }
            LiveMsg::Pong => {}
        }
    }

    /// Verbindung halten – läuft für immer (einmal in `state::provide` gestartet).
    pub async fn run(&self, app: App) {
        let mut wait_ms = 500u32;
        loop {
            if let Ok(ws) = WebSocket::open(&ws_url()) {
                let (mut write, mut read) = ws.split();
                let (tx, mut rx) = mpsc::unbounded::<LiveCmd>();
                let _ = tx.unbounded_send(LiveCmd::Ping);
                let _ = tx.unbounded_send(self.wanted());
                *self.inner.tx.borrow_mut() = Some(tx);

                let writer = async move {
                    while let Some(cmd) = rx.next().await {
                        let Ok(text) = serde_json::to_string(&cmd) else { continue };
                        if write.send(Message::Text(text)).await.is_err() {
                            break;
                        }
                    }
                };
                let reader = async {
                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                wait_ms = 500;
                                self.handle(&text, app);
                            }
                            Ok(Message::Bytes(_)) => {}
                            Err(_) => break,
                        }
                    }
                };
                futures::pin_mut!(writer, reader);
                futures::future::select(writer, reader).await;
                *self.inner.tx.borrow_mut() = None;
                let mut connected = self.connected;
                connected.set(false);
            }
            gloo_timers::future::TimeoutFuture::new(wait_ms).await;
            wait_ms = (wait_ms * 2).min(8000);
        }
    }
}

fn ws_url() -> String {
    let location = web_sys::window().map(|w| w.location());
    let protocol = location.as_ref().and_then(|l| l.protocol().ok()).unwrap_or_default();
    let host = location.as_ref().and_then(|l| l.host().ok()).unwrap_or_default();
    let scheme = if protocol == "https:" { "wss" } else { "ws" };
    format!("{scheme}://{host}/api/v1/live")
}

/// Neueste Werte eines Diagramms. Die ID muss über die Lebenszeit der Komponente gleich bleiben
/// (Komponenten mit wechselnder ID bekommen ein `key`).
pub fn use_tick(id: &str) -> Signal<Tick> {
    let live = use_context::<Live>();
    let id = id.to_string();
    let signal = use_hook({
        let (live, id) = (live.clone(), id.clone());
        move || live.acquire_chart(&id)
    });
    use_drop(move || live.release_chart(&id));
    signal
}

/// Momentaufnahme als Typ aus `shared` – `None`, solange noch keine da ist oder sie nicht passt.
pub fn use_snap<T: DeserializeOwned + PartialEq + 'static>(key: &str) -> Memo<Option<T>> {
    let raw = use_snap_raw(key);
    use_memo(move || raw.read().as_ref().and_then(|(_, v)| serde_json::from_value::<T>((**v).clone()).ok()))
}

/// Momentaufnahme mit Zeit (Unix-Millisekunden) als rohes JSON.
pub fn use_snap_raw(key: &str) -> Signal<Snap> {
    let live = use_context::<Live>();
    let key = key.to_string();
    let signal = use_hook({
        let (live, key) = (live.clone(), key.clone());
        move || live.acquire_snap(&key)
    });
    use_drop(move || live.release_snap(&key));
    signal
}

/// Laufende Ereignisse, neueste zuerst (ältere holt man über `GET /api/v1/events`).
pub fn use_events() -> Signal<Vec<Event>> {
    let live = use_context::<Live>();
    use_hook({
        let live = live.clone();
        move || {
            let n = live.inner.event_refs.get();
            live.inner.event_refs.set(n + 1);
            if n == 0 {
                live.send(LiveCmd::Sub { charts: vec![], snaps: vec![], events: true });
            }
        }
    });
    let events = live.events;
    use_drop(move || {
        let n = live.inner.event_refs.get().saturating_sub(1);
        live.inner.event_refs.set(n);
        if n == 0 {
            live.send(LiveCmd::Unsub { charts: vec![], snaps: vec![], events: true });
        }
    });
    events
}

/// Verbindung zum Server steht.
pub fn use_connected() -> Signal<bool> {
    use_context::<Live>().connected
}
