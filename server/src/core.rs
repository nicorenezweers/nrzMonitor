//! Der Kern: ein billig klonbarer Griff auf alles, was Sammler und API brauchen.

use std::{
    collections::BTreeMap,
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use parking_lot::Mutex;
use serde::Serialize;
use shared::events::Event;
use tokio::sync::broadcast;

use crate::{
    auth::Sessions,
    config::Config,
    db::Db,
    frame::Frame,
    registry::{ChartDef, ChartHandle, Registry},
    ring::Ring,
    settings::Settings,
    snaps::Snaps,
    store::Store,
    util,
};

pub struct Inner {
    pub cfg: Config,
    pub db: Db,
    pub registry: Arc<Registry>,
    pub ring: Arc<Ring>,
    pub store: Arc<Store>,
    pub snaps: Snaps,
    pub settings: Settings,
    pub sessions: Sessions,
    /// HTTP-Client für Integrationen (Zeitlimit 20 s, gzip).
    pub http: reqwest::Client,
    /// Unix-Sekunden.
    pub started_at: i64,
    pub hostname: String,
    /// Dauer des letzten Laufs je Sammler in Millisekunden.
    pub durations: Mutex<BTreeMap<String, f64>>,
    pub live_clients: AtomicUsize,
    ticks: broadcast::Sender<Arc<Frame>>,
    events: broadcast::Sender<Arc<Event>>,
}

#[derive(Clone)]
pub struct Core(Arc<Inner>);

impl Deref for Core {
    type Target = Inner;

    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl Core {
    pub fn new(cfg: Config) -> anyhow::Result<Self> {
        let db = Db::open(&cfg)?;
        let registry = Arc::new(Registry::load(Some(&db), cfg.interval)?);
        let ring = Arc::new(Ring::new(cfg.live_window));
        let store = Store::open(&cfg, db.clone(), registry.clone(), ring.clone())?;
        let settings = Settings::load(cfg.data_dir.join("settings.json"));
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .user_agent(concat!("nrzMonitor/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self(Arc::new(Inner {
            cfg,
            db,
            registry,
            ring,
            store,
            snaps: Snaps::default(),
            settings,
            sessions: Sessions::default(),
            http,
            started_at: util::now(),
            hostname: hostname(),
            durations: Mutex::new(BTreeMap::new()),
            live_clients: AtomicUsize::new(0),
            ticks: broadcast::channel(256).0,
            events: broadcast::channel(256).0,
        })))
    }

    /// Diagramm anmelden (siehe [`Registry::chart`]).
    pub fn chart(&self, def: ChartDef) -> ChartHandle {
        self.registry.chart(def)
    }

    /// Werte eines Takts abgeben: in den Arbeitsspeicher, an den Speicher und an die Live-Zuschauer.
    pub fn submit(&self, frame: Frame) {
        if frame.is_empty() {
            return;
        }
        self.ring.write(&frame);
        self.store.ingest(&frame);
        if self.ticks.receiver_count() > 0 {
            let _ = self.ticks.send(Arc::new(frame));
        }
    }

    /// Ereignis festhalten (Zeit wird gesetzt, wenn sie fehlt).
    pub fn event(&self, ev: Event) {
        let ev = self.store.add_event(ev);
        let _ = self.events.send(Arc::new(ev));
    }

    /// Momentaufnahme ablegen (Schlüssel z. B. `docker.containers`).
    pub fn snapshot<T: Serialize>(&self, key: &str, data: &T) {
        match serde_json::to_value(data) {
            Ok(v) => self.snaps.put(key, util::now_ms(), v),
            Err(e) => tracing::warn!("Momentaufnahme {key}: {e}"),
        }
    }

    pub fn subscribe_ticks(&self) -> broadcast::Receiver<Arc<Frame>> {
        self.ticks.subscribe()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<Arc<Event>> {
        self.events.subscribe()
    }

    pub fn record_duration(&self, collector: &str, took: Duration) {
        self.durations.lock().insert(collector.to_string(), util::round(took.as_secs_f64() * 1000.0, 2));
    }

    pub fn live_clients(&self) -> usize {
        self.live_clients.load(Ordering::Relaxed)
    }
}

fn hostname() -> String {
    // Im Container mit Host-Netzwerk ist das der Name des Hosts.
    if let Some(h) = crate::config::var("NRZMON_HOSTNAME") {
        return h;
    }
    let mut buf = [0u8; 256];
    // SAFETY: Puffer ist gültig und gross genug; gethostname schreibt höchstens `len` Bytes.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    if ok {
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        if let Ok(s) = std::str::from_utf8(&buf[..end]) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    "localhost".to_string()
}
