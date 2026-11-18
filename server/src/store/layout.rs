//! Aufbau des Archivs: Stufen, Abschnitte, Dateinamen und das Verzeichnis dessen, was wo liegt.
//!
//! Jede Stufe ist in Abschnitte fester Länge geteilt. Ein offener Abschnitt liegt als Tabelle
//! `h{stufe}_{beginn}` in DuckDB, ein abgeschlossener als Datei `<data>/tiers/{stufe}/{beginn}.parquet`
//! (ZSTD, sortiert nach `(series, ts)`). Löschen = Datei löschen.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicI64, Ordering},
};

pub use shared::storage::TIERS;

use crate::util;

/// Länge eines Abschnitts je Stufe: 1 h Sekundenwerte, 6 h Minutenwerte, 7 Tage Stundenwerte.
pub const SECTION: [i64; TIERS] = [3600, 6 * 3600, 7 * 86_400];
/// So lange nach Ende einer Stunde nehmen wir noch verspätete Werte an (Cloudflare liefert spät).
pub const GRACE: i64 = 900;

/// Beginn des Abschnitts, in dem `ts` liegt.
pub fn section_of(tier: usize, ts: i64) -> i64 {
    ts.div_euclid(SECTION[tier]) * SECTION[tier]
}

pub fn table(tier: usize, start: i64) -> String {
    format!("h{tier}_{start}")
}

/// `h1_1700000000` → (1, 1700000000).
pub fn parse_table(name: &str) -> Option<(usize, i64)> {
    let rest = name.strip_prefix('h')?;
    let (t, s) = rest.split_once('_')?;
    let tier: usize = t.parse().ok()?;
    (tier < TIERS).then_some(())?;
    Some((tier, s.parse().ok()?))
}

/// Spalten der Tabellen und Dateien. Stufe 0: ein Wert; Stufe 1/2: Mittel, Minimum, Maximum, Anzahl.
pub fn columns(tier: usize) -> &'static str {
    if tier == 0 { "series UINTEGER, ts UINTEGER, v FLOAT" } else { "series UINTEGER, ts UINTEGER, v FLOAT, lo FLOAT, hi FLOAT, n USMALLINT" }
}

/// Optionen für `COPY … TO` – Ergebnis der Messung in `bench.rs`.
pub const PARQUET_OPTS: &str = "FORMAT parquet, COMPRESSION zstd, COMPRESSION_LEVEL 19, PARQUET_VERSION v2, ROW_GROUP_SIZE 1048576";

/// Pfad in SQL einsetzen (der Datenordner kann Leerzeichen und Hochkommas enthalten).
pub fn sql_str(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "''"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileInfo {
    pub bytes: u64,
    pub rows: u64,
}

/// Was gerade wo liegt. Der Schreib-Thread ändert es, Abfragen lesen eine Kopie.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    /// Alles vor diesem Zeitpunkt ist in Stufe 1/2 verdichtet und in Stufe 0 abgeschlossen.
    pub closed: i64,
    pub hot: BTreeSet<(usize, i64)>,
    pub files: BTreeMap<(usize, i64), FileInfo>,
}

impl Catalog {
    /// Abschnitte einer Stufe, die `[from, to)` berühren: (Dateien, offene Tabellen).
    /// Gibt es zu einem Abschnitt schon die Datei, zählt nur sie.
    pub fn sources(&self, tier: usize, from: i64, to: i64, dir: &Path) -> (Vec<PathBuf>, Vec<String>) {
        let len = SECTION[tier];
        let lo = section_of(tier, from);
        let files = self
            .files
            .range((tier, lo)..(tier, to))
            .filter(|((_, s), _)| s + len > from)
            .map(|((t, s), _)| file_path(dir, *t, *s))
            .collect();
        let hot = self
            .hot
            .range((tier, lo)..(tier, to))
            .filter(|(t, s)| s + len > from && !self.files.contains_key(&(*t, *s)))
            .map(|(t, s)| table(*t, *s))
            .collect();
        (files, hot)
    }

    pub fn tier_bytes(&self, tier: usize) -> u64 {
        self.files.range((tier, i64::MIN)..=(tier, i64::MAX)).map(|(_, f)| f.bytes).sum()
    }
}

pub fn tier_dir(dir: &Path, tier: usize) -> PathBuf {
    dir.join("tiers").join(tier.to_string())
}

pub fn file_path(dir: &Path, tier: usize, start: i64) -> PathBuf {
    tier_dir(dir, tier).join(format!("{start}.parquet"))
}

/// Uhr des Speichers – in Tests verstellbar.
#[derive(Debug, Default)]
pub struct Clock(AtomicI64);

impl Clock {
    pub fn now(&self) -> i64 {
        match self.0.load(Ordering::Relaxed) {
            0 => util::now(),
            t => t,
        }
    }

    pub fn now_ms(&self) -> i64 {
        match self.0.load(Ordering::Relaxed) {
            0 => util::now_ms(),
            t => t * 1000,
        }
    }

    #[cfg(test)]
    pub fn set(&self, ts: i64) {
        self.0.store(ts, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_sources() {
        assert_eq!(parse_table(&table(1, 21_600)), Some((1, 21_600)));
        assert_eq!(parse_table("h9_1"), None);
        assert_eq!(parse_table("series"), None);
        let mut c = Catalog::default();
        c.files.insert((0, 0), FileInfo { bytes: 1, rows: 1 });
        c.files.insert((0, 3600), FileInfo { bytes: 1, rows: 1 });
        c.hot.insert((0, 3600));
        c.hot.insert((0, 7200));
        let (f, h) = c.sources(0, 1800, 7300, Path::new("/d"));
        assert_eq!(f.len(), 2);
        assert_eq!(h, vec!["h0_7200".to_string()], "Abschnitt mit Datei zählt nur einmal");
        let (f, h) = c.sources(0, 3600, 7200, Path::new("/d"));
        assert_eq!((f.len(), h.len()), (1, 0));
    }
}
