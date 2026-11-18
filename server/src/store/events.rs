//! Ereignisse dauerhaft in DuckDB: fortlaufende Nummern, Filter wie [`EventQuery`], Freitext.
//! Frisch aufgenommene Ereignisse, die der Schreib-Thread noch nicht geschrieben hat, werden aus
//! der Warteliste dazugemischt – eine Abfrage direkt nach `add_event` findet sie also schon.

use std::collections::BTreeMap;

use duckdb::{Connection, ToSql};
use shared::events::{Event, EventPage, EventQuery, Severity};

pub fn sev_num(s: Severity) -> u8 {
    match s {
        Severity::Info => 0,
        Severity::Ok => 1,
        Severity::Warn => 2,
        Severity::Crit => 3,
    }
}

fn sev_from(n: u8) -> Severity {
    match n {
        1 => Severity::Ok,
        2 => Severity::Warn,
        3 => Severity::Crit,
        _ => Severity::Info,
    }
}

/// Passt ein Ereignis zum Filter? (für die Warteliste; gleiche Regeln wie das SQL unten)
pub fn matches(e: &Event, q: &EventQuery) -> bool {
    let needle = q.q.as_deref().map(str::to_lowercase).filter(|s| !s.is_empty());
    q.after.is_none_or(|t| e.ts >= t)
        && q.before.is_none_or(|t| e.ts < t)
        && q.source.as_deref().filter(|s| !s.is_empty()).is_none_or(|s| e.source == s)
        && q.kind.as_deref().filter(|s| !s.is_empty()).is_none_or(|k| e.kind == k || e.kind.starts_with(&format!("{k}.")))
        && q.subject.as_deref().filter(|s| !s.is_empty()).is_none_or(|s| e.subject == s)
        && q.severity.is_none_or(|s| e.severity >= s)
        && needle.as_deref().is_none_or(|n| {
            e.title.to_lowercase().contains(n) || e.message.to_lowercase().contains(n) || e.subject.to_lowercase().contains(n)
        })
}

pub fn query(conn: &Connection, q: &EventQuery, pending: &[Event]) -> anyhow::Result<EventPage> {
    let mut wh: Vec<&str> = vec!["TRUE"];
    let mut args: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(t) = q.after {
        wh.push("ts >= ?");
        args.push(Box::new(t));
    }
    if let Some(t) = q.before {
        wh.push("ts < ?");
        args.push(Box::new(t));
    }
    if let Some(s) = q.source.clone().filter(|s| !s.is_empty()) {
        wh.push("source = ?");
        args.push(Box::new(s));
    }
    if let Some(k) = q.kind.clone().filter(|s| !s.is_empty()) {
        wh.push("(kind = ? OR starts_with(kind, ?))");
        args.push(Box::new(k.clone()));
        args.push(Box::new(format!("{k}.")));
    }
    if let Some(s) = q.subject.clone().filter(|s| !s.is_empty()) {
        wh.push("subject = ?");
        args.push(Box::new(s));
    }
    if let Some(s) = q.severity {
        wh.push("severity >= ?");
        args.push(Box::new(sev_num(s)));
    }
    if let Some(n) = q.q.as_deref().map(str::to_lowercase).filter(|s| !s.is_empty()) {
        wh.push("(contains(lower(title), ?) OR contains(lower(message), ?) OR contains(lower(subject), ?))");
        for _ in 0..3 {
            args.push(Box::new(n.clone()));
        }
    }
    let filter = wh.join(" AND ");
    let limit = q.limit.unwrap_or(200).clamp(1, 2000) as usize;
    let params: Vec<&dyn ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let total: i64 = conn.query_row(&format!("SELECT count(*) FROM events WHERE {filter}"), params.as_slice(), |r| r.get(0))?;
    let mut stmt = conn.prepare(&format!(
        "SELECT id, ts, source, kind, severity, subject, title, message, labels FROM events WHERE {filter} ORDER BY id DESC LIMIT {limit}"
    ))?;
    let mut events: Vec<Event> = stmt
        .query_map(params.as_slice(), |r| {
            let labels: Option<String> = r.get(8)?;
            Ok(Event {
                id: r.get(0)?,
                ts: r.get(1)?,
                source: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                kind: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                severity: sev_from(r.get::<_, Option<u8>>(4)?.unwrap_or(0)),
                subject: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                title: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                message: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
                labels: labels.filter(|l| !l.is_empty()).and_then(|l| serde_json::from_str::<BTreeMap<String, String>>(&l).ok()).unwrap_or_default(),
            })
        })?
        .collect::<Result<_, _>>()?;
    let mut total = total as u64;
    let known: std::collections::HashSet<u64> = events.iter().map(|e| e.id).collect();
    for e in pending.iter().filter(|e| matches(e, q)) {
        if !known.contains(&e.id) {
            events.push(e.clone());
            total += 1;
        }
    }
    events.sort_by_key(|e| std::cmp::Reverse(e.id));
    events.truncate(limit);
    Ok(EventPage { events, total })
}
