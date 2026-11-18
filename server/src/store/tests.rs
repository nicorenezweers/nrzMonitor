//! Tests des gestuften Speichers: Stufen, Verdichtung, Mischen, Verspätung, Neustart,
//! Aufbewahrung, Obergrenze, Ereignisse, Aufräumen des Katalogs.

use std::{path::PathBuf, sync::Arc};

use shared::{
    ChartKind, Unit,
    events::{Event, EventQuery, Severity},
    query::{Agg, QueryReq, TIER_RAM},
    storage::{CorrelateReq, StatsReq, StorageSettings, TopReq},
};

use super::{Store, layout::section_of};
use crate::{
    config::Config,
    db::Db,
    frame::Frame,
    registry::{ChartDef, ChartHandle, Registry},
    ring::Ring,
};

/// Wochenanfang + 3 h: alle Abschnittsgrenzen sind sauber.
const BASE: i64 = 1_799_884_800 + 3 * 3600;

struct Env {
    dir: PathBuf,
    store: Arc<Store>,
    registry: Arc<Registry>,
    ring: Arc<Ring>,
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("nrzmon-store-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("tmp");
    d
}

fn open(dir: &std::path::Path, clock: i64) -> Env {
    let mut cfg = Config::from_env().expect("config");
    cfg.data_dir = dir.to_path_buf();
    let db = Db::open(&cfg).expect("db");
    let registry = Arc::new(Registry::load(Some(&db), 1).expect("registry"));
    let ring = Arc::new(Ring::new(900));
    let store = Store::open_at(&cfg, db, registry.clone(), ring.clone(), clock).expect("store");
    Env { dir: dir.to_path_buf(), store, registry, ring }
}

impl Env {
    fn chart(&self, id: &str, dims: &[&str]) -> ChartHandle {
        self.registry.chart(ChartDef::custom(id, "test", id, Unit::Count, ChartKind::Line).dims(dims.iter().map(|d| (*d, *d))))
    }

    /// Wie `Core::submit`: Ring und Speicher.
    fn put(&self, ts: i64, h: &ChartHandle, values: &[f64]) {
        let mut f = Frame::new(ts);
        f.put(h, values);
        self.ring.write(&f);
        self.store.ingest(&f);
    }

    fn at(&self, ts: i64) {
        self.store.sh.clock.set(ts);
    }

    async fn q(&self, id: &str, after: i64, before: i64, points: u32) -> shared::query::ChartData {
        let req = QueryReq { charts: vec![id.into()], after, before, points, agg: Agg::Avg, envelope: true };
        self.store.query(&req).await.expect("query").charts.remove(0)
    }

    async fn close(self) {
        self.store.shutdown().await;
    }
}

fn wave(t: i64) -> f64 {
    ((t - BASE) % 100) as f64
}

/// Drei Stunden Werte schreiben, wie im Betrieb Stunde für Stunde mit laufender Uhr.
async fn fill(e: &Env, h: &ChartHandle, hours: i64) {
    for hour in 0..hours {
        e.at(BASE + (hour + 1) * 3600);
        for t in BASE + hour * 3600..BASE + (hour + 1) * 3600 {
            e.put(t, h, &[wave(t), 5.0]);
        }
        e.store.maintain().await;
    }
}

#[tokio::test]
async fn tiers_compaction_and_mixing() {
    let d = dir("tiers");
    let e = open(&d, BASE);
    let h = e.chart("t.wave", &["w", "c"]);
    fill(&e, &h, 3).await;
    {
        let cat = e.store.sh.catalog.read();
        assert!(cat.files.contains_key(&(0, BASE)), "Stunde 0 als Datei: {:?}", cat.files.keys());
        assert!(cat.files.contains_key(&(0, BASE + 3600)));
        assert!(cat.hot.contains(&(0, BASE + 7200)), "Stunde 2 noch offen");
        assert_eq!(cat.closed, BASE + 7200);
        assert!(d.join("tiers/0").join(format!("{BASE}.parquet")).exists());
    }
    // Stufe 0: Sekunden genau so, wie sie kamen.
    let c = e.q("t.wave", BASE, BASE + 3599, 3600).await;
    assert_eq!((c.tier, c.step, c.start), (0, 1, BASE));
    for i in [0usize, 1, 99, 100, 1234, 3599] {
        assert_eq!(c.values[0][i], Some(wave(BASE + i as i64)), "Punkt {i}");
    }
    assert_eq!(c.values[1][10], Some(5.0));
    // Stufe 1: Minuten über alle drei Stunden – verdichtet, offene Stunde roh, Ende aus dem Ring.
    let c = e.q("t.wave", BASE, BASE + 3 * 3600 - 1, 180).await;
    assert_eq!((c.tier, c.step), (1, 60));
    assert_eq!(c.values[0].len(), 180);
    for (i, v) in c.values[0].iter().enumerate() {
        let from = BASE + i as i64 * 60;
        let raw: Vec<f64> = (from..from + 60).map(wave).collect();
        let avg = raw.iter().sum::<f64>() / 60.0;
        assert!((v.expect("Wert") - avg).abs() < 1e-3, "Minute {i}: {v:?} statt {avg}");
        assert_eq!(c.min[0][i], raw.iter().copied().reduce(f64::min));
        assert_eq!(c.max[0][i], raw.iter().copied().reduce(f64::max));
    }
    // Stufe 2: Stunden.
    let c = e.q("t.wave", BASE, BASE + 3 * 3600 - 1, 3).await;
    assert_eq!((c.tier, c.step), (2, 3600));
    assert_eq!(c.values[0], vec![Some(49.5); 3]);
    assert_eq!(c.min[0][0], Some(0.0));
    assert_eq!(c.max[0][2], Some(99.0));
    // Nur die letzten Minuten: ganz aus dem Arbeitsspeicher.
    let c = e.q("t.wave", -300, 0, 300).await;
    assert_eq!(c.tier, TIER_RAM);
    assert_eq!(c.values[0].iter().flatten().count(), 300);
    // Kennzahlen, Rangliste, Veränderung, CSV.
    let s = e.store.analyze_stats(&StatsReq { charts: vec!["t.wave".into()], after: BASE, before: BASE + 3600 }).await.expect("stats");
    let w = &s.series[0];
    assert_eq!((w.min, w.max), (Some(0.0), Some(99.0)));
    assert!((w.avg.expect("avg") - 49.5).abs() < 0.1);
    assert!((w.p50.expect("p50") - 49.5).abs() < 2.0);
    assert!(w.stddev.expect("stddev") > 20.0);
    let csv = e.store.export_csv(&["t.wave".into()], BASE, BASE + 59, 0).await.expect("csv");
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], "time,t.wave.w,t.wave.c");
    assert!(lines[2].ends_with(",1,5"), "{}", lines[2]);
    e.close().await;
    let _ = std::fs::remove_dir_all(d);
}

#[tokio::test]
async fn late_values_restart_and_broken_files() {
    let d = dir("late");
    let e = open(&d, BASE);
    let h = e.chart("t.late", &["a"]);
    let late = e.chart("t.cf", &["req"]);
    fill(&e, &h, 1).await;
    // 13 Minuten nach Stundenende kommen Werte für die Stunde davor (wie bei Cloudflare).
    e.at(BASE + 3600 + 780);
    for t in BASE + 3000..BASE + 3060 {
        e.put(t, &late, &[1000.0]);
    }
    e.store.maintain().await;
    e.at(BASE + 3600 + 901);
    e.store.maintain().await;
    // Jetzt ist die Stunde abgeschlossen: noch spätere Werte werden verworfen.
    e.put(BASE + 10, &late, &[7.0]);
    e.store.maintain().await;
    assert_eq!(e.store.sh.dropped.load(std::sync::atomic::Ordering::Relaxed), 1);
    let c = e.q("t.cf", BASE, BASE + 3599, 60).await;
    assert_eq!(c.tier, 1);
    assert_eq!(c.values[0][50], Some(1000.0), "verspätete Minute landet richtig");
    assert_eq!(c.values[0][0], None);
    let ev = e.store.add_event(Event::new("test", "x.start", Severity::Info, "a", "Vor dem Neustart"));
    // Halbe und kaputte Dateien eines Absturzes.
    std::fs::write(d.join("tiers/0/123.parquet"), b"kaputt").expect("schreiben");
    std::fs::write(d.join("tiers/0/456.parquet.tmp"), b"halb").expect("schreiben");
    let before = e.q("t.late", BASE, BASE + 3599, 120).await;
    e.close().await;

    let e = open(&d, BASE + 3600 + 960);
    let h = e.chart("t.late", &["a"]);
    assert!(!d.join("tiers/0/123.parquet").exists() && !d.join("tiers/0/456.parquet.tmp").exists());
    let after = e.q("t.late", BASE, BASE + 3599, 120).await;
    assert_eq!(before.values, after.values, "nach dem Neustart dieselben Werte");
    // Ungeschriebenes vor dem Beenden geht nicht verloren.
    e.put(BASE + 3600 + 950, &h, &[42.0]);
    e.close().await;
    let e = open(&d, BASE + 3600 + 1000);
    // Kurz nach dem Neustart: die letzten Minuten kommen vom Datenträger, nicht aus dem leeren Ring.
    let c = e.q("t.late", BASE + 3600 + 940, BASE + 3600 + 959, 20).await;
    assert_ne!(c.tier, TIER_RAM);
    assert_eq!(c.values[0][10], Some(42.0));
    let page = e.store.events(&EventQuery::default()).await.expect("events");
    assert_eq!(page.events[0].id, ev.id);
    let next = e.store.add_event(Event::new("test", "x.stop", Severity::Warn, "a", "Danach"));
    assert_eq!(next.id, ev.id + 1, "Nummern laufen weiter");
    e.close().await;
    let _ = std::fs::remove_dir_all(d);
}

#[tokio::test]
async fn retention_and_size_limit() {
    let d = dir("retention");
    let e = open(&d, BASE);
    let h = e.chart("t.ret", &["a", "b"]);
    fill(&e, &h, 4).await;
    let files = |t: usize| e.store.sh.catalog.read().files.keys().filter(|(tt, _)| *tt == t).count();
    assert_eq!(files(0), 3);
    // Obergrenze knapp unter der Belegung: genau die älteste Sekundendatei fällt weg.
    let total = e.store.sh.db_bytes() + e.store.sh.catalog.read().files.values().map(|f| f.bytes).sum::<u64>();
    *e.store.sh.settings.write() = StorageSettings { max_bytes: total - 1, ..StorageSettings::default() };
    e.store.maintain().await;
    let cat = e.store.sh.catalog.read().clone();
    assert!(!cat.files.contains_key(&(0, BASE)), "älteste Sekunden zuerst");
    assert!(!d.join("tiers/0").join(format!("{BASE}.parquet")).exists());
    // Minuten und Stunden der gelöschten Stunde bleiben abfragbar.
    let c = e.q("t.ret", BASE, BASE + 3599, 60).await;
    let expect = (BASE + 60..BASE + 120).map(wave).sum::<f64>() / 60.0;
    assert!((c.values[0][1].expect("Wert") - expect).abs() < 1e-3);
    // Aufbewahrung: Sekunden nur 1 h → nach 3 h ist keine Sekundendatei mehr da.
    e.store.set_settings(StorageSettings { retention: [3600, 86_400, 7 * 86_400], max_bytes: 0, ..StorageSettings::default() });
    e.at(BASE + 8 * 3600);
    e.store.maintain().await;
    assert_eq!(files(0), 0);
    assert!(e.store.sh.catalog.read().hot.iter().any(|(t, _)| *t == 1), "Minuten bleiben");
    let st = e.store.stats().await.expect("stats");
    assert_eq!(st.tiers.len(), 3);
    assert!(st.tiers[1].hot_rows > 0);
    assert!(st.db_bytes > 0);
    e.close().await;
    let _ = std::fs::remove_dir_all(d);
}

#[tokio::test]
async fn events_filters() {
    let d = dir("events");
    let e = open(&d, BASE);
    for (i, (src, kind, sev, title)) in [
        ("docker", "container.start", Severity::Info, "web gestartet"),
        ("docker", "container.oom", Severity::Crit, "web: Speicher voll"),
        ("alert", "alert.raised", Severity::Warn, "CPU hoch"),
        ("netbird", "peer.offline", Severity::Warn, "Laptop offline"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut ev = Event::new(src, kind, sev, "web", title);
        ev.ts = BASE * 1000 + i as i64 * 1000;
        e.store.add_event(ev.label("k", "v"));
    }
    let q = |f: fn(&mut EventQuery)| {
        let mut q = EventQuery::default();
        f(&mut q);
        q
    };
    // Sofort sichtbar (Warteliste) …
    assert_eq!(e.store.events(&EventQuery::default()).await.expect("e").total, 4);
    e.store.maintain().await;
    // … und danach aus DuckDB.
    let all = e.store.events(&EventQuery::default()).await.expect("e");
    assert_eq!(all.total, 4);
    assert_eq!(all.events[0].title, "Laptop offline");
    assert_eq!(all.events[0].labels.get("k").map(String::as_str), Some("v"));
    assert_eq!(e.store.events(&q(|q| q.source = Some("docker".into()))).await.expect("e").total, 2);
    assert_eq!(e.store.events(&q(|q| q.kind = Some("container".into()))).await.expect("e").total, 2);
    assert_eq!(e.store.events(&q(|q| q.severity = Some(Severity::Warn))).await.expect("e").total, 3);
    assert_eq!(e.store.events(&q(|q| q.q = Some("SPEICHER".into()))).await.expect("e").total, 1);
    let page = e.store.events(&q(|q| q.limit = Some(2))).await.expect("e");
    assert_eq!((page.events.len(), page.total), (2, 4));
    let older = e.store.events(&EventQuery { before: Some(page.events[1].ts), ..EventQuery::default() }).await.expect("e");
    assert_eq!(older.events.len(), 2, "Nachladen älterer");
    e.close().await;
    let _ = std::fs::remove_dir_all(d);
}

#[tokio::test]
async fn forget_retired_without_data_and_analysis() {
    let d = dir("forget");
    let e = open(&d, BASE);
    let a = e.chart("t.has", &["x"]);
    e.chart("t.empty", &["x"]);
    let p = e.chart("docker.cpu.web", &["cpu"]);
    let q = e.chart("docker.cpu.db", &["cpu"]);
    for t in BASE..BASE + 600 {
        e.put(t, &a, &[1.0]);
        e.put(t, &p, &[if t >= BASE + 480 { 80.0 } else { 10.0 }]);
        e.put(t, &q, &[5.0]);
    }
    e.registry.retire("t.has");
    e.registry.retire("t.empty");
    e.at(BASE + 600);
    e.store.maintain().await;
    let top = e.store.analyze_top(&TopReq { context: "docker.cpu.web".into(), after: BASE, before: BASE + 600, ..TopReq::default() }).await.expect("top");
    assert_eq!(top.items.len(), 1);
    let ch = e
        .store
        .analyze_correlate(&CorrelateReq { after: BASE + 480, before: BASE + 600, baseline_after: BASE, ..CorrelateReq::default() })
        .await
        .expect("correlate");
    assert_eq!(ch.items[0].chart, "docker.cpu.web", "{:?}", ch.items.iter().map(|i| (&i.chart, i.score)).collect::<Vec<_>>());
    assert!(ch.items.iter().all(|i| i.chart != "t.has" && i.chart != "docker.cpu.db"), "gleichbleibende Reihen fehlen");
    assert!(!ch.items[0].spark.is_empty());
    assert!(e.registry.get("t.empty").is_none(), "ohne Daten vergessen");
    assert!(e.registry.get("t.has").is_some(), "mit Daten behalten");
    assert_eq!(section_of(0, BASE + 10), BASE);
    e.close().await;
    let _ = std::fs::remove_dir_all(d);
}

/// 2000 Reihen über 24 h: Archiv erzeugen (wie nach einem Tag Betrieb), dann Abfragen messen.
/// `cargo test -p nrzmon-server store::tests::bench_query_24h -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Messung, dauert etwa eine Minute"]
async fn bench_query_24h() {
    let d = dir("bench24");
    {
        let e = open(&d, BASE);
        for c in 0..400 {
            e.chart(&format!("b.c{c}"), &["a", "b", "c", "d", "e"]);
        }
        e.close().await;
    }
    let db = {
        let mut cfg = Config::from_env().expect("config");
        cfg.data_dir = d.clone();
        Db::open(&cfg).expect("db")
    };
    let conn = db.conn().expect("conn");
    let t = std::time::Instant::now();
    for h in 0..25 {
        let s = BASE + h * 3600;
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS h0_{s} (series UINTEGER, ts UINTEGER, v FLOAT);
             INSERT INTO h0_{s} SELECT (r % 2000 + 1)::UINTEGER AS series, ({s} + r // 2000)::UINTEGER AS ts,
             round(CASE WHEN r % 3 = 0 THEN 0 ELSE 50 + 30 * sin((r // 2000) / 300.0 + r % 2000) END, 2)::FLOAT AS v FROM range(7200000) t(r);"
        ))
        .expect("erzeugen");
    }
    drop(conn);
    drop(db);
    eprintln!("Rohdaten erzeugt in {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let e = open(&d, BASE + 25 * 3600 + 100);
    for c in 0..400 {
        e.chart(&format!("b.c{c}"), &["a", "b", "c", "d", "e"]);
    }
    e.store.maintain().await;
    e.store.maintain().await;
    eprintln!("24 Stunden verdichtet und als Parquet abgelegt in {:?}", t.elapsed());
    let st = e.store.stats().await.expect("stats");
    for t in &st.tiers {
        eprintln!("Stufe {}: {} Zeilen, {} Bytes, {} Dateien, {:.3} B/Zeile, offen {}", t.tier, t.rows, t.bytes, t.files, t.bytes_per_value, t.hot_rows);
    }
    eprintln!("DuckDB-Datei: {} Bytes", st.db_bytes);
    let charts: Vec<String> = (0..20).map(|c| format!("b.c{}", c * 17)).collect();
    for (label, span, points) in [
        ("24 h, 360 Punkte", 86_400, 360),
        ("24 h, 360 Punkte (nochmals)", 86_400, 360),
        ("24 h, 5000 Punkte", 86_400, 5000),
        ("6 h, 360 Punkte", 21_600, 360),
        ("3 h, 5000 Punkte", 10_800, 5000),
        ("1 h, 360 Punkte", 3600, 360),
    ] {
        let req = QueryReq { charts: charts.clone(), after: -span, before: 0, points, agg: Agg::Avg, envelope: true };
        let t = std::time::Instant::now();
        let r = e.store.query(&req).await.expect("query");
        let took = t.elapsed();
        let filled = r.charts[0].values[0].iter().flatten().count();
        eprintln!("{label}: 20 Diagramme (100 Reihen) in {took:?}, Stufe {}, Schritt {} s, {filled} Punkte mit Wert", r.charts[0].tier, r.charts[0].step);
    }
    let t = std::time::Instant::now();
    let c = e.store.analyze_correlate(&CorrelateReq { after: -3600 - 1800, before: -1800, ..CorrelateReq::default() }).await.expect("corr");
    eprintln!("Was hat sich geändert (2000 Reihen, 1 h vs. 4 h davor): {:?}, {} untersucht", t.elapsed(), c.examined);
    e.close().await;
    let _ = std::fs::remove_dir_all(d);
}
