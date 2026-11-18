//! Verzeichnis aller Diagramme und Reihen.
//!
//! Sammler melden ihre Diagramme mit [`Registry::chart`] an (beliebig oft – der Aufruf ist billig,
//! wenn sich nichts geändert hat) und bekommen ein [`ChartHandle`], mit dem sie Werte abgeben.
//! Die Nummern der Reihen bleiben über Neustarts gleich (Tabellen `series` und `charts`).

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use parking_lot::{Mutex, RwLock};
use shared::{
    catalog::{self, Tpl},
    model::{ChartKind, ChartMeta, DimMeta, SeriesId, Unit},
};

use crate::{db::Db, util};

/// Beschreibung eines Diagramms beim Anmelden.
#[derive(Debug, Clone)]
pub struct ChartDef {
    pub id: String,
    pub context: String,
    pub section: String,
    pub family: String,
    pub title: String,
    pub unit: Unit,
    pub kind: ChartKind,
    pub priority: u32,
    /// 0 = Takt der lokalen Sammler.
    pub interval: u32,
    /// (ID, Anzeigename, unter der Nulllinie)
    pub dims: Vec<(String, String, bool)>,
    pub labels: BTreeMap<String, String>,
    instance_label: &'static str,
}

impl ChartDef {
    /// Diagramm aus einer Katalog-Vorlage (ohne Instanz: ID = Kontext).
    pub fn tpl(t: &Tpl) -> Self {
        Self {
            id: t.context.to_string(),
            context: t.context.to_string(),
            section: t.section.to_string(),
            family: String::new(),
            title: t.title.to_string(),
            unit: t.unit,
            kind: t.kind,
            priority: t.priority,
            interval: t.interval,
            dims: t.dims.iter().map(|d| (d.id.to_string(), d.name.to_string(), d.negate)).collect(),
            labels: BTreeMap::new(),
            instance_label: t.instance_label,
        }
    }

    /// Freies Diagramm ausserhalb des Katalogs.
    pub fn custom(id: &str, section: &str, title: &str, unit: Unit, kind: ChartKind) -> Self {
        Self {
            id: id.to_string(),
            context: id.to_string(),
            section: section.to_string(),
            family: String::new(),
            title: title.to_string(),
            unit,
            kind,
            priority: 1000,
            interval: 0,
            dims: Vec::new(),
            labels: BTreeMap::new(),
            instance_label: "",
        }
    }

    /// Instanz setzen (Gerät, Schnittstelle, Container …): ID wird `kontext.instanz`, die Instanz
    /// steht als Familie und im Label der Vorlage.
    #[must_use]
    pub fn instance(mut self, name: &str) -> Self {
        self.id = format!("{}.{}", self.context, catalog::sanitize(name));
        self.family = name.to_string();
        if !self.instance_label.is_empty() {
            self.labels.insert(self.instance_label.to_string(), name.to_string());
        }
        self
    }

    #[must_use]
    pub fn family(mut self, family: &str) -> Self {
        self.family = family.to_string();
        self
    }

    #[must_use]
    pub fn title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    #[must_use]
    pub fn label(mut self, key: &str, value: impl Into<String>) -> Self {
        self.labels.insert(key.to_string(), value.into());
        self
    }

    #[must_use]
    pub fn priority(mut self, p: u32) -> Self {
        self.priority = p;
        self
    }

    #[must_use]
    pub fn every(mut self, secs: u32) -> Self {
        self.interval = secs;
        self
    }

    /// Dimensionen ersetzen (für Vorlagen mit Laufzeit-Dimensionen): (ID, Anzeigename).
    #[must_use]
    pub fn dims<I, A, B>(mut self, dims: I) -> Self
    where
        I: IntoIterator<Item = (A, B)>,
        A: Into<String>,
        B: Into<String>,
    {
        self.dims = dims.into_iter().map(|(a, b)| (a.into(), b.into(), false)).collect();
        self
    }

    /// Eine Dimension anhängen.
    #[must_use]
    pub fn dim(mut self, id: &str, name: &str, negate: bool) -> Self {
        self.dims.push((id.to_string(), name.to_string(), negate));
        self
    }
}

/// Griff auf ein angemeldetes Diagramm. Billig zu klonen.
#[derive(Debug, Clone)]
pub struct ChartHandle {
    pub id: Arc<str>,
    /// Reihen in der Reihenfolge der Dimensionen.
    pub series: Arc<[SeriesId]>,
    /// Sekunden zwischen zwei Werten.
    pub interval: u32,
}

impl ChartHandle {
    pub fn len(&self) -> usize {
        self.series.len()
    }

    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }
}

struct Entry {
    meta: ChartMeta,
    handle: ChartHandle,
}

#[derive(Default)]
struct Inner {
    charts: HashMap<String, Entry>,
    /// (Diagramm, Dimension) → Reihe; enthält auch Reihen früherer Läufe.
    keys: HashMap<(String, String), SeriesId>,
    /// Reihe → Takt in Sekunden (Index = Nummer).
    intervals: Vec<u32>,
    next: SeriesId,
}

pub struct Registry {
    inner: RwLock<Inner>,
    generation: AtomicU64,
    default_interval: u32,
    db: Option<Mutex<duckdb::Connection>>,
}

impl Registry {
    /// Verzeichnis laden. Bekannte Diagramme früherer Läufe gelten als „liefert nichts mehr“,
    /// bis ein Sammler sie wieder anmeldet – ihre alten Daten bleiben abfragbar.
    pub fn load(db: Option<&Db>, default_interval: u32) -> anyhow::Result<Self> {
        let mut inner = Inner { next: 1, intervals: vec![default_interval], ..Inner::default() };
        let conn = match db {
            Some(db) => {
                let conn = db.conn()?;
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS series (id UINTEGER PRIMARY KEY, chart VARCHAR NOT NULL, dim VARCHAR NOT NULL);
                     CREATE TABLE IF NOT EXISTS charts (id VARCHAR PRIMARY KEY, meta VARCHAR NOT NULL, updated BIGINT NOT NULL);",
                )?;
                {
                    let mut stmt = conn.prepare("SELECT id, chart, dim FROM series")?;
                    let rows = stmt.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
                    for row in rows {
                        let (id, chart, dim) = row?;
                        inner.next = inner.next.max(id + 1);
                        inner.keys.insert((chart, dim), id);
                    }
                    let mut stmt = conn.prepare("SELECT meta FROM charts")?;
                    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
                    for row in rows {
                        let Ok(mut meta) = serde_json::from_str::<ChartMeta>(&row?) else { continue };
                        meta.retired = true;
                        let handle = ChartHandle {
                            id: Arc::from(meta.id.as_str()),
                            series: meta.dims.iter().map(|d| d.series).collect(),
                            interval: meta.interval.max(1),
                        };
                        inner.charts.insert(meta.id.clone(), Entry { meta, handle });
                    }
                }
                Some(Mutex::new(conn))
            }
            None => None,
        };
        inner.intervals.resize(inner.next as usize, default_interval);
        for e in inner.charts.values() {
            for s in e.handle.series.iter() {
                if let Some(slot) = inner.intervals.get_mut(*s as usize) {
                    *slot = e.handle.interval;
                }
            }
        }
        Ok(Self { inner: RwLock::new(inner), generation: AtomicU64::new(1), default_interval, db: conn })
    }

    /// Diagramm anmelden oder aktualisieren. Gleiche Angaben wie beim letzten Mal kosten fast nichts.
    pub fn chart(&self, def: ChartDef) -> ChartHandle {
        let interval = if def.interval == 0 { self.default_interval } else { def.interval };
        {
            let inner = self.inner.read();
            if let Some(e) = inner.charts.get(&def.id) {
                if !e.meta.retired && same(&e.meta, &def, interval) {
                    return e.handle.clone();
                }
            }
        }
        let mut inner = self.inner.write();
        let mut new_series = Vec::new();
        let mut dims = Vec::with_capacity(def.dims.len());
        for (id, name, negate) in &def.dims {
            let key = (def.id.clone(), id.clone());
            let series = match inner.keys.get(&key) {
                Some(s) => *s,
                None => {
                    let s = inner.next;
                    inner.next += 1;
                    inner.keys.insert(key, s);
                    new_series.push((s, id.clone()));
                    s
                }
            };
            if inner.intervals.len() <= series as usize {
                inner.intervals.resize(series as usize + 1, self.default_interval);
            }
            inner.intervals[series as usize] = interval;
            dims.push(DimMeta { id: id.clone(), name: name.clone(), series, negate: *negate });
        }
        let meta = ChartMeta {
            id: def.id.clone(),
            context: def.context,
            section: def.section,
            family: def.family,
            title: def.title,
            unit: def.unit,
            kind: def.kind,
            priority: def.priority,
            interval,
            dims,
            labels: def.labels,
            last: 0,
            retired: false,
        };
        let handle = ChartHandle {
            id: Arc::from(meta.id.as_str()),
            series: meta.dims.iter().map(|d| d.series).collect(),
            interval,
        };
        self.persist(&meta, &new_series);
        inner.charts.insert(def.id, Entry { meta, handle: handle.clone() });
        self.generation.fetch_add(1, Ordering::Relaxed);
        handle
    }

    fn persist(&self, meta: &ChartMeta, new_series: &[(SeriesId, String)]) {
        let Some(db) = &self.db else { return };
        let conn = db.lock();
        let result = (|| -> duckdb::Result<()> {
            for (id, dim) in new_series {
                conn.execute("INSERT OR REPLACE INTO series VALUES (?, ?, ?)", duckdb::params![id, meta.id, dim])?;
            }
            let json = serde_json::to_string(meta).unwrap_or_default();
            conn.execute("INSERT OR REPLACE INTO charts VALUES (?, ?, ?)", duckdb::params![meta.id, json, util::now()])?;
            Ok(())
        })();
        if let Err(e) = result {
            tracing::warn!("Diagramm {} nicht gespeichert: {e}", meta.id);
        }
    }

    /// Diagramm liefert nichts mehr (Container entfernt, Gerät weg). Alte Daten bleiben abfragbar.
    pub fn retire(&self, chart_id: &str) {
        let mut inner = self.inner.write();
        if let Some(e) = inner.charts.get_mut(chart_id) {
            if !e.meta.retired {
                e.meta.retired = true;
                self.generation.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Diagramm ganz vergessen (wenn es dazu keine gespeicherten Daten mehr gibt).
    /// Die Nummern seiner Reihen bleiben vergeben.
    pub fn forget(&self, chart_id: &str) {
        let removed = self.inner.write().charts.remove(chart_id).is_some();
        if removed {
            self.generation.fetch_add(1, Ordering::Relaxed);
            if let Some(db) = &self.db {
                let _ = db.lock().execute("DELETE FROM charts WHERE id = ?", duckdb::params![chart_id]);
            }
        }
    }

    /// Zählt bei jeder Änderung hoch.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn get(&self, chart_id: &str) -> Option<ChartMeta> {
        self.inner.read().charts.get(chart_id).map(|e| e.meta.clone())
    }

    pub fn handle(&self, chart_id: &str) -> Option<ChartHandle> {
        self.inner.read().charts.get(chart_id).map(|e| e.handle.clone())
    }

    /// Alle Diagramme, sortiert nach Bereich, Rang, Familie, ID.
    pub fn list(&self, include_retired: bool) -> Vec<ChartMeta> {
        let inner = self.inner.read();
        let mut out: Vec<ChartMeta> =
            inner.charts.values().filter(|e| include_retired || !e.meta.retired).map(|e| e.meta.clone()).collect();
        out.sort_by(|a, b| {
            (catalog::section_order(&a.section), a.priority, &a.family, &a.id)
                .cmp(&(catalog::section_order(&b.section), b.priority, &b.family, &b.id))
        });
        out
    }

    /// Diagramme eines Kontexts (ohne stillgelegte).
    pub fn by_context(&self, context: &str) -> Vec<ChartMeta> {
        let inner = self.inner.read();
        let mut out: Vec<ChartMeta> =
            inner.charts.values().filter(|e| !e.meta.retired && e.meta.context == context).map(|e| e.meta.clone()).collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// (Diagramme, Reihen) – nur aktive.
    pub fn counts(&self) -> (usize, usize) {
        let inner = self.inner.read();
        let active = inner.charts.values().filter(|e| !e.meta.retired);
        active.fold((0, 0), |(c, s), e| (c + 1, s + e.meta.dims.len()))
    }

    /// Höchste vergebene Reihennummer + 1.
    pub fn series_capacity(&self) -> usize {
        self.inner.read().next as usize
    }

    /// Takt einer Reihe in Sekunden.
    pub fn series_interval(&self, series: SeriesId) -> u32 {
        self.inner.read().intervals.get(series as usize).copied().unwrap_or(self.default_interval)
    }

    pub fn default_interval(&self) -> u32 {
        self.default_interval
    }
}

fn same(meta: &ChartMeta, def: &ChartDef, interval: u32) -> bool {
    meta.interval == interval
        && meta.dims.len() == def.dims.len()
        && meta.dims.iter().zip(&def.dims).all(|(m, (id, name, negate))| m.id == *id && m.name == *name && m.negate == *negate)
        && meta.title == def.title
        && meta.family == def.family
        && meta.labels == def.labels
        && meta.unit == def.unit
        && meta.kind == def.kind
        && meta.priority == def.priority
        && meta.section == def.section
        && meta.context == def.context
}

#[cfg(test)]
mod tests {
    use shared::catalog::system::{CPU_CORES, DISK_IO, SYSTEM_CPU};

    use super::*;

    #[test]
    fn registers_and_reuses() {
        let r = Registry::load(None, 1).expect("registry");
        let a = r.chart(ChartDef::tpl(&SYSTEM_CPU));
        let g = r.generation();
        let b = r.chart(ChartDef::tpl(&SYSTEM_CPU));
        assert_eq!(a.series, b.series);
        assert_eq!(r.generation(), g, "unverändert → keine neue Generation");
        let d = r.chart(ChartDef::tpl(&DISK_IO).instance("nvme0n1"));
        assert_eq!(&*d.id, "disk.io.nvme0n1");
        assert_eq!(r.get("disk.io.nvme0n1").and_then(|m| m.label("device").map(str::to_string)).as_deref(), Some("nvme0n1"));
        let c1 = r.chart(ChartDef::tpl(&CPU_CORES).dims([("cpu0", "Kern 0")]));
        let c2 = r.chart(ChartDef::tpl(&CPU_CORES).dims([("cpu0", "Kern 0"), ("cpu1", "Kern 1")]));
        assert_eq!(c1.series[0], c2.series[0], "bestehende Dimension behält ihre Reihe");
        assert_eq!(c2.len(), 2);
        assert_eq!(r.counts().0, 3);
    }
}
