//! Speicher: nimmt jeden Takt entgegen, beantwortet Abfragen und hält Ereignisse fest.
//!
//! Gestuft (siehe `ARCHITECTURE.md`, Abschnitt 4):
//! - **RAM** (`ring.rs`): die letzten Minuten, daraus kommt alles ganz Frische.
//! - **Stufe 0** Sekundenwerte, **Stufe 1** Minuten, **Stufe 2** Stunden (je Mittel, Minimum, Maximum).
//!
//! `ingest` legt die Werte nur in einen Kanal. Ein eigener Thread ([`writer`]) schreibt gebündelt
//! in DuckDB-Tabellen je offenem Abschnitt, verdichtet jede abgeschlossene Stunde zu Minuten- und
//! Stundenwerten und legt abgeschlossene Abschnitte als Parquet (ZSTD, sortiert nach `(series, ts)`)
//! unter `<data>/tiers/…` ab. Aufbewahrung und Obergrenze löschen ganze Dateien.

mod analyze;
#[cfg(test)]
mod bench;
mod events;
mod layout;
mod read;
#[cfg(test)]
mod tests;
mod writer;

use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender},
    },
    time::Duration,
};

use anyhow::Context;
use duckdb::Connection;
use parking_lot::{Mutex, RwLock};
use shared::{
    events::{Event, EventPage, EventQuery},
    query::{ChartData, QueryReq, QueryResp},
    storage::{StorageSettings, StorageStats, TIER_RES, TierStats},
};

pub use self::read::Acc;
use self::{
    layout::{Catalog, Clock, SECTION, TIERS, table},
    read::Plan,
    writer::Msg,
};
use crate::{config::Config, db::Db, frame::Frame, registry::Registry, ring::Ring};

/// Was Schreib-Thread und Abfragen gemeinsam sehen.
pub(crate) struct Shared {
    pub dir: PathBuf,
    pub db: Db,
    pub registry: Arc<Registry>,
    pub ring: Arc<Ring>,
    pub clock: Clock,
    /// Start dieses Prozesses (Unix-Sekunden).
    pub started: i64,
    pub catalog: RwLock<Catalog>,
    pub settings: RwLock<StorageSettings>,
    /// Aufgenommen, aber noch nicht geschrieben.
    pub pending_events: Mutex<Vec<Event>>,
    pub samples: AtomicU64,
    pub dropped: AtomicU64,
    pub events_count: AtomicU64,
    /// (Zeit, Werte bis dahin) – für Werte je Sekunde.
    pub rate: Mutex<VecDeque<(i64, u64)>>,
    /// Lese-Verbindungen zur Wiederverwendung.
    readers: Mutex<Vec<Connection>>,
}

impl Shared {
    /// Grösse der DuckDB-Datei samt Protokoll.
    pub fn db_bytes(&self) -> u64 {
        let p = self.db.path();
        let wal = p.with_extension("duckdb.wal");
        [p.to_path_buf(), wal].iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum()
    }

    /// Mit einer Lese-Verbindung arbeiten (blockiert – nur aus `spawn_blocking`/Threads).
    pub fn with_reader<T>(&self, f: impl FnOnce(&Connection) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let conn = match self.readers.lock().pop() {
            Some(c) => c,
            None => self.db.conn()?,
        };
        let out = f(&conn);
        let mut pool = self.readers.lock();
        if pool.len() < 4 {
            pool.push(conn);
        }
        out
    }
}

pub struct Store {
    pub(crate) sh: Arc<Shared>,
    tx: Mutex<Option<Sender<Msg>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    next_event: AtomicU64,
}

impl Store {
    pub fn open(cfg: &Config, db: Db, registry: Arc<Registry>, ring: Arc<Ring>) -> anyhow::Result<Arc<Self>> {
        Self::open_at(cfg, db, registry, ring, 0)
    }

    /// Wie [`Store::open`], mit fester Uhr (`clock` ≠ 0, nur Tests).
    pub(crate) fn open_at(cfg: &Config, db: Db, registry: Arc<Registry>, ring: Arc<Ring>, clock: i64) -> anyhow::Result<Arc<Self>> {
        let settings = read_settings(&cfg.data_dir);
        let sh = Arc::new(Shared {
            dir: cfg.data_dir.clone(),
            db: db.clone(),
            registry,
            ring,
            clock: Clock::default(),
            started: if clock != 0 { clock } else { crate::util::now() },
            catalog: RwLock::new(Catalog::default()),
            settings: RwLock::new(settings),
            pending_events: Mutex::new(Vec::new()),
            samples: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            events_count: AtomicU64::new(0),
            rate: Mutex::new(VecDeque::new()),
            readers: Mutex::new(Vec::new()),
        });
        #[cfg(test)]
        if clock != 0 {
            sh.clock.set(clock);
        }
        #[cfg(not(test))]
        let _ = clock;
        let conn = db.conn()?;
        writer::prepare(&sh, &conn)?;
        let next: Option<u64> = conn.query_row("SELECT max(id) FROM events", [], |r| r.get(0))?;
        let (tx, rx) = mpsc::channel();
        let w = writer::Writer::new(sh.clone(), conn);
        let thread = std::thread::Builder::new().name("nrz-store".into()).spawn(move || w.run(rx)).context("Schreib-Thread")?;
        let cat = sh.catalog.read().clone();
        tracing::info!(
            "Speicher: {} Archivdateien, {} offene Abschnitte, {} Ereignisse",
            cat.files.len(),
            cat.hot.len(),
            sh.events_count.load(Ordering::Relaxed)
        );
        Ok(Arc::new(Self {
            sh,
            tx: Mutex::new(Some(tx)),
            thread: Mutex::new(Some(thread)),
            next_event: AtomicU64::new(next.unwrap_or(0) + 1),
        }))
    }

    fn send(&self, msg: Msg) -> bool {
        self.tx.lock().as_ref().is_some_and(|tx| tx.send(msg).is_ok())
    }

    /// Nachricht schicken und auf die Antwort des Schreib-Threads warten (ohne den Async-Thread zu blockieren).
    async fn roundtrip(&self, make: impl FnOnce(mpsc::SyncSender<()>) -> Msg, timeout: Duration) {
        let (reply, done) = mpsc::sync_channel(1);
        if !self.send(make(reply)) {
            return;
        }
        let _ = tokio::task::spawn_blocking(move || done.recv_timeout(timeout)).await;
    }

    /// Wird für **jeden** Takt jedes Sammlers gerufen (mehrmals je Sekunde, aus verschiedenen
    /// Threads). Muss sofort zurückkehren: nur puffern, nie auf den Datenträger warten.
    /// Der Ringpuffer ist zu diesem Zeitpunkt schon geschrieben.
    pub fn ingest(&self, frame: &Frame) {
        let rows: Vec<(u32, i64, f32)> = frame.samples_iter().map(|(s, v)| (s, frame.ts, v as f32)).collect();
        if rows.is_empty() {
            return;
        }
        self.sh.samples.fetch_add(rows.len() as u64, Ordering::Relaxed);
        self.send(Msg::Samples(rows));
    }

    /// Seit dem Start angenommene Werte.
    pub fn samples_total(&self) -> u64 {
        self.sh.samples.load(Ordering::Relaxed)
    }

    pub async fn query(&self, req: &QueryReq) -> anyhow::Result<QueryResp> {
        let now = self.sh.clock.now();
        let (after, before) = resolve(now, req.after, req.before)?;
        let points = if req.points == 0 { 300 } else { req.points.clamp(2, 5000) };
        let metas: Vec<_> = req.charts.iter().filter_map(|id| self.sh.registry.get(id)).collect();
        // Diagramme mit gleichem Takt gemeinsam lesen: eine Abfrage für alle ihre Reihen.
        let mut groups: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for m in &metas {
            groups.entry(m.interval.max(1)).or_default().extend(m.dims.iter().map(|d| d.series));
        }
        let sh = self.sh.clone();
        let (plans, data) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let mut plans = BTreeMap::new();
            let mut data = std::collections::HashMap::new();
            for (own, series) in groups {
                let plan = read::plan(&sh, now, after, before, points, own);
                let got = if plan.tier == shared::query::TIER_RAM {
                    // Nur Ringpuffer: keine Verbindung nötig.
                    read::read_ring_only(&sh, &series, &plan)
                } else {
                    sh.with_reader(|c| read::read(&sh, c, &series, &plan, now))?
                };
                data.extend(got);
                plans.insert(own, plan);
            }
            Ok((plans, data))
        })
        .await??;
        let mut charts = Vec::with_capacity(metas.len());
        for meta in metas {
            let plan: &Plan = &plans[&meta.interval.max(1)];
            let mut c = ChartData {
                id: meta.id.clone(),
                dims: meta.dims.iter().map(|d| d.id.clone()).collect(),
                start: plan.start,
                step: plan.step,
                tier: plan.tier,
                ..ChartData::default()
            };
            for d in &meta.dims {
                let accs = data.get(&d.series).map_or(&[][..], Vec::as_slice);
                c.values.push(accs.iter().map(|a| a.value(req.agg)).collect());
                if req.envelope {
                    c.min.push(accs.iter().map(Acc::lo).collect());
                    c.max.push(accs.iter().map(Acc::hi).collect());
                }
            }
            charts.push(c);
        }
        Ok(QueryResp { now, charts })
    }

    /// Rohe Teilsummen für Auswertungen: Reihen (beliebiger Takt) auf ein gemeinsames Raster.
    pub(crate) async fn grid(&self, series: Vec<u32>, after: i64, before: i64, points: u32) -> anyhow::Result<(Plan, std::collections::HashMap<u32, Vec<Acc>>)> {
        let now = self.sh.clock.now();
        let sh = self.sh.clone();
        tokio::task::spawn_blocking(move || {
            let own = series.iter().map(|s| sh.registry.series_interval(*s)).max().unwrap_or(1);
            let plan = read::plan(&sh, now, after, before, points, own);
            let data =
                if plan.tier == shared::query::TIER_RAM { read::read_ring_only(&sh, &series, &plan) } else { sh.with_reader(|c| read::read(&sh, c, &series, &plan, now))? };
            Ok((plan, data))
        })
        .await?
    }

    pub fn now(&self) -> i64 {
        self.sh.clock.now()
    }

    /// Beim Beenden: alles Gepufferte schreiben.
    pub async fn shutdown(&self) {
        let (reply, done) = mpsc::sync_channel(1);
        let sent = self.send(Msg::Shutdown(reply));
        self.tx.lock().take();
        if sent {
            let _ = tokio::task::spawn_blocking(move || done.recv_timeout(Duration::from_secs(20))).await;
        }
        let thread = self.thread.lock().take();
        if let Some(t) = thread {
            let _ = tokio::task::spawn_blocking(move || t.join()).await;
        }
    }

    /// Puffer schreiben und Wartung (Abschliessen, Aufbewahrung) sofort ausführen.
    pub async fn maintain(&self) {
        self.roundtrip(Msg::Maintain, Duration::from_secs(120)).await;
    }

    /// Ereignis aufnehmen: vergibt die Nummer und – wenn sie fehlt – die Zeit.
    pub fn add_event(&self, mut ev: Event) -> Event {
        ev.id = self.next_event.fetch_add(1, Ordering::Relaxed);
        if ev.ts == 0 {
            ev.ts = self.sh.clock.now_ms();
        }
        self.sh.pending_events.lock().push(ev.clone());
        if !self.send(Msg::Event(ev.clone())) {
            self.sh.pending_events.lock().retain(|e| e.id != ev.id);
        }
        ev
    }

    pub async fn events(&self, q: &EventQuery) -> anyhow::Result<EventPage> {
        let sh = self.sh.clone();
        let q = q.clone();
        tokio::task::spawn_blocking(move || {
            let pending = sh.pending_events.lock().clone();
            sh.with_reader(|c| events::query(c, &q, &pending))
        })
        .await?
    }

    pub fn settings(&self) -> StorageSettings {
        self.sh.settings.read().clone()
    }

    /// Neue Einstellungen gelten sofort (Aufbewahrung wird gleich geprüft).
    pub fn set_settings(&self, s: StorageSettings) {
        *self.sh.settings.write() = s.sanitized();
        self.send(Msg::Settings);
    }

    /// Belegung je Stufe und Kennzahlen des Speichers.
    pub async fn stats(&self) -> anyhow::Result<StorageStats> {
        let sh = self.sh.clone();
        tokio::task::spawn_blocking(move || {
            let cat = sh.catalog.read().clone();
            let settings = sh.settings.read().clone();
            let hot_rows: BTreeMap<(usize, i64), u64> = sh.with_reader(|c| {
                let mut out = BTreeMap::new();
                for key in &cat.hot {
                    let n: i64 = c.query_row(&format!("SELECT count(*) FROM {}", table(key.0, key.1)), [], |r| r.get(0)).unwrap_or(0);
                    out.insert(*key, n as u64);
                }
                Ok(out)
            })?;
            let now = sh.clock.now();
            let tiers = (0..TIERS)
                .map(|t| {
                    let files: Vec<_> = cat.files.range((t, i64::MIN)..=(t, i64::MAX)).collect();
                    let rows: u64 = files.iter().map(|(_, f)| f.rows).sum();
                    let bytes: u64 = files.iter().map(|(_, f)| f.bytes).sum();
                    let hot: Vec<_> = hot_rows.iter().filter(|((tt, _), n)| *tt == t && **n > 0).collect();
                    let oldest = files.first().map(|((_, s), _)| *s).into_iter().chain(hot.iter().map(|((_, s), _)| *s)).min().unwrap_or(0);
                    let newest = if hot.is_empty() { files.last().map_or(0, |((_, s), _)| (s + SECTION[t]).min(now)) } else { now };
                    TierStats {
                        tier: t as u8,
                        resolution: TIER_RES[t],
                        retention: settings.retention[t],
                        rows,
                        bytes,
                        files: files.len() as u64,
                        hot_rows: hot.iter().map(|(_, n)| **n).sum(),
                        oldest,
                        newest,
                        bytes_per_value: if rows > 0 { bytes as f64 / rows as f64 } else { 0.0 },
                    }
                })
                .collect::<Vec<_>>();
            let db_bytes = sh.db_bytes();
            let files_bytes: u64 = tiers.iter().map(|t| t.bytes).sum();
            let (charts, _) = sh.registry.counts();
            let samples_per_sec = {
                let r = sh.rate.lock();
                match (r.front(), r.back()) {
                    (Some(a), Some(b)) if b.0 > a.0 => (b.1 - a.1) as f64 / (b.0 - a.0) as f64,
                    _ => 0.0,
                }
            };
            Ok(StorageStats {
                ts: now,
                tiers,
                db_bytes,
                total_bytes: db_bytes + files_bytes,
                max_bytes: settings.max_bytes,
                ring_bytes: sh.ring.bytes() as u64,
                ring_window: sh.ring.window(),
                series: sh.registry.series_capacity().saturating_sub(1) as u64,
                charts: charts as u64,
                samples_per_sec: (samples_per_sec * 10.0).round() / 10.0,
                samples_total: sh.samples.load(Ordering::Relaxed),
                dropped: sh.dropped.load(Ordering::Relaxed),
                events: sh.events_count.load(Ordering::Relaxed),
                disk_free: disk_free(&sh.dir),
            })
        })
        .await?
    }
}

/// `after`/`before` wie in [`QueryReq`] auflösen (`≤ 0` = relativ).
pub fn resolve(now: i64, after: i64, before: i64) -> anyhow::Result<(i64, i64)> {
    let before = if before <= 0 { now + before } else { before };
    let after = if after <= 0 { before + after } else { after };
    anyhow::ensure!(after < before, "Zeitraum ist leer");
    Ok((after, before))
}

/// Einstellungen beim Start direkt aus `settings.json` lesen (der Kern lädt sie erst nach dem Speicher).
fn read_settings(dir: &std::path::Path) -> StorageSettings {
    std::fs::read_to_string(dir.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("storage").cloned())
        .and_then(|v| serde_json::from_value::<StorageSettings>(v).ok())
        .unwrap_or_default()
        .sanitized()
}

fn disk_free(dir: &std::path::Path) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return 0 };
    // SAFETY: `path` ist nullterminiert, `st` ein gültiger Puffer, den statvfs nur füllt.
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(path.as_ptr(), &mut st) != 0 {
            return 0;
        }
        st.f_bavail as u64 * st.f_frsize as u64
    }
}
