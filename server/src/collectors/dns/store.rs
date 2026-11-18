//! Speicher des DNS-Abfrageprotokolls.
//!
//! - Frische Anfragen in der DuckDB-Tabelle `dns_log` (Appender, gebündelt).
//! - Abgeschlossene Tage (UTC) als Parquet mit ZSTD unter `<data>/dns/JJJJ-MM-TT.parquet`,
//!   sortiert nach Zeit – danach verschwinden sie aus der Tabelle. Löschen = Datei löschen.
//! - Der Merker `synced_until` (bis hierhin lückenlos abgeholt) steht in `dns_state` und wird in
//!   derselben Transaktion wie die Zeilen geschrieben: nach einem Absturz fehlt nichts und es
//!   kommt nichts doppelt.
//!
//! Alle Funktionen blockieren – aus async-Code über `spawn_blocking` rufen.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::Context;
use duckdb::{Connection, params_from_iter, types::Value as Sql};
use parking_lot::Mutex;
use shared::dns::{ClientRow, DomainInfo, DnsTop, QueryEntry, QueryFilter, QueryPage, Timeline, TimelinePoint, TopEntry};

const DAY_MS: i64 = 86_400_000;
const COLS: &str = "ts, id, client, proto, name, qtype, rcode, kind, answer, ms, upstream";

pub struct DnsStore {
    writer: Mutex<Connection>,
    reader: Mutex<Connection>,
    /// Ordner der Parquet-Dateien (`None` = nur im Arbeitsspeicher, Beispieldaten).
    dir: Option<PathBuf>,
    /// Gemessene Bytes je Anfrage (Parquet), zwischengespeichert.
    bpq: Mutex<Option<f64>>,
}

/// Belegung des Speichers.
#[derive(Debug, Clone, Default)]
pub struct StoreStats {
    pub rows: u64,
    pub bytes: u64,
    pub bytes_per_query: f64,
    pub oldest: i64,
}

impl DnsStore {
    /// Auf der gemeinsamen Datenbank (`core.db`) mit Parquet-Ordner `dir`.
    pub fn open(conn: Connection, dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&dir).with_context(|| format!("Ordner {}", dir.display()))?;
        Self::init(conn, Some(dir))
    }

    /// Nur im Arbeitsspeicher (Beispieldaten, Tests).
    pub fn memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("SET memory_limit='128MB'; SET threads=1;")?;
        Self::init(conn, None)
    }

    fn init(conn: Connection, dir: Option<PathBuf>) -> anyhow::Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS dns_log (
                ts BIGINT, id BIGINT, client VARCHAR, proto VARCHAR, name VARCHAR, qtype VARCHAR,
                rcode VARCHAR, kind VARCHAR, answer VARCHAR, ms UINTEGER, upstream VARCHAR);
             CREATE TABLE IF NOT EXISTS dns_state (k VARCHAR PRIMARY KEY, v BIGINT);",
        )
        .context("DNS-Tabellen anlegen")?;
        let reader = conn.try_clone()?;
        Ok(Self { writer: Mutex::new(conn), reader: Mutex::new(reader), dir, bpq: Mutex::new(None) })
    }

    pub fn is_memory(&self) -> bool {
        self.dir.is_none()
    }

    // --- Merker -----------------------------------------------------------------------------

    pub fn state(&self, key: &str) -> anyhow::Result<Option<i64>> {
        let conn = self.reader.lock();
        let mut stmt = conn.prepare("SELECT v FROM dns_state WHERE k = ?")?;
        let mut rows = stmt.query([key])?;
        Ok(match rows.next()? {
            Some(r) => Some(r.get(0)?),
            None => None,
        })
    }

    pub fn set_state(&self, key: &str, v: i64) -> anyhow::Result<()> {
        self.writer.lock().execute("INSERT OR REPLACE INTO dns_state VALUES (?, ?)", duckdb::params![key, v])?;
        Ok(())
    }

    /// Nummern und Zeiten der Anfragen ab `since_ms` (zum Erkennen von Doppelten nach einem Neustart).
    pub fn ids_since(&self, since_ms: i64) -> anyhow::Result<Vec<(i64, i64)>> {
        let conn = self.reader.lock();
        let mut stmt = conn.prepare("SELECT id, ts FROM dns_log WHERE ts >= ?")?;
        let rows = stmt.query_map([since_ms], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.flatten().collect())
    }

    /// Zeilen anhängen und den Merker setzen – in einer Transaktion.
    pub fn insert(&self, rows: &[QueryEntry], synced_until: Option<i64>) -> anyhow::Result<()> {
        let conn = self.writer.lock();
        conn.execute_batch("BEGIN TRANSACTION")?;
        let result = (|| -> anyhow::Result<()> {
            if !rows.is_empty() {
                let mut app = conn.appender("dns_log")?;
                for e in rows {
                    app.append_row(duckdb::params![
                        e.ts, e.id, e.client, e.proto, e.name, e.qtype, e.rcode, e.kind, e.answer, e.ms, e.upstream
                    ])?;
                }
                app.flush()?;
            }
            if let Some(s) = synced_until {
                conn.execute("INSERT OR REPLACE INTO dns_state VALUES ('synced_until', ?)", [s])?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // --- Parquet und Aufbewahrung -------------------------------------------------------------

    /// Parquet-Dateien mit ihrem Tag (Tage seit 1970), sortiert.
    fn files(&self) -> Vec<(i64, PathBuf)> {
        let Some(dir) = &self.dir else { return Vec::new() };
        let mut out: Vec<(i64, PathBuf)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let name = p.file_name()?.to_str()?;
                if !name.ends_with(".parquet") {
                    return None;
                }
                Some((parse_day(name.get(..10)?)?, p))
            })
            .collect();
        out.sort();
        out
    }

    /// Tage vor `cutoff_ms` (abgeschlossen) als Parquet ablegen und aus der Tabelle löschen.
    /// Liefert die Zahl der archivierten Zeilen.
    pub fn archive(&self, cutoff_ms: i64) -> anyhow::Result<u64> {
        let Some(dir) = self.dir.clone() else { return Ok(0) };
        let cutoff_day = cutoff_ms.div_euclid(DAY_MS);
        let days: Vec<i64> = {
            let conn = self.writer.lock();
            let mut stmt = conn.prepare("SELECT DISTINCT ts // 86400000 FROM dns_log WHERE ts < ? ORDER BY 1")?;
            let rows = stmt.query_map([cutoff_day * DAY_MS], |r| r.get(0))?;
            rows.flatten().collect()
        };
        let mut moved = 0u64;
        for day in days {
            let existing: Vec<PathBuf> = self.files().into_iter().filter(|(d, _)| *d == day).map(|(_, p)| p).collect();
            let name = if existing.is_empty() {
                format!("{}.parquet", day_string(day))
            } else {
                format!("{}-{}.parquet", day_string(day), existing.len())
            };
            let path = dir.join(&name);
            let tmp = dir.join(format!("{name}.tmp"));
            let (a, b) = (day * DAY_MS, (day + 1) * DAY_MS);
            // Nach einem Absturz zwischen Umbenennen und Löschen: schon Archiviertes nicht doppelt.
            let skip = if existing.is_empty() {
                String::new()
            } else {
                format!(" AND id NOT IN (SELECT id FROM read_parquet({}))", file_list(&existing))
            };
            let conn = self.writer.lock();
            let n: i64 = conn.query_row(
                &format!("SELECT count(*) FROM dns_log WHERE ts >= {a} AND ts < {b}{skip}"),
                [],
                |r| r.get(0),
            )?;
            if n > 0 {
                conn.execute_batch(&format!(
                    "COPY (SELECT {COLS} FROM dns_log WHERE ts >= {a} AND ts < {b}{skip} ORDER BY ts, id)
                     TO {} (FORMAT parquet, COMPRESSION zstd, COMPRESSION_LEVEL 9, ROW_GROUP_SIZE 1000000);",
                    sql_str(&tmp)
                ))?;
                std::fs::rename(&tmp, &path)?;
            }
            conn.execute(&format!("DELETE FROM dns_log WHERE ts >= {a} AND ts < {b}"), [])?;
            moved += n as u64;
            *self.bpq.lock() = None;
        }
        Ok(moved)
    }

    /// Älteres als `days` Tage löschen (Dateien und Tabelle).
    pub fn retain(&self, days: u32, now_ms: i64) -> anyhow::Result<()> {
        let limit = now_ms - i64::from(days.max(1)) * DAY_MS;
        let limit_day = limit.div_euclid(DAY_MS);
        for (day, path) in self.files() {
            if day < limit_day {
                let _ = std::fs::remove_file(path);
                *self.bpq.lock() = None;
            }
        }
        self.writer.lock().execute("DELETE FROM dns_log WHERE ts < ?", [limit])?;
        Ok(())
    }

    /// Zeilen, Bytes und Bytes je Anfrage. Ohne archivierte Tage wird die Tabelle probehalber
    /// als Parquet geschrieben und gemessen (einmal, dann zwischengespeichert).
    pub fn stats(&self) -> anyhow::Result<StoreStats> {
        let files = self.files();
        let conn = self.reader.lock();
        let (table_rows, table_oldest): (i64, Option<i64>) =
            conn.query_row("SELECT count(*), min(ts) FROM dns_log", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let mut file_rows = 0i64;
        let mut file_bytes = 0u64;
        let mut oldest = table_oldest.unwrap_or(0);
        if !files.is_empty() {
            let paths: Vec<PathBuf> = files.iter().map(|(_, p)| p.clone()).collect();
            file_rows = conn.query_row(
                &format!("SELECT coalesce(sum(num_rows), 0)::BIGINT FROM parquet_file_metadata({})", file_list(&paths)),
                [],
                |r| r.get(0),
            )?;
            file_bytes = paths.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
            oldest = files[0].0 * DAY_MS;
        }
        let mut bpq = *self.bpq.lock();
        if bpq.is_none() {
            bpq = if file_rows > 0 {
                Some(file_bytes as f64 / file_rows as f64)
            } else if table_rows >= 1000 {
                self.probe_bytes(&conn).ok()
            } else {
                None
            };
            *self.bpq.lock() = bpq;
        }
        let bpq = bpq.unwrap_or(0.0);
        Ok(StoreStats {
            rows: (table_rows + file_rows) as u64,
            bytes: file_bytes + (table_rows as f64 * bpq) as u64,
            bytes_per_query: (bpq * 10.0).round() / 10.0,
            oldest,
        })
    }

    fn probe_bytes(&self, conn: &Connection) -> anyhow::Result<f64> {
        let dir = self.dir.clone().unwrap_or_else(std::env::temp_dir);
        let tmp = dir.join(format!("probe-{}.parquet.tmp", std::process::id()));
        let n: i64 = conn.query_row("SELECT count(*) FROM dns_log", [], |r| r.get(0))?;
        conn.execute_batch(&format!(
            "COPY (SELECT {COLS} FROM dns_log ORDER BY ts, id) TO {} (FORMAT parquet, COMPRESSION zstd, COMPRESSION_LEVEL 9, ROW_GROUP_SIZE 1000000);",
            sql_str(&tmp)
        ))?;
        let bytes = std::fs::metadata(&tmp)?.len();
        let _ = std::fs::remove_file(&tmp);
        Ok(bytes as f64 / n.max(1) as f64)
    }

    // --- Abfragen ----------------------------------------------------------------------------

    /// Quelle für Abfragen: Tabelle plus die Parquet-Dateien, die den Zeitraum berühren.
    fn source(&self, from_ms: Option<i64>, to_ms: Option<i64>) -> String {
        let a = from_ms.map(|v| v.div_euclid(DAY_MS));
        let b = to_ms.map(|v| (v - 1).div_euclid(DAY_MS));
        let paths: Vec<PathBuf> = self
            .files()
            .into_iter()
            .filter(|(d, _)| a.is_none_or(|a| *d >= a) && b.is_none_or(|b| *d <= b))
            .map(|(_, p)| p)
            .collect();
        if paths.is_empty() {
            "dns_log".to_string()
        } else {
            format!("(SELECT {COLS} FROM dns_log UNION ALL SELECT {COLS} FROM read_parquet({}))", file_list(&paths))
        }
    }

    pub fn page(&self, f: &QueryFilter) -> anyhow::Result<QueryPage> {
        let per_page = if f.per_page == 0 { 100 } else { f.per_page.clamp(1, 500) };
        let page = f.page.max(1);
        let (wh, params) = filter_sql(f);
        let src = self.source(f.from.map(|v| v * 1000), f.to.map(|v| v * 1000));
        let conn = self.reader.lock();
        let total: i64 = conn.query_row(&format!("SELECT count(*) FROM {src} WHERE {wh}"), params_from_iter(params.iter()), |r| r.get(0))?;
        let sql = format!(
            "SELECT {COLS} FROM {src} WHERE {wh} ORDER BY ts DESC, id DESC LIMIT {per_page} OFFSET {}",
            u64::from(page - 1) * u64::from(per_page)
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), row_entry)?;
        Ok(QueryPage { entries: rows.collect::<Result<_, _>>()?, total: total as u64, page, per_page })
    }

    pub fn top(&self, f: &QueryFilter, limit: u32) -> anyhow::Result<DnsTop> {
        let limit = limit.clamp(1, 100);
        let (wh, params) = filter_sql(f);
        let src = self.source(f.from.map(|v| v * 1000), f.to.map(|v| v * 1000));
        let conn = self.reader.lock();
        let mut top = conn.query_row(
            &format!(
                "SELECT count(*), count(*) FILTER (kind = 'blocked'), count(*) FILTER (kind = 'cached'),
                        count(*) FILTER (kind IN ('failed', 'dropped') OR rcode = 'SERVFAIL'),
                        count(DISTINCT client), count(DISTINCT name), coalesce(avg(ms), 0)
                 FROM {src} WHERE {wh}"
            ),
            params_from_iter(params.iter()),
            |r| {
                Ok(DnsTop {
                    total: r.get::<_, i64>(0)? as u64,
                    blocked: r.get::<_, i64>(1)? as u64,
                    cached: r.get::<_, i64>(2)? as u64,
                    failed: r.get::<_, i64>(3)? as u64,
                    clients_count: r.get::<_, i64>(4)? as u64,
                    domains_count: r.get::<_, i64>(5)? as u64,
                    avg_ms: (r.get::<_, f64>(6)? * 100.0).round() / 100.0,
                    ..DnsTop::default()
                })
            },
        )?;
        let group = |col: &str, extra: &str| -> anyhow::Result<Vec<TopEntry>> {
            group_by(&conn, &src, &wh, &params, col, extra, limit)
        };
        top.clients = group("client", "")?;
        top.domains = group("name", "")?;
        top.blocked_domains = group("name", "kind = 'blocked'")?;
        top.qtypes = group("qtype", "")?;
        top.upstreams = group("upstream", "upstream <> ''")?;
        top.kinds = group("kind", "")?;
        top.rcodes = group("rcode", "")?;
        Ok(top)
    }

    pub fn timeline(&self, f: &QueryFilter, points: u32) -> anyhow::Result<Timeline> {
        let to = f.to.unwrap_or_else(crate::util::now);
        let from = f.from.unwrap_or(to - 86_400);
        let step = nice_step((to - from).max(60), points.clamp(10, 1000));
        let mut f = f.clone();
        f.from = Some(from);
        f.to = Some(to);
        let (wh, params) = filter_sql(&f);
        let src = self.source(Some(from * 1000), Some(to * 1000));
        let conn = self.reader.lock();
        let sql = format!(
            "SELECT (ts // {ms}) * {step} AS b, count(*), count(*) FILTER (kind = 'blocked'), count(*) FILTER (kind = 'cached'),
                    count(*) FILTER (kind = 'forwarded'), count(*) FILTER (kind IN ('failed', 'dropped'))
             FROM {src} WHERE {wh} GROUP BY b ORDER BY b",
            ms = step * 1000
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |r| {
            Ok(TimelinePoint {
                ts: r.get(0)?,
                total: r.get::<_, i64>(1)? as u64,
                blocked: r.get::<_, i64>(2)? as u64,
                cached: r.get::<_, i64>(3)? as u64,
                forwarded: r.get::<_, i64>(4)? as u64,
                failed: r.get::<_, i64>(5)? as u64,
            })
        })?;
        let found: BTreeMap<i64, TimelinePoint> = rows.flatten().map(|p| (p.ts, p)).collect();
        // Lücken mit Nullen füllen, damit das Diagramm gleichmässig ist.
        let mut points_out = Vec::new();
        let mut b = from.div_euclid(step) * step;
        while b < to {
            points_out.push(found.get(&b).cloned().unwrap_or(TimelinePoint { ts: b, ..TimelinePoint::default() }));
            b += step;
        }
        Ok(Timeline { step: step as u32, points: points_out })
    }

    pub fn clients(&self, f: &QueryFilter, limit: u32) -> anyhow::Result<Vec<ClientRow>> {
        let (wh, params) = filter_sql(f);
        let src = self.source(f.from.map(|v| v * 1000), f.to.map(|v| v * 1000));
        let conn = self.reader.lock();
        let sql = format!(
            "SELECT client, count(*), count(*) FILTER (kind = 'blocked'), count(DISTINCT name), max(ts), coalesce(avg(ms), 0)
             FROM {src} WHERE {wh} GROUP BY client ORDER BY 2 DESC, 1 LIMIT {}",
            limit.clamp(1, 1000)
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |r| {
            Ok(ClientRow {
                client: r.get(0)?,
                queries: r.get::<_, i64>(1)? as u64,
                blocked: r.get::<_, i64>(2)? as u64,
                domains: r.get::<_, i64>(3)? as u64,
                last: r.get(4)?,
                avg_ms: (r.get::<_, f64>(5)? * 100.0).round() / 100.0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn domain(&self, name: &str, f: &QueryFilter) -> anyhow::Result<DomainInfo> {
        let name = name.trim().trim_end_matches('.').to_ascii_lowercase();
        let mut f = f.clone();
        f.name.clear();
        let (wh, mut params) = filter_sql(&f);
        let wh = format!("{wh} AND name = ?");
        params.push(Sql::Text(name.clone()));
        let src = self.source(f.from.map(|v| v * 1000), f.to.map(|v| v * 1000));
        let mut info = {
            let conn = self.reader.lock();
            let mut info = conn.query_row(
                &format!(
                    "SELECT count(*), count(*) FILTER (kind = 'blocked'), coalesce(min(ts), 0), coalesce(max(ts), 0), coalesce(avg(ms), 0)
                     FROM {src} WHERE {wh}"
                ),
                params_from_iter(params.iter()),
                |r| {
                    Ok(DomainInfo {
                        name: name.clone(),
                        queries: r.get::<_, i64>(0)? as u64,
                        blocked: r.get::<_, i64>(1)? as u64,
                        first: r.get(2)?,
                        last: r.get(3)?,
                        avg_ms: (r.get::<_, f64>(4)? * 100.0).round() / 100.0,
                        ..DomainInfo::default()
                    })
                },
            )?;
            info.clients = group_by(&conn, &src, &wh, &params, "client", "", 20)?;
            info.qtypes = group_by(&conn, &src, &wh, &params, "qtype", "", 10)?;
            info.kinds = group_by(&conn, &src, &wh, &params, "kind", "", 10)?;
            info.rcodes = group_by(&conn, &src, &wh, &params, "rcode", "", 10)?;
            info.answers = group_by(&conn, &src, &wh, &params, "answer", "answer <> ''", 10)?;
            info
        };
        let mut tf = f.clone();
        tf.name.clear();
        info.timeline = self.timeline_exact(&tf, &name)?;
        Ok(info)
    }

    fn timeline_exact(&self, f: &QueryFilter, name: &str) -> anyhow::Result<Timeline> {
        // Verlauf nur für diesen Namen: Filter `name` ist ein Teilstring – darum exakt nachfiltern.
        let to = f.to.unwrap_or_else(crate::util::now);
        let from = f.from.unwrap_or(to - 86_400);
        let step = nice_step((to - from).max(60), 120);
        let mut f = f.clone();
        f.from = Some(from);
        f.to = Some(to);
        let (wh, mut params) = filter_sql(&f);
        params.push(Sql::Text(name.to_string()));
        let src = self.source(Some(from * 1000), Some(to * 1000));
        let conn = self.reader.lock();
        let sql = format!(
            "SELECT (ts // {ms}) * {step} AS b, count(*), count(*) FILTER (kind = 'blocked'), count(*) FILTER (kind = 'cached'),
                    count(*) FILTER (kind = 'forwarded'), count(*) FILTER (kind IN ('failed', 'dropped'))
             FROM {src} WHERE {wh} AND name = ? GROUP BY b ORDER BY b",
            ms = step * 1000
        );
        let mut stmt = conn.prepare(&sql)?;
        let found: BTreeMap<i64, TimelinePoint> = stmt
            .query_map(params_from_iter(params.iter()), |r| {
                Ok(TimelinePoint {
                    ts: r.get(0)?,
                    total: r.get::<_, i64>(1)? as u64,
                    blocked: r.get::<_, i64>(2)? as u64,
                    cached: r.get::<_, i64>(3)? as u64,
                    forwarded: r.get::<_, i64>(4)? as u64,
                    failed: r.get::<_, i64>(5)? as u64,
                })
            })?
            .flatten()
            .map(|p| (p.ts, p))
            .collect();
        let mut points = Vec::new();
        let mut b = from.div_euclid(step) * step;
        while b < to {
            points.push(found.get(&b).cloned().unwrap_or(TimelinePoint { ts: b, ..TimelinePoint::default() }));
            b += step;
        }
        Ok(Timeline { step: step as u32, points })
    }
}

fn row_entry(r: &duckdb::Row<'_>) -> duckdb::Result<QueryEntry> {
    Ok(QueryEntry {
        ts: r.get(0)?,
        id: r.get(1)?,
        client: r.get(2)?,
        proto: r.get(3)?,
        name: r.get(4)?,
        qtype: r.get(5)?,
        rcode: r.get(6)?,
        kind: r.get(7)?,
        answer: r.get(8)?,
        ms: r.get(9)?,
        upstream: r.get(10)?,
    })
}

fn group_by(
    conn: &Connection,
    src: &str,
    wh: &str,
    params: &[Sql],
    col: &str,
    extra: &str,
    limit: u32,
) -> anyhow::Result<Vec<TopEntry>> {
    let extra = if extra.is_empty() { String::new() } else { format!(" AND {extra}") };
    let sql = format!(
        "SELECT {col}, count(*), count(*) FILTER (kind = 'blocked') FROM {src} WHERE {wh}{extra}
         GROUP BY {col} ORDER BY 2 DESC, 1 LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |r| {
        Ok(TopEntry { name: r.get(0)?, count: r.get::<_, i64>(1)? as u64, blocked: r.get::<_, i64>(2)? as u64 })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// `WHERE`-Teil und Parameter aus dem Filter.
fn filter_sql(f: &QueryFilter) -> (String, Vec<Sql>) {
    let mut wh = vec!["1 = 1".to_string()];
    let mut p = Vec::new();
    if let Some(from) = f.from {
        wh.push("ts >= ?".into());
        p.push(Sql::BigInt(from.saturating_mul(1000)));
    }
    if let Some(to) = f.to {
        wh.push("ts < ?".into());
        p.push(Sql::BigInt(to.saturating_mul(1000)));
    }
    let mut exact = |col: &str, v: &str, upper: bool| {
        let v = v.trim();
        if !v.is_empty() {
            wh.push(format!("{col} = ?"));
            p.push(Sql::Text(if upper { v.to_ascii_uppercase() } else { v.to_string() }));
        }
    };
    exact("client", &f.client, false);
    exact("qtype", &f.qtype, true);
    exact("kind", &f.kind.to_ascii_lowercase(), false);
    exact("rcode", &f.rcode, true);
    exact("upstream", &f.upstream, false);
    let name = f.name.trim().trim_end_matches('.').to_ascii_lowercase();
    if !name.is_empty() {
        wh.push("contains(name, ?)".into());
        p.push(Sql::Text(name));
    }
    (wh.join(" AND "), p)
}

/// Abschnittslänge, sodass höchstens `points` Abschnitte entstehen (runde Werte).
fn nice_step(span: i64, points: u32) -> i64 {
    const STEPS: &[i64] = &[10, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 10_800, 21_600, 43_200, 86_400];
    let min = span / i64::from(points.max(1));
    STEPS.iter().copied().find(|s| *s >= min).unwrap_or(86_400)
}

fn sql_str(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "''"))
}

fn file_list(paths: &[PathBuf]) -> String {
    format!("[{}]", paths.iter().map(|p| sql_str(p)).collect::<Vec<_>>().join(", "))
}

/// Tage seit 1970 → `JJJJ-MM-TT` (UTC).
pub fn day_string(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn parse_day(s: &str) -> Option<i64> {
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    (s.as_bytes().get(4) == Some(&b'-') && s.as_bytes().get(7) == Some(&b'-')).then(|| days_from_civil(y, m, d))
}

/// RFC 3339 (`2026-11-15T10:23:00Z`, `…+01:00`, mit Bruchteilen) → Unix-Sekunden.
pub fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let day = parse_day(s.get(..10)?)?;
    if s.len() == 10 {
        return Some(day * 86_400);
    }
    let t = s.get(11..)?;
    let h: i64 = t.get(0..2)?.parse().ok()?;
    let m: i64 = t.get(3..5)?.parse().ok()?;
    let sec: i64 = t.get(6..8).and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut rest = t.get(8..).unwrap_or("");
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        r if r.len() >= 6 && (r.starts_with('+') || r.starts_with('-')) => {
            let oh: i64 = r.get(1..3)?.parse().ok()?;
            let om: i64 = r.get(4..6)?.parse().ok()?;
            let o = oh * 3600 + om * 60;
            if r.starts_with('-') { -o } else { o }
        }
        _ => return None,
    };
    Some(day * 86_400 + h * 3600 + m * 60 + sec - offset)
}

/// Unix-Sekunden → `2026-11-15T10:23:00Z`.
pub fn format_time(ts: i64) -> String {
    let day = ts.div_euclid(86_400);
    let s = ts.rem_euclid(86_400);
    format!("{}T{:02}:{:02}:{:02}Z", day_string(day), s / 3600, (s / 60) % 60, s % 60)
}

// Kalenderrechnung nach Howard Hinnant (proleptisch gregorianisch).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar() {
        assert_eq!(day_string(0), "1970-01-01");
        assert_eq!(parse_day("2026-11-15"), Some(20_772));
        assert_eq!(day_string(20_772), "2026-11-15");
        for d in [-1000, 0, 59, 60, 11_016, 20_000, 30_000] {
            assert_eq!(parse_day(&day_string(d)), Some(d));
        }
    }

    #[test]
    fn times() {
        assert_eq!(parse_time("1970-01-01T00:01:00Z"), Some(60));
        assert_eq!(parse_time("2023-05-05T10:05:26.420578Z"), Some(1_683_281_126));
        assert_eq!(parse_time("2023-05-05T12:05:26+02:00"), Some(1_683_281_126));
        assert_eq!(format_time(1_683_281_126), "2023-05-05T10:05:26Z");
        assert_eq!(parse_time("kaputt"), None);
    }

    #[test]
    fn steps() {
        assert_eq!(nice_step(3600, 120), 30);
        assert_eq!(nice_step(86_400, 120), 900);
    }
}
