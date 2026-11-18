//! Messung: Bytes je Wert im Sekundenarchiv für verschiedene Parquet-Varianten.
//! `cargo test -p nrzmon-server store::bench -- --ignored --nocapture`

use std::time::Instant;

use crate::util::{self, Walk};

/// Realistische Reihen: Prozent, Raten, Temperaturen, Speicherstände, Zähler, oft Null.
/// `hard`: fast nur Zufallsläufe (ungünstigster Fall). Sonst wie auf einem Server mit ~80 Containern:
/// viele ruhen, Raten oft null, Speicherstände ändern sich in Seitenschritten.
pub(crate) fn synth(series: u32, secs: i64, start: i64, hard: bool, mut emit: impl FnMut(u32, i64, f64)) {
    fastrand::seed(7);
    let mut gens: Vec<Box<dyn FnMut(i64) -> f64>> = Vec::new();
    for s in 0..series {
        if !hard {
            gens.push(realistic(s));
            continue;
        }
        let g: Box<dyn FnMut(i64) -> f64> = match s % 10 {
            0..=2 => {
                let mut w = Walk::new(fastrand::f64() * 60.0, 0.0, 100.0, 1.5);
                Box::new(move |_| util::round(w.next_spiky(0.01, 30.0), 2))
            }
            3 | 4 => {
                let mut w = Walk::new(fastrand::f64() * 5e6, 0.0, 1e8, 2e5);
                let idle = fastrand::bool();
                Box::new(move |_| if idle && fastrand::f64() < 0.7 { 0.0 } else { util::round(w.next(), 0) })
            }
            5 => {
                let mut w = Walk::new(40.0 + fastrand::f64() * 20.0, 20.0, 90.0, 0.3);
                Box::new(move |_| util::round(w.next(), 1))
            }
            6 => {
                let mut w = Walk::new(2e9 * fastrand::f64(), 1e8, 8e9, 1e5);
                Box::new(move |t| if t % 5 == 0 { util::round(w.next(), 0) } else { util::round(w.value, 0) })
            }
            7 => {
                let base = fastrand::u32(1..200) as f64;
                Box::new(move |t| base + ((t / 600) % 3) as f64)
            }
            8 => {
                let mut w = Walk::new(5.0, 0.0, 500.0, 2.0);
                Box::new(move |_| util::round(w.next(), 2))
            }
            _ => Box::new(|_| 0.0),
        };
        gens.push(g);
    }
    for t in start..start + secs {
        for (s, g) in gens.iter_mut().enumerate() {
            emit(s as u32 + 1, t, g(t));
        }
    }
}

fn realistic(s: u32) -> Box<dyn FnMut(i64) -> f64> {
    match s % 20 {
        // CPU eines ruhenden Containers
        0..=3 => {
            let mut w = Walk::new(0.3, 0.0, 100.0, 0.08);
            Box::new(move |_| util::round(w.next_spiky(0.002, 20.0), 2))
        }
        // CPU eines arbeitenden Containers / Kerns
        4 => {
            let mut w = Walk::new(25.0, 0.0, 100.0, 2.0);
            Box::new(move |_| util::round(w.next_spiky(0.01, 30.0), 2))
        }
        // Arbeitsspeicher: Seitenschritte, selten
        5 | 6 => {
            let mut v = (fastrand::u64(1..50_000) * 4096) as f64;
            Box::new(move |_| {
                if fastrand::f64() < 0.1 {
                    v += (fastrand::i64(-20..20) * 4096) as f64;
                }
                v
            })
        }
        // Netzwerk und Datenträger: meist null, sonst Ausschläge
        7..=10 => {
            let busy = fastrand::f64() < 0.25;
            Box::new(move |_| {
                if fastrand::f64() < if busy { 0.6 } else { 0.05 } { util::round(fastrand::f64() * 2e5, 0) } else { 0.0 }
            })
        }
        // Temperaturen
        11 => {
            let mut w = Walk::new(45.0, 20.0, 90.0, 0.2);
            Box::new(move |_| util::round(w.next(), 1))
        }
        // Belegung in Prozent, Anzahl Prozesse/Verbindungen
        12 | 13 => {
            let mut w = Walk::new(fastrand::f64() * 80.0, 0.0, 100.0, 0.01);
            Box::new(move |t| if t % 10 == 0 { util::round(w.next(), 2) } else { util::round(w.value, 2) })
        }
        14 | 15 => {
            let mut v = fastrand::u32(1..200) as f64;
            Box::new(move |_| {
                if fastrand::f64() < 0.02 {
                    v = (v + fastrand::i32(-2..3) as f64).max(0.0);
                }
                v
            })
        }
        // Lastmittel, Latenzen
        16 => {
            let mut w = Walk::new(1.0, 0.0, 8.0, 0.05);
            Box::new(move |_| util::round(w.next(), 2))
        }
        17 => {
            let mut w = Walk::new(12.0, 0.5, 200.0, 1.0);
            Box::new(move |_| util::round(w.next_spiky(0.01, 50.0), 2))
        }
        _ => Box::new(|_| 0.0),
    }
}

#[test]
#[ignore = "Messung, dauert einige Sekunden"]
fn parquet_variants() {
    let dir = std::env::temp_dir().join(format!("nrzmon-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp");
    let conn = duckdb::Connection::open_in_memory().expect("duckdb");
    conn.execute_batch("SET threads=2; CREATE TABLE raw (series UINTEGER, ts BIGINT, v DOUBLE);").expect("tabelle");
    let (series, secs) = (2000u32, 3600i64);
    for hard in [false, true] {
    conn.execute_batch("DELETE FROM raw;").expect("leeren");
    let t0 = Instant::now();
    {
        let mut app = conn.appender("raw").expect("appender");
        synth(series, secs, 1_700_000_000, hard, |s, t, v| app.append_row(duckdb::params![s, t, v]).expect("append"));
        app.flush().expect("flush");
    }
    let n: i64 = conn.query_row("SELECT count(*) FROM raw", [], |r| r.get(0)).expect("count");
    eprintln!("\n{} – {n} Werte erzeugt in {:?}", if hard { "Ungünstig (Zufallsläufe)" } else { "Realistisch" }, t0.elapsed());
    let variants = [
        ("ts BIGINT, v DOUBLE, zstd (Ausgang)", "ts, v", "COMPRESSION zstd"),
        ("FLOAT, v2, rg 256k, lvl 9", "ts::UINTEGER AS ts, v::FLOAT AS v", "COMPRESSION zstd, PARQUET_VERSION v2, ROW_GROUP_SIZE 262144, COMPRESSION_LEVEL 9"),
        ("FLOAT, v2, rg 256k, lvl 19", "ts::UINTEGER AS ts, v::FLOAT AS v", "COMPRESSION zstd, PARQUET_VERSION v2, ROW_GROUP_SIZE 262144, COMPRESSION_LEVEL 19"),
        ("FLOAT, v2, rg 1M, lvl 15", "ts::UINTEGER AS ts, v::FLOAT AS v", "COMPRESSION zstd, PARQUET_VERSION v2, ROW_GROUP_SIZE 1048576, COMPRESSION_LEVEL 15"),
        ("FLOAT, v2, rg 1M, lvl 19", "ts::UINTEGER AS ts, v::FLOAT AS v", "COMPRESSION zstd, PARQUET_VERSION v2, ROW_GROUP_SIZE 1048576, COMPRESSION_LEVEL 19"),
        ("FLOAT, v1, rg 1M, lvl 19", "ts::UINTEGER AS ts, v::FLOAT AS v", "COMPRESSION zstd, ROW_GROUP_SIZE 1048576, COMPRESSION_LEVEL 19"),
        ("DOUBLE, v2, rg 1M, lvl 19", "ts::UINTEGER AS ts, v", "COMPRESSION zstd, PARQUET_VERSION v2, ROW_GROUP_SIZE 1048576, COMPRESSION_LEVEL 19"),
        ("FLOAT, v2, rg 2M, lvl 19", "ts::UINTEGER AS ts, v::FLOAT AS v", "COMPRESSION zstd, PARQUET_VERSION v2, ROW_GROUP_SIZE 2097152, COMPRESSION_LEVEL 19"),
        ("FLOAT, v2, rg 1M, lvl 22", "ts::UINTEGER AS ts, v::FLOAT AS v", "COMPRESSION zstd, PARQUET_VERSION v2, ROW_GROUP_SIZE 1048576, COMPRESSION_LEVEL 22"),
    ];
    for (name, cols, opts) in variants {
        let file = dir.join("v.parquet");
        let t = Instant::now();
        conn.execute_batch(&format!(
            "COPY (SELECT series, {cols} FROM raw ORDER BY series, ts) TO '{}' (FORMAT parquet, {opts});",
            file.display()
        ))
        .expect("copy");
        let took = t.elapsed();
        let bytes = std::fs::metadata(&file).expect("meta").len();
        let t = Instant::now();
        let _: f64 = conn
            .query_row(&format!("SELECT avg(v) FROM read_parquet('{}') WHERE series IN (5, 900, 1777)", file.display()), [], |r| r.get(0))
            .expect("lesen");
        eprintln!(
            "{name:42} {:>10} B  {:.3} B/Wert  schreiben {:>5} ms  lesen 3 Reihen {:>4} ms",
            bytes,
            bytes as f64 / n as f64,
            took.as_millis(),
            t.elapsed().as_millis()
        );
    }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[ignore = "Messung"]
fn sql_variants() {
    let dir = std::env::temp_dir().join(format!("nrzmon-sqlbench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp");
    let conn = duckdb::Connection::open(dir.join("db.duckdb")).expect("duckdb");
    let s = 1_799_895_600i64;
    conn.execute_batch(&format!(
        "SET threads=2; SET memory_limit='256MB';
         CREATE TABLE h0 (series UINTEGER, ts UINTEGER, v FLOAT);
         INSERT INTO h0 SELECT (r % 2000 + 1)::UINTEGER, ({s} + r // 2000)::UINTEGER, round(50 + 30 * sin(r / 7.0), 2)::FLOAT FROM range(7200000) t(r);
         COPY (SELECT * FROM h0 ORDER BY series, ts) TO '{}' (FORMAT parquet, COMPRESSION zstd, COMPRESSION_LEVEL 19, PARQUET_VERSION v2, ROW_GROUP_SIZE 1048576);
         COPY (SELECT * FROM h0 ORDER BY series, ts) TO '{}' (FORMAT parquet, COMPRESSION zstd, COMPRESSION_LEVEL 19, PARQUET_VERSION v2, ROW_GROUP_SIZE 262144);",
        dir.join("a.parquet").display(),
        dir.join("b.parquet").display()
    ))
    .expect("setup");
    let list: Vec<String> = (0..100).map(|i| (i * 17 % 2000 + 1).to_string()).collect();
    let list = list.join(",");
    let a = dir.join("a.parquet").display().to_string();
    let b = dir.join("b.parquet").display().to_string();
    let e = s + 3600;
    let q = |name: &str, src: &str, filter: &str| {
        let sql = format!("SELECT series, ((ts::BIGINT - {s}) // 10)::BIGINT AS b, sum(v::DOUBLE), count(*), min(v), max(v) FROM {src} WHERE {filter} GROUP BY 1, 2");
        let t = Instant::now();
        let mut stmt = conn.prepare(&sql).expect("prep");
        let n = stmt.query_map([], |_| Ok(())).expect("q").count();
        eprintln!("{name:50} {n:>7} Zeilen {:>6} ms", t.elapsed().as_millis());
    };
    for _ in 0..2 {
        q("Datei 1M, IN, ts int", &format!("read_parquet('{a}')"), &format!("series IN ({list}) AND ts >= {s} AND ts < {e}"));
        q("Datei 1M, IN, ts UINTEGER", &format!("read_parquet('{a}')"), &format!("series IN ({list}) AND ts >= {s}::UINTEGER AND ts < {e}::UINTEGER"));
        q("Datei 1M, IN, ohne ts", &format!("read_parquet('{a}')"), &format!("series IN ({list})"));
        q("Datei 256k, IN, ohne ts", &format!("read_parquet('{b}')"), &format!("series IN ({list})"));
        q("Datei 1M, 5 Reihen am Stück", &format!("read_parquet('{a}')"), "series IN (100,101,102,103,104)");
        q("Tabelle, IN, ts int", "h0", &format!("series IN ({list}) AND ts >= {s} AND ts < {e}"));
        q("Tabelle, IN, ohne ts", "h0", &format!("series IN ({list})"));
        q("Tabelle, 5 Reihen", "h0", "series IN (100,101,102,103,104)");
    }
    let _ = std::fs::remove_dir_all(dir);
}
