//! Ressourcen (lesbar und abonnierbar), Vorlagen für Prompts und Vervollständigung.

use axum::http::Method;
use serde_json::{Value, json};

use super::{Ctx, RpcErr, call_api, invalid, tools};
use crate::{core::Core, util};

fn res(uri: String, name: String, title: String, description: &str) -> Value {
    json!({"uri": uri, "name": name, "title": title, "description": description, "mimeType": "application/json"})
}

pub fn list(core: &Core) -> Vec<Value> {
    let mut out = vec![
        res(
            "nrzmon://overview".into(),
            "overview".into(),
            "Überblick".into(),
            "Zustand des Hosts mit den wichtigsten Live-Werten (jede Sekunde neu).",
        ),
        res("nrzmon://info".into(), "info".into(), "Server".into(), "Version, Host, Takt, Laufzeit."),
        res("nrzmon://catalog".into(), "catalog".into(), "Katalog".into(), "Alle Diagramme (kompakt)."),
        res("nrzmon://events/recent".into(), "events".into(), "Ereignisse".into(), "Die letzten 100 Ereignisse."),
    ];
    for key in core.snaps.keys() {
        out.push(res(
            format!("nrzmon://snap/{key}"),
            format!("snap:{key}"),
            format!("Momentaufnahme {key}"),
            "Letzter Stand, abonnierbar.",
        ));
    }
    for s in shared::catalog::SECTIONS {
        out.push(res(
            format!("nrzmon://section/{}", s.id),
            format!("section:{}", s.id),
            format!("Live: {}", s.title),
            "Neueste Werte aller Diagramme des Bereichs.",
        ));
    }
    out
}

pub fn templates() -> Vec<Value> {
    let t = |uri: &str, name: &str, title: &str, d: &str| json!({"uriTemplate": uri, "name": name, "title": title, "description": d, "mimeType": "application/json"});
    vec![
        t("nrzmon://chart/{id}", "chart", "Diagramm live", "Neueste Werte eines Diagramms (abonnierbar: jede neue Messung)."),
        t("nrzmon://context/{context}", "context", "Kontext live", "Neueste Werte aller Instanzen eines Kontexts, z. B. docker.cpu."),
        t("nrzmon://section/{section}", "section", "Bereich live", "Neueste Werte aller Diagramme eines Bereichs."),
        t("nrzmon://snap/{key}", "snap", "Momentaufnahme", "Liste oder Zustand, z. B. docker.containers."),
        t(
            "nrzmon://history/{id}?after={after}&points={points}",
            "history",
            "Verlauf",
            "Zeitreihe eines Diagramms (after relativ wie -1h).",
        ),
    ]
}

fn content(uri: &str, v: &Value) -> Value {
    json!({"uri": uri, "mimeType": "application/json", "text": v.to_string()})
}

pub async fn read(ctx: &Ctx, uri: &str) -> Result<Value, RpcErr> {
    let core = &ctx.core;
    let rest = uri.strip_prefix("nrzmon://").ok_or_else(|| invalid("Unbekannte Ressource (nrzmon://…)."))?;
    let (kind, arg) = rest.split_once('/').unwrap_or((rest, ""));
    let v = match kind {
        "overview" => tool_json(ctx, "overview", &json!({})).await?,
        "info" => {
            json!({"version": env!("CARGO_PKG_VERSION"), "hostname": core.hostname, "demo": core.cfg.demo,
            "started_at": core.started_at, "now": util::now(), "interval": core.cfg.interval, "live_window": core.cfg.live_window})
        }
        "catalog" => tool_json(ctx, "list_charts", &json!({"limit": 5000})).await?,
        "events" => {
            let (_, body) = call_api(ctx.api(), Method::GET, "/api/v1/events?limit=100", None).await.map_err(|e| (-32603, e))?;
            serde_json::from_slice(&body).unwrap_or(Value::Null)
        }
        "snap" => {
            let s = core.snaps.get(arg).ok_or_else(|| (-32002, format!("Keine Momentaufnahme `{arg}`.")))?;
            json!({"key": arg, "ts": s.ts, "data": s.data})
        }
        "chart" => {
            let c = core.registry.get(arg).ok_or_else(|| (-32002, format!("Diagramm `{arg}` gibt es nicht.")))?;
            tools::latest(core, &c)
        }
        "context" => tool_json(ctx, "live_values", &json!({"context": arg, "limit": 2000})).await?,
        "section" => tool_json(ctx, "live_values", &json!({"section": arg, "limit": 2000})).await?,
        "history" => {
            let (id, q) = arg.split_once('?').unwrap_or((arg, ""));
            let mut a = json!({"charts": [id]});
            for kv in q.split('&') {
                if let Some((k, v)) = kv.split_once('=') {
                    a[k] = json!(v);
                }
            }
            tool_json(ctx, "query_timeseries", &a).await?
        }
        _ => return Err((-32002, format!("Unbekannte Ressource `{uri}`."))),
    };
    Ok(content(uri, &v))
}

async fn tool_json(ctx: &Ctx, name: &str, args: &Value) -> Result<Value, RpcErr> {
    let r = tools::call(ctx, name, args).await;
    if r.get("isError").and_then(Value::as_bool) == Some(true) {
        let msg = r.pointer("/content/0/text").and_then(Value::as_str).unwrap_or("Fehler").to_string();
        return Err((-32603, msg));
    }
    Ok(r.get("structuredContent").cloned().unwrap_or(Value::Null))
}

// --- Prompts ----------------------------------------------------------------------------------------

pub fn prompts() -> Vec<Value> {
    vec![
        json!({"name": "health_check", "title": "Gesundheitscheck",
            "description": "Prüft Host, Container, Alarme und Ereignisse und fasst Auffälligkeiten zusammen."}),
        json!({"name": "container_diagnose", "title": "Container untersuchen",
            "description": "Untersucht einen Container: Zustand, Ressourcen, Logs, Ereignisse.",
            "arguments": [{"name": "name", "description": "Containername", "required": true}]}),
        json!({"name": "incident", "title": "Störung analysieren",
            "description": "Sucht die Ursache einer Spitze oder Störung in einem Zeitfenster.",
            "arguments": [{"name": "after", "description": "Beginn (z. B. -30m oder ISO-Zeit)", "required": false},
                          {"name": "before", "description": "Ende (Standard jetzt)", "required": false}]}),
        json!({"name": "capacity", "title": "Kapazität planen",
            "description": "Wertet Auslastung und Trends der letzten 30 Tage aus (CPU, RAM, Platten, Container)."}),
    ]
}

pub fn prompt(name: &str, args: &Value) -> Result<Value, RpcErr> {
    let arg = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let (description, text) = match name {
        "health_check" => (
            "Gesundheitscheck",
            "Prüfe den Zustand dieses Servers mit nrzMonitor: rufe `overview` auf, dann `alerts` und \
`events` (severity warn, letzte 24 h). Schau bei auffälligen Containern mit `docker_container` nach. Fasse kurz zusammen: \
was ist in Ordnung, was braucht Aufmerksamkeit (mit Zahlen), was sollte man tun."
                .to_string(),
        ),
        "container_diagnose" => {
            let n = arg("name");
            if n.is_empty() {
                return Err(invalid("`name` fehlt."));
            }
            (
                "Container untersuchen",
                format!(
                    "Untersuche den Container `{n}` mit nrzMonitor: `docker_container` (Zustand, Konfiguration, \
Live-Werte), `query_timeseries` mit label container={n} für die letzten 24 h (CPU, RAM, Netz, I/O), `docker_logs` (errors_only \
und die letzten 300 Zeilen), `events` mit subject={n}. Erkläre, ob etwas nicht stimmt (Neustarts, OOM, Health, Spitzen) und was die Ursache sein könnte."
                ),
            )
        }
        "incident" => {
            let after = if arg("after").is_empty() { "-30m".to_string() } else { arg("after") };
            let before = if arg("before").is_empty() { "now".to_string() } else { arg("before") };
            (
                "Störung analysieren",
                format!(
                    "Analysiere das Zeitfenster after={after} before={before} mit nrzMonitor: `what_changed` für das Fenster, \
`events` im selben Zeitraum, `top_instances` für docker.cpu und docker.mem. Schau dir die auffälligsten Reihen mit `query_timeseries` an \
und nenne die wahrscheinlichste Ursache mit Belegen."
                ),
            )
        }
        "capacity" => (
            "Kapazität planen",
            "Werte mit nrzMonitor die letzten 30 Tage aus: `chart_stats` für system.cpu, system.ram, \
system.load (after -30d), `query_timeseries` mit format summary für die Datenträger (section disk), `top_instances` für docker.mem und \
docker.cpu über 30 Tage. Zeige Trends, Spitzen und Reserven und empfiehl, ob und wo es eng wird."
                .to_string(),
        ),
        _ => return Err(invalid(format!("Unbekannter Prompt `{name}`."))),
    };
    Ok(json!({"description": description, "messages": [{"role": "user", "content": {"type": "text", "text": text}}]}))
}

// --- Vervollständigung ------------------------------------------------------------------------------

pub fn complete(core: &Core, params: &Value) -> Value {
    let arg = params.pointer("/argument/name").and_then(Value::as_str).unwrap_or_default();
    let value = params.pointer("/argument/value").and_then(Value::as_str).unwrap_or_default().to_lowercase();
    let mut values: Vec<String> = match arg {
        "id" => core.registry.list(false).into_iter().map(|c| c.id).collect(),
        "context" => {
            let mut v: Vec<String> = core.registry.list(false).into_iter().map(|c| c.context).collect();
            v.dedup();
            v
        }
        "section" => shared::catalog::SECTIONS.iter().map(|s| s.id.to_string()).collect(),
        "key" => core.snaps.keys(),
        "name" => core
            .snaps
            .get("docker.containers")
            .and_then(|s| {
                s.data.as_array().map(|a| a.iter().filter_map(|c| c.get("name").and_then(Value::as_str).map(String::from)).collect())
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    values.sort();
    values.dedup();
    values.retain(|v| v.to_lowercase().contains(&value));
    let total = values.len();
    values.truncate(100);
    json!({"values": values, "total": total, "hasMore": total > 100})
}
