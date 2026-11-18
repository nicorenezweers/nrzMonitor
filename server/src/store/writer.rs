//! Der Schreib-Thread: nimmt Werte und Ereignisse aus dem Kanal, schreibt gebündelt (Appender) in die
//! offenen Abschnitte, verdichtet abgeschlossene Stunden, schreibt Parquet-Dateien und räumt auf.
//!
//! Nur dieser Thread schreibt ins Archiv. Abfragen lesen über eigene Verbindungen (DuckDB trennt
//! Lesen und Schreiben selbst) und eine Kopie des [`Catalog`].

use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{Receiver, RecvTimeoutError, SyncSender},
    },
    time::{Duration, Instant},
};

use anyhow::Context;
use duckdb::{Connection, params};
use shared::{catalog::monitor::MONITOR_DISK, events::Event};

use super::{
    Shared,
    events::sev_num,
    layout::{FileInfo, GRACE, PARQUET_OPTS, SECTION, TIERS, file_path, parse_table, section_of, sql_str, table, tier_dir},
};
use crate::{frame::Frame, registry::ChartDef};

pub enum Msg {
    /// (Reihe, Zeit, Wert)
    Samples(Vec<(u32, i64, f32)>),
    Event(Event),
    /// Einstellungen geändert → Aufbewahrung gleich prüfen.
    Settings,
    /// Puffer schreiben und Wartung sofort ausführen (Tests, Einstellungen).
    Maintain(SyncSender<()>),
    /// Nur Puffer schreiben.
    Flush(SyncSender<()>),
    Shutdown(SyncSender<()>),
}

/// So oft werden die Puffer spätestens geschrieben.
const FLUSH_EVERY: Duration = Duration::from_secs(2);
const FLUSH_ROWS: usize = 200_000;

/// Hilfsfunktion in SQL: auf 5 gültige Stellen runden – Mittelwerte komprimieren dann viel besser.
const MACROS: &str = "CREATE OR REPLACE TEMP MACRO nrz_r(x) AS CASE WHEN x IS NULL OR x = 0 OR NOT isfinite(x) THEN x \
                      ELSE round(x, greatest(0, 4 - floor(log10(abs(x)))::INTEGER)) END;";

/// Tabellen anlegen, Zustand und Verzeichnis nach einem Neustart wiederherstellen.
pub fn prepare(sh: &Shared, conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS store_meta (k VARCHAR PRIMARY KEY, v BIGINT NOT NULL);
         CREATE TABLE IF NOT EXISTS events (id UBIGINT NOT NULL, ts BIGINT NOT NULL, source VARCHAR, kind VARCHAR,
             severity UTINYINT, subject VARCHAR, title VARCHAR, message VARCHAR, labels VARCHAR);",
    )
    .context("Tabellen des Speichers")?;
    conn.execute_batch(MACROS)?;
    let closed: Option<i64> = conn.query_row("SELECT max(v) FROM store_meta WHERE k = 'closed'", [], |r| r.get(0))?;
    let mut cat = sh.catalog.write();
    cat.closed = closed.unwrap_or(0);
    let mut stmt = conn.prepare("SELECT table_name FROM duckdb_tables() WHERE schema_name = 'main'")?;
    let names = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for name in names.flatten() {
        if let Some(key) = parse_table(&name) {
            cat.hot.insert(key);
        }
    }
    for tier in 0..TIERS {
        let dir = tier_dir(&sh.dir, tier);
        std::fs::create_dir_all(&dir).with_context(|| format!("Ordner {}", dir.display()))?;
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".tmp") {
                // Halb geschriebene Datei vom letzten Lauf: die Daten liegen noch in DuckDB.
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let Some(start) = name.strip_suffix(".parquet").and_then(|s| s.parse::<i64>().ok()) else { continue };
            let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let rows = conn.query_row(&format!("SELECT sum(num_rows)::BIGINT FROM parquet_file_metadata({})", sql_str(&path)), [], |r| {
                r.get::<_, Option<i64>>(0)
            });
            match rows {
                Ok(rows) => {
                    cat.files.insert((tier, start), FileInfo { bytes, rows: rows.unwrap_or(0) as u64 });
                }
                Err(e) => {
                    tracing::warn!("Beschädigte Archivdatei {} entfernt: {e}", path.display());
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
    let n: i64 = conn.query_row("SELECT count(*) FROM events", [], |r| r.get(0))?;
    sh.events_count.store(n as u64, Ordering::Relaxed);
    Ok(())
}

pub struct Writer {
    sh: Arc<Shared>,
    conn: Connection,
    /// Abschnitt (Stufe 0) → gepufferte Zeilen.
    buf: BTreeMap<i64, Vec<(u32, u32, f32)>>,
    buffered: usize,
    events: Vec<Event>,
    closed: i64,
    last_flush: Instant,
    next_maint: i64,
    next_retention: i64,
    next_event_cleanup: i64,
    next_forget: i64,
}

impl Writer {
    pub fn new(sh: Arc<Shared>, conn: Connection) -> Self {
        let closed = sh.catalog.read().closed;
        let now = sh.clock.now();
        Self {
            sh,
            conn,
            buf: BTreeMap::new(),
            buffered: 0,
            events: Vec::new(),
            closed,
            last_flush: Instant::now(),
            next_maint: 0,
            next_retention: 0,
            next_event_cleanup: 0,
            next_forget: now + 120,
        }
    }

    pub fn run(mut self, rx: Receiver<Msg>) {
        self.maintain(true);
        loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Msg::Samples(rows)) => self.push(rows),
                Ok(Msg::Event(ev)) => self.events.push(ev),
                Ok(Msg::Settings) => self.next_retention = 0,
                Ok(Msg::Maintain(reply)) => {
                    self.write_events();
                    self.flush();
                    self.maintain(true);
                    let _ = reply.send(());
                }
                Ok(Msg::Flush(reply)) => {
                    self.write_events();
                    self.flush();
                    let _ = reply.send(());
                }
                Ok(Msg::Shutdown(reply)) => {
                    self.write_events();
                    self.flush();
                    if let Err(e) = self.conn.execute_batch("CHECKPOINT;") {
                        tracing::warn!("DuckDB-Checkpoint beim Beenden: {e}");
                    }
                    let _ = reply.send(());
                    return;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.write_events();
                    self.flush();
                    return;
                }
            }
            if !self.events.is_empty() {
                self.write_events();
            }
            if self.buffered >= FLUSH_ROWS || (self.buffered > 0 && self.last_flush.elapsed() >= FLUSH_EVERY) {
                self.flush();
            }
            if self.sh.clock.now() >= self.next_maint {
                self.flush();
                self.maintain(false);
            }
        }
    }

    fn push(&mut self, rows: Vec<(u32, i64, f32)>) {
        let now = self.sh.clock.now();
        let mut dropped = 0;
        for (series, ts, v) in rows {
            if ts < self.closed || ts > now + 3600 || ts < 0 || ts > i64::from(u32::MAX) {
                dropped += 1;
                continue;
            }
            self.buf.entry(section_of(0, ts)).or_default().push((series, ts as u32, v));
            self.buffered += 1;
        }
        if dropped > 0 {
            self.sh.dropped.fetch_add(dropped, Ordering::Relaxed);
        }
    }

    fn ensure_table(&self, tier: usize, start: i64) -> duckdb::Result<()> {
        if self.sh.catalog.read().hot.contains(&(tier, start)) {
            return Ok(());
        }
        self.conn.execute_batch(&format!("CREATE TABLE IF NOT EXISTS {} ({});", table(tier, start), super::layout::columns(tier)))?;
        self.sh.catalog.write().hot.insert((tier, start));
        Ok(())
    }

    /// Gepufferte Werte in die offenen Abschnitte schreiben.
    fn flush(&mut self) {
        self.last_flush = Instant::now();
        if self.buffered == 0 {
            return;
        }
        for (start, rows) in std::mem::take(&mut self.buf) {
            let result = (|| -> duckdb::Result<()> {
                self.ensure_table(0, start)?;
                let mut app = self.conn.appender(&table(0, start))?;
                for (s, t, v) in &rows {
                    app.append_row(params![s, t, v])?;
                }
                app.flush()
            })();
            if let Err(e) = result {
                tracing::warn!("{} Werte nicht gespeichert: {e}", rows.len());
                self.sh.dropped.fetch_add(rows.len() as u64, Ordering::Relaxed);
            }
        }
        self.buffered = 0;
    }

    fn write_events(&mut self) {
        if self.events.is_empty() {
            return;
        }
        let events = std::mem::take(&mut self.events);
        let result = (|| -> duckdb::Result<()> {
            let mut app = self.conn.appender("events")?;
            for e in &events {
                let labels = if e.labels.is_empty() { String::new() } else { serde_json::to_string(&e.labels).unwrap_or_default() };
                app.append_row(params![e.id, e.ts, e.source, e.kind, sev_num(e.severity), e.subject, e.title, e.message, labels])?;
            }
            app.flush()
        })();
        match result {
            Ok(()) => {
                self.sh.events_count.fetch_add(events.len() as u64, Ordering::Relaxed);
            }
            Err(e) => tracing::warn!("{} Ereignisse nicht gespeichert: {e}", events.len()),
        }
        let ids: HashSet<u64> = events.iter().map(|e| e.id).collect();
        self.sh.pending_events.lock().retain(|e| !ids.contains(&e.id));
    }

    /// Abschnitte abschliessen, Aufbewahrung, Obergrenze, Kennzahlen.
    pub fn maintain(&mut self, force: bool) {
        let now = self.sh.clock.now();
        self.next_maint = now + 10;
        // 1. Stunden ohne weitere Werte: verdichten und als Datei ablegen.
        let raw: Vec<i64> = self.sh.catalog.read().hot.iter().filter(|(t, _)| *t == 0).map(|(_, s)| *s).collect();
        for start in raw {
            if start + SECTION[0] + GRACE > now {
                continue;
            }
            if let Err(e) = self.close_raw(start) {
                tracing::warn!("Abschnitt {start} nicht abgeschlossen: {e:#}");
            }
        }
        // Alles vor dieser Stunde ist jetzt endgültig.
        let bound = section_of(0, now - GRACE);
        if bound > self.closed {
            self.set_closed(bound);
        }
        // 2. Minuten- und Stundenabschnitte, die vollständig verdichtet sind.
        let done: Vec<(usize, i64)> =
            self.sh.catalog.read().hot.iter().filter(|(t, s)| *t > 0 && s + SECTION[*t] <= self.closed).copied().collect();
        for (tier, start) in done {
            if let Err(e) = self.export(tier, start) {
                tracing::warn!("Abschnitt {tier}/{start} nicht abgeschlossen: {e:#}");
            }
        }
        // 3. Kennzahlen für die Belegungsrate.
        {
            let mut r = self.sh.rate.lock();
            r.push_back((now, self.sh.samples.load(Ordering::Relaxed)));
            while r.len() > 7 {
                r.pop_front();
            }
        }
        if force || now >= self.next_retention {
            self.next_retention = now + 60;
            self.retention(now);
            self.record_disk(now);
        }
        if now >= self.next_event_cleanup {
            self.next_event_cleanup = now + 3600;
            let keep = self.sh.settings.read().event_retention as i64;
            let limit = self.sh.clock.now_ms() - keep * 1000;
            let result = self.conn.execute("DELETE FROM events WHERE ts < ?", params![limit]).and_then(|_| {
                self.conn.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
            });
            match result {
                Ok(n) => self.sh.events_count.store(n as u64, Ordering::Relaxed),
                Err(e) => tracing::warn!("Alte Ereignisse nicht gelöscht: {e}"),
            }
        }
        if now >= self.next_forget {
            self.next_forget = now + 86_400;
            if let Err(e) = self.forget_empty() {
                tracing::warn!("Stillgelegte Diagramme nicht geprüft: {e:#}");
            }
        }
    }

    fn set_closed(&mut self, ts: i64) {
        self.closed = ts;
        self.sh.catalog.write().closed = ts;
        if let Err(e) = self.conn.execute("INSERT OR REPLACE INTO store_meta VALUES ('closed', ?)", params![ts]) {
            tracing::warn!("Zustand nicht gespeichert: {e}");
        }
    }

    /// Eine Stunde Sekundenwerte: Minuten und Stunde berechnen, dann als Datei ablegen.
    fn close_raw(&mut self, start: i64) -> anyhow::Result<()> {
        let end = start + SECTION[0];
        if end > self.closed {
            let (m, h) = (section_of(1, start), section_of(2, start));
            self.ensure_table(1, m)?;
            self.ensure_table(2, h)?;
            let src = table(0, start);
            let sql = format!(
                "BEGIN;
                 INSERT INTO {t1} SELECT series, ts // 60 * 60, nrz_r(avg(v)), min(v), max(v), count(*) FROM {src} GROUP BY ALL;
                 INSERT INTO {t2} SELECT series, {start}, nrz_r(avg(v)), min(v), max(v), count(*) FROM {src} GROUP BY ALL;
                 INSERT OR REPLACE INTO store_meta VALUES ('closed', {end});
                 COMMIT;",
                t1 = table(1, m),
                t2 = table(2, h),
            );
            if let Err(e) = self.conn.execute_batch(&sql) {
                let _ = self.conn.execute_batch("ROLLBACK;");
                return Err(e.into());
            }
            self.closed = end;
            self.sh.catalog.write().closed = end;
        }
        self.export(0, start)
    }

    /// Offenen Abschnitt als Parquet-Datei ablegen (tmp + rename) und die Tabelle löschen.
    fn export(&mut self, tier: usize, start: i64) -> anyhow::Result<()> {
        let name = table(tier, start);
        let path = file_path(&self.sh.dir, tier, start);
        let exists = self.sh.catalog.read().files.contains_key(&(tier, start)) || path.exists();
        if !exists {
            let rows: i64 = self.conn.query_row(&format!("SELECT count(*) FROM {name}"), [], |r| r.get(0))?;
            if rows > 0 {
                let tmp = path.with_extension("parquet.tmp");
                self.conn
                    .execute_batch(&format!("COPY (SELECT * FROM {name} ORDER BY series, ts) TO {} ({PARQUET_OPTS});", sql_str(&tmp)))
                    .with_context(|| format!("Parquet {}", tmp.display()))?;
                if let Ok(f) = std::fs::File::open(&tmp) {
                    let _ = f.sync_all();
                }
                std::fs::rename(&tmp, &path)?;
                let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.sh.catalog.write().files.insert((tier, start), FileInfo { bytes, rows: rows as u64 });
            }
        }
        // Erst aus dem Verzeichnis nehmen, dann löschen: Abfragen sehen nie beides doppelt.
        self.sh.catalog.write().hot.remove(&(tier, start));
        self.conn.execute_batch(&format!("DROP TABLE IF EXISTS {name};"))?;
        Ok(())
    }

    fn remove_file(&self, tier: usize, start: i64) -> u64 {
        let info = self.sh.catalog.write().files.remove(&(tier, start));
        let _ = std::fs::remove_file(file_path(&self.sh.dir, tier, start));
        info.map_or(0, |i| i.bytes)
    }

    /// Aufbewahrung je Stufe und Obergrenze für den ganzen Speicher.
    fn retention(&mut self, now: i64) {
        let settings = self.sh.settings.read().clone();
        let expired: Vec<(usize, i64)> = self
            .sh
            .catalog
            .read()
            .files
            .keys()
            .filter(|(t, s)| s + SECTION[*t] <= now - settings.retention[*t] as i64)
            .copied()
            .collect();
        for (t, s) in expired {
            self.remove_file(t, s);
        }
        if settings.max_bytes == 0 {
            return;
        }
        let mut total = self.sh.db_bytes() + self.sh.catalog.read().files.values().map(|f| f.bytes).sum::<u64>();
        while total > settings.max_bytes {
            // Zuerst fällt das Älteste der feinsten Stufe.
            let victim = (0..TIERS).find_map(|t| self.sh.catalog.read().files.range((t, i64::MIN)..=(t, i64::MAX)).next().map(|(k, _)| *k));
            let Some((t, s)) = victim else { break };
            tracing::info!("Speichergrenze erreicht – lösche Stufe {t}, Abschnitt {s}");
            total = total.saturating_sub(self.remove_file(t, s));
        }
    }

    /// Diagramm `monitor.disk`: Belegung je Stufe.
    fn record_disk(&mut self, now: i64) {
        let handle = self.sh.registry.chart(ChartDef::tpl(&MONITOR_DISK));
        let cat = self.sh.catalog.read().clone();
        let ts = now.div_euclid(60) * 60;
        let mut frame = Frame::new(ts);
        let values = [self.sh.db_bytes() as f64, cat.tier_bytes(0) as f64, cat.tier_bytes(1) as f64, cat.tier_bytes(2) as f64, f64::NAN];
        frame.put(&handle, &values);
        self.sh.ring.write(&frame);
        let rows: Vec<(u32, i64, f32)> = frame.samples_iter().map(|(s, v)| (s, ts, v as f32)).collect();
        self.sh.samples.fetch_add(rows.len() as u64, Ordering::Relaxed);
        self.push(rows);
    }

    /// Stillgelegte Diagramme, zu denen es keine Daten mehr gibt, aus dem Katalog nehmen.
    pub fn forget_empty(&mut self) -> anyhow::Result<usize> {
        let retired: Vec<_> = self.sh.registry.list(true).into_iter().filter(|c| c.retired).collect();
        if retired.is_empty() {
            return Ok(0);
        }
        self.flush();
        let cat = self.sh.catalog.read().clone();
        let mut parts = Vec::new();
        for tier in 0..TIERS {
            let files: Vec<String> =
                cat.files.keys().filter(|(t, _)| *t == tier).map(|(t, s)| sql_str(&file_path(&self.sh.dir, *t, *s))).collect();
            if !files.is_empty() {
                parts.push(format!("SELECT DISTINCT series FROM read_parquet([{}])", files.join(", ")));
            }
        }
        for (t, s) in &cat.hot {
            parts.push(format!("SELECT DISTINCT series FROM {}", table(*t, *s)));
        }
        let mut present = HashSet::new();
        if !parts.is_empty() {
            let mut stmt = self.conn.prepare(&parts.join(" UNION "))?;
            for s in stmt.query_map([], |r| r.get::<_, u32>(0))?.flatten() {
                present.insert(s);
            }
        }
        let mut n = 0;
        for c in retired {
            let empty = c.dims.iter().all(|d| !present.contains(&d.series) && self.sh.ring.last_ts(d.series).is_none());
            if empty {
                self.sh.registry.forget(&c.id);
                n += 1;
            }
        }
        if n > 0 {
            tracing::info!("{n} stillgelegte Diagramme ohne Daten entfernt");
        }
        Ok(n)
    }
}
