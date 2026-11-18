//! DuckDB: eine Datei (`<data>/nrzmonitor.duckdb`), mehrere Verbindungen im selben Prozess.
//!
//! Jeder Teil der Anwendung holt sich mit [`Db::conn`] eine eigene Verbindung und behält sie
//! (Verbindungen sind `Send`, aber nicht `Sync` – also je Thread/Aufgabe eine, oder in einem `Mutex`).
//! Abfragen blockieren: aus async-Code immer über `tokio::task::spawn_blocking` aufrufen.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use duckdb::Connection;
use parking_lot::Mutex;

use crate::config::Config;

#[derive(Clone)]
pub struct Db {
    base: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl Db {
    pub fn open(cfg: &Config) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&cfg.data_dir).with_context(|| format!("Datenordner {}", cfg.data_dir.display()))?;
        let path = cfg.data_dir.join("nrzmonitor.duckdb");
        let conn = Connection::open(&path).with_context(|| format!("DuckDB {}", path.display()))?;
        let tmp = cfg.data_dir.join("tmp");
        // Klein halten: nrzMonitor soll neben den überwachten Apps nicht auffallen.
        conn.execute_batch(&format!(
            "SET memory_limit='{}'; SET threads={}; SET temp_directory='{}'; SET preserve_insertion_order=false;",
            cfg.db_memory.replace('\'', ""),
            cfg.db_threads,
            tmp.display().to_string().replace('\'', "''"),
        ))
        .context("DuckDB-Einstellungen")?;
        Ok(Self { base: Arc::new(Mutex::new(conn)), path })
    }

    /// Neue Verbindung zur selben Datenbank.
    pub fn conn(&self) -> anyhow::Result<Connection> {
        self.base.lock().try_clone().context("DuckDB-Verbindung")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Kurze Abfrage auf der gemeinsamen Verbindung (für seltene, kleine Zugriffe).
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> duckdb::Result<T>) -> anyhow::Result<T> {
        let guard = self.base.lock();
        f(&guard).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn library_has_parquet_and_json() {
        let conn = duckdb::Connection::open_in_memory().expect("duckdb");
        let version: String = conn.query_row("SELECT version()", [], |r| r.get(0)).expect("version");
        let mut stmt = conn
            .prepare("SELECT extension_name FROM duckdb_extensions() WHERE loaded AND extension_name IN ('parquet', 'json') ORDER BY 1")
            .expect("prepare");
        let names: Vec<String> = stmt.query_map([], |r| r.get(0)).expect("query").flatten().collect();
        assert_eq!(names, ["json", "parquet"], "DuckDB {version} ohne eingebautes Parquet/JSON");
        let dir = std::env::temp_dir().join(format!("nrzmon-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp");
        let file = dir.join("t.parquet");
        conn.execute_batch(&format!(
            "COPY (SELECT range AS ts, (range % 7)::FLOAT AS v FROM range(1000)) TO '{}' (FORMAT parquet, COMPRESSION zstd);",
            file.display()
        ))
        .expect("parquet schreiben");
        let n: i64 = conn.query_row(&format!("SELECT count(*) FROM read_parquet('{}')", file.display()), [], |r| r.get(0)).expect("lesen");
        assert_eq!(n, 1000);
        let _ = std::fs::remove_dir_all(dir);
    }
}
