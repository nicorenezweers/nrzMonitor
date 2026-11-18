//! Zeit lesbar machen (Ortszeit des Browsers). Zahlen formatiert `shared::Unit::format`.

use js_sys::Date;
use wasm_bindgen::JsValue;

fn date(ts_ms: f64) -> Date {
    Date::new(&JsValue::from_f64(ts_ms))
}

/// Jetzt in Unix-Sekunden.
pub fn now() -> i64 {
    (Date::now() / 1000.0) as i64
}

/// Jetzt in Unix-Millisekunden.
pub fn now_ms() -> i64 {
    Date::now() as i64
}

/// `14:03:27`
pub fn clock(ts: i64) -> String {
    let d = date(ts as f64 * 1000.0);
    format!("{:02}:{:02}:{:02}", d.get_hours(), d.get_minutes(), d.get_seconds())
}

/// `14:03`
pub fn hm(ts: i64) -> String {
    let d = date(ts as f64 * 1000.0);
    format!("{:02}:{:02}", d.get_hours(), d.get_minutes())
}

/// `15.11.`
pub fn day(ts: i64) -> String {
    let d = date(ts as f64 * 1000.0);
    format!("{:02}.{:02}.", d.get_date(), d.get_month() + 1)
}

/// `15.11.2026 14:03:27`
pub fn full(ts: i64) -> String {
    let d = date(ts as f64 * 1000.0);
    format!("{:02}.{:02}.{} {}", d.get_date(), d.get_month() + 1, d.get_full_year(), clock(ts))
}

/// Wie [`full`], für Unix-Millisekunden.
pub fn full_ms(ts_ms: i64) -> String {
    full(ts_ms / 1000)
}

/// Achsenbeschriftung passend zur Länge des Zeitraums.
pub fn axis(ts: i64, span_secs: i64) -> String {
    if span_secs <= 20 * 60 {
        clock(ts)
    } else if span_secs <= 36 * 3600 {
        hm(ts)
    } else if span_secs <= 8 * 86_400 {
        format!("{} {}", day(ts), hm(ts))
    } else {
        day(ts)
    }
}

/// Zeitpunkt im Tooltip passend zum Raster.
pub fn point(ts: i64, step: u32) -> String {
    let today = day(now()) == day(ts);
    match (step < 60, today) {
        (true, true) => clock(ts),
        (false, true) => hm(ts),
        (true, false) => format!("{} {}", day(ts), clock(ts)),
        (false, false) => format!("{} {}", day(ts), hm(ts)),
    }
}

/// `vor 5 min`, `vor 3 h`, `gerade eben`.
pub fn ago(ts: i64) -> String {
    let s = (now() - ts).max(0);
    match s {
        0..=9 => "gerade eben".to_string(),
        10..=59 => format!("vor {s} s"),
        60..=3599 => format!("vor {} min", s / 60),
        3600..=86_399 => format!("vor {} h", s / 3600),
        _ => format!("vor {} d", s / 86_400),
    }
}

/// Wie [`ago`], für Unix-Millisekunden.
pub fn ago_ms(ts_ms: i64) -> String {
    ago(ts_ms / 1000)
}
