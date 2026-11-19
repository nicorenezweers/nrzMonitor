//! Werkzeuge des MCP-Servers: alles, was nrzMonitor misst und weiss, abfragbar.

use std::collections::{BTreeMap, HashMap};

use axum::http::{Method, StatusCode};
use serde_json::{Map, Value, json};
use shared::{
    ChartMeta,
    catalog::SECTIONS,
    mcp::{McpSettings, McpToolInfo},
    query::{Agg, QueryReq},
};
use tokio::sync::broadcast::error::RecvError;

use super::{Ctx, call_api};
use crate::{core::Core, util};

struct Def {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    write: bool,
    schema: Value,
}

/// Gemeinsame Felder, um Diagramme auszuwählen.
fn selection() -> Map<String, Value> {
    let v = json!({
        "charts": {"type": ["array", "string"], "items": {"type": "string"},
            "description": "Diagramm-IDs (Liste oder mit Komma), `*` als Platzhalter, z. B. `system.cpu`, `docker.cpu.*`"},
        "section": {"type": "string", "description": "Bereich: system, cpu, memory, disk, network, sensors, power, processes, docker, apps, dns, cloudflare, netbird, probes, monitor"},
        "context": {"type": "string", "description": "Kontext (Vorlage), z. B. `disk.io`, `docker.mem`, `net.traffic`"},
        "family": {"type": "string", "description": "Untergruppe (Gerät, Schnittstelle, Container …)"},
        "label": {"type": ["string", "object"], "description": "Label-Filter `key=value` (z. B. `container=nginx`, `app=nextcloud`) oder Objekt"},
        "search": {"type": "string", "description": "Freitext in ID, Titel, Familie und Labels"},
        "include_retired": {"type": "boolean", "description": "Auch Diagramme, die nichts mehr liefern"}
    });
    v.as_object().cloned().unwrap_or_default()
}

const TIME: &str = "Unix-Sekunden, ISO-Datum (`2026-11-19T08:00:00Z`) oder relativ wie `-15m`, `-1h`, `-7d` (0/`now` = jetzt). \
`after` relativ zählt ab `before`.";

fn obj(props: Value, extra_sel: bool) -> Value {
    let mut p = if extra_sel { selection() } else { Map::new() };
    if let Value::Object(m) = props {
        p.extend(m);
    }
    json!({"type": "object", "properties": p})
}

fn time_props() -> Value {
    json!({
        "after": {"type": ["integer", "string"], "description": format!("Beginn. {TIME} Standard -1h.")},
        "before": {"type": ["integer", "string"], "description": format!("Ende. {TIME} Standard jetzt.")}
    })
}

fn merge(a: Value, b: Value) -> Value {
    let mut m = a.as_object().cloned().unwrap_or_default();
    m.extend(b.as_object().cloned().unwrap_or_default());
    Value::Object(m)
}

fn defs() -> Vec<Def> {
    vec![
        Def {
            name: "overview",
            title: "Überblick",
            write: false,
            description: "Zustand auf einen Blick: Host, wichtigste Live-Kennzahlen (CPU, RAM, Last, Netz, Datenträger, Temperatur), \
Container nach Zustand, aktive Alarme, letzte Warnungen, Speicher und Zustand von nrzMonitor.",
            schema: obj(json!({}), false),
        },
        Def {
            name: "list_sections",
            title: "Bereiche",
            write: false,
            description: "Alle Bereiche (Menü) mit Anzahl Diagramme und Kontexten.",
            schema: obj(json!({}), false),
        },
        Def {
            name: "list_charts",
            title: "Diagramme finden",
            write: false,
            description: "Katalog der Diagramme (Zeitreihen) mit ID, Titel, Einheit, Dimensionen und Labels. Filtert nach Bereich, Kontext, Label oder Freitext.",
            schema: obj(
                json!({
                    "limit": {"type": "integer", "description": "Höchstens so viele (Standard 200, max 5000)"},
                    "offset": {"type": "integer"}
                }),
                true,
            ),
        },
        Def {
            name: "get_chart",
            title: "Diagramm",
            write: false,
            description: "Alles zu einem Diagramm: Beschreibung, Dimensionen, Labels, neueste Werte und Kennzahlen der letzten Stunde.",
            schema: json!({"type": "object", "properties": {"id": {"type": "string", "description": "Diagramm-ID"}}, "required": ["id"]}),
        },
        Def {
            name: "live_values",
            title: "Live-Werte",
            write: false,
            description: "Neueste Werte (Sekundenauflösung aus dem Arbeitsspeicher) der ausgewählten Diagramme, roh und lesbar formatiert. Ideal für Dashboards.",
            schema: obj(
                json!({"limit": {"type": "integer", "description": "Höchstens so viele Diagramme (Standard 100, max 2000)"}}),
                true,
            ),
        },
        Def {
            name: "query_timeseries",
            title: "Zeitreihen abfragen",
            write: false,
            description: "Verlauf der ausgewählten Diagramme über einen Zeitraum (Live-Speicher oder DuckDB, Stufe automatisch: Sekunden, Minuten, Stunden). \
Mit Min/Mittel/Max/Letzter je Dimension.",
            schema: obj(
                merge(
                    time_props(),
                    json!({
                        "points": {"type": "integer", "description": "Punkte je Reihe (Standard 120, max 5000)"},
                        "agg": {"type": "string", "enum": ["avg", "min", "max", "sum", "last"], "description": "Verdichtung je Punkt (Standard avg)"},
                        "envelope": {"type": "boolean", "description": "Min/Max je Punkt mitliefern"},
                        "format": {"type": "string", "enum": ["series", "rows", "summary"], "description": "series = je Dimension eine Liste (Standard), rows = Zeilen [ts, v1, v2 …], summary = nur Kennzahlen"},
                        "limit": {"type": "integer", "description": "Höchstens so viele Diagramme (Standard 20, max 500)"}
                    }),
                ),
                true,
            ),
        },
        Def {
            name: "chart_stats",
            title: "Kennzahlen",
            write: false,
            description: "Min, Mittel, Max, Letzter, Perzentile und Streuung je Dimension über einen Zeitraum (direkt in DuckDB gerechnet).",
            schema: obj(
                merge(
                    time_props(),
                    json!({"limit": {"type": "integer", "description": "Höchstens so viele Diagramme (Standard 50, max 200)"}}),
                ),
                true,
            ),
        },
        Def {
            name: "top_instances",
            title: "Rangliste",
            write: false,
            description: "Rangliste der Instanzen eines Kontexts über einen Zeitraum, z. B. welche Container am meisten CPU (`docker.cpu`) oder RAM (`docker.mem`) brauchen, welche Platte am meisten schreibt (`disk.io`).",
            schema: merge(
                obj(
                    merge(
                        time_props(),
                        json!({
                            "context": {"type": "string", "description": "Kontext, z. B. docker.cpu, docker.mem, app.cpu, disk.io, net.traffic"},
                            "agg": {"type": "string", "enum": ["avg", "min", "max", "sum", "last"]},
                            "dim": {"type": "string", "description": "Nur diese Dimension (leer = Summe)"},
                            "limit": {"type": "integer"}
                        }),
                    ),
                    false,
                ),
                json!({"required": ["context"]}),
            ),
        },
        Def {
            name: "what_changed",
            title: "Was hat sich geändert?",
            write: false,
            description: "Vergleicht ein Zeitfenster mit dem Zeitraum davor und listet die Reihen, die sich am stärksten verändert haben – zur Ursachensuche bei Spitzen und Störungen.",
            schema: obj(
                merge(
                    time_props(),
                    json!({
                        "baseline_after": {"type": ["integer", "string"], "description": "Beginn des Vergleichszeitraums (Standard: viermal das Fenster davor)"},
                        "section": {"type": "string", "description": "Nur dieser Bereich"},
                        "limit": {"type": "integer"}, "points": {"type": "integer"}
                    }),
                ),
                false,
            ),
        },
        Def {
            name: "export_csv",
            title: "CSV-Export",
            write: false,
            description: "Zeitreihen als CSV (Zeit + eine Spalte je Dimension).",
            schema: obj(merge(time_props(), json!({"points": {"type": "integer", "description": "0 = volle Auflösung"}})), true),
        },
        Def {
            name: "events",
            title: "Ereignisse",
            write: false,
            description: "Ereignisse (Container gestartet/abgestürzt/OOM, Alarme, Peers, Erreichbarkeit …), neueste zuerst, filterbar.",
            schema: obj(
                merge(
                    time_props(),
                    json!({
                        "source": {"type": "string", "description": "docker, alert, system, dns, cloudflare, netbird, probe, monitor"},
                        "kind": {"type": "string", "description": "z. B. container.start, container.oom, alert.raised"},
                        "subject": {"type": "string", "description": "Betroffenes Objekt (Containername …)"},
                        "severity": {"type": "string", "enum": ["info", "ok", "warn", "crit"], "description": "Mindest-Schwere"},
                        "q": {"type": "string", "description": "Freitext"},
                        "limit": {"type": "integer", "description": "Standard 50, max 2000"}
                    }),
                ),
                false,
            ),
        },
        Def {
            name: "list_snapshots",
            title: "Momentaufnahmen",
            write: false,
            description: "Schlüssel aller Momentaufnahmen (Listen und Zustände: docker.containers, docker.images, alerts.active, system.info, netbird.peers, probes.status …) mit Alter und Grösse.",
            schema: obj(json!({}), false),
        },
        Def {
            name: "get_snapshot",
            title: "Momentaufnahme lesen",
            write: false,
            description: "Inhalt einer Momentaufnahme, bei Listen filterbar und auf Felder kürzbar.",
            schema: json!({"type": "object", "required": ["key"], "properties": {
                "key": {"type": "string"},
                "filter": {"type": "string", "description": "Nur Listeneinträge, die diesen Text enthalten"},
                "fields": {"type": ["array", "string"], "items": {"type": "string"}, "description": "Nur diese Felder je Eintrag"},
                "limit": {"type": "integer", "description": "Höchstens so viele Einträge (Standard 500)"}
            }}),
        },
        Def {
            name: "docker_containers",
            title: "Container",
            write: false,
            description: "Alle Container mit Zustand, Health, Neustarts, OOM, App, Image und aktueller CPU-/RAM-Nutzung. Filter nach Zustand, App, Health oder Text.",
            schema: obj(
                json!({
                    "state": {"type": "string", "description": "running, exited, restarting, paused …"},
                    "health": {"type": "string", "description": "healthy, unhealthy, starting"},
                    "app": {"type": "string"},
                    "search": {"type": "string"},
                    "live": {"type": "boolean", "description": "CPU/RAM/Netz mitliefern (Standard true)"},
                    "full": {"type": "boolean", "description": "Alle Felder statt der wichtigsten"},
                    "sort": {"type": "string", "enum": ["name", "cpu", "mem", "restarts"]}
                }),
                false,
            ),
        },
        Def {
            name: "docker_container",
            title: "Container im Detail",
            write: false,
            description: "Ein Container: Konfiguration (Image, Ports, Mounts, Netzwerke, Umgebung ohne Geheimnisse, Labels), Live-Werte aller seiner Diagramme und letzte Ereignisse.",
            schema: json!({"type": "object", "required": ["name"], "properties": {"name": {"type": "string", "description": "Containername oder ID"}}}),
        },
        Def {
            name: "docker_logs",
            title: "Container-Logs",
            write: false,
            description: "Letzte Logzeilen eines Containers, optional gefiltert.",
            schema: json!({"type": "object", "required": ["name"], "properties": {
                "name": {"type": "string"},
                "tail": {"type": "integer", "description": "Zeilen (Standard 200, max 5000)"},
                "since": {"type": "integer", "description": "Unix-Nanosekunden: nur neuere Zeilen (zum Folgen, Wert `last` der vorigen Antwort)"},
                "grep": {"type": "string", "description": "Nur Zeilen mit diesem Text (ohne Gross/Klein)"},
                "errors_only": {"type": "boolean", "description": "Nur stderr"}
            }}),
        },
        Def {
            name: "docker_processes",
            title: "Prozesse im Container",
            write: false,
            description: "Prozessliste eines Containers (wie `docker top`).",
            schema: json!({"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}}),
        },
        Def {
            name: "docker_resources",
            title: "Docker-Ressourcen",
            write: false,
            description: "Docker-Engine (info), Apps (Compose-Projekte), Images, Volumes oder Netzwerke.",
            schema: json!({"type": "object", "required": ["kind"], "properties": {
                "kind": {"type": "string", "enum": ["info", "apps", "images", "volumes", "networks"]},
                "filter": {"type": "string"}
            }}),
        },
        Def {
            name: "alerts",
            title: "Alarme",
            write: false,
            description: "Aktive Alarme und alle Alarmregeln mit Zustand.",
            schema: obj(json!({"include_rules": {"type": "boolean", "description": "Regeln mitliefern (Standard true)"}}), false),
        },
        Def {
            name: "system_info",
            title: "Host",
            write: false,
            description: "Host: Betriebssystem, Kernel, CPU, RAM, Laufzeit, Datenträger und Live-Werte aller Systemdiagramme.",
            schema: obj(json!({}), false),
        },
        Def {
            name: "dns",
            title: "DNS",
            write: false,
            description: "DNS-Anfragen des Netzwerks (über nrzNetwork): Liste, Toplisten, Verlauf, Clients oder Details zu einer Domain.",
            schema: obj(
                merge(
                    time_props(),
                    json!({
                        "action": {"type": "string", "enum": ["queries", "top", "timeline", "clients", "domain"]},
                        "client": {"type": "string"}, "name": {"type": "string", "description": "Domain"},
                        "qtype": {"type": "string"}, "kind": {"type": "string"}, "rcode": {"type": "string"}, "upstream": {"type": "string"},
                        "page": {"type": "integer"}, "per_page": {"type": "integer"}, "limit": {"type": "integer"}, "points": {"type": "integer"}
                    }),
                ),
                false,
            ),
        },
        Def {
            name: "integrations",
            title: "Netzwerk-Integrationen",
            write: false,
            description: "Zustand von DNS (nrzNetwork), Cloudflare, NetBird und den Erreichbarkeits-Prüfungen (ohne Zugangsdaten), optional mit NetBird-Peers.",
            schema: obj(json!({"include_peers": {"type": "boolean"}}), false),
        },
        Def {
            name: "power_summary",
            title: "Strom",
            write: false,
            description: "Stromverbrauch und Kosten: heute, gestern, Monat, Jahr, Verlauf je Stunde/Tag.",
            schema: obj(json!({"tz": {"type": "integer", "description": "Abstand zu UTC in Minuten (Standard 60)"}}), false),
        },
        Def {
            name: "storage_stats",
            title: "Speicher",
            write: false,
            description: "Belegung der Datenbank je Stufe, Bytes je Wert, Werte je Sekunde, Aufbewahrung.",
            schema: obj(json!({}), false),
        },
        Def {
            name: "watch_live",
            title: "Live mitlesen",
            write: false,
            description: "Liest die ausgewählten Diagramme für einige Sekunden live mit (jede Sekunde ein Wert). Mit Fortschritts-Token kommen die Werte schon während des Laufs als Fortschrittsmeldungen.",
            schema: obj(
                json!({
                    "seconds": {"type": "integer", "description": "Dauer (Standard 10, max 300)"},
                    "limit": {"type": "integer", "description": "Höchstens so viele Diagramme (Standard 20, max 200)"}
                }),
                true,
            ),
        },
        Def {
            name: "api_request",
            title: "REST-API",
            write: false,
            description: "Beliebiger Aufruf der REST-API von nrzMonitor (`/api/v1` + path), z. B. `/charts`, `/snap/docker.containers`, `/alerts/rules`, `/storage/settings`, `/power/summary`. \
Andere Methoden als GET nur, wenn ändernde Zugriffe erlaubt sind.",
            schema: json!({"type": "object", "required": ["path"], "properties": {
                "method": {"type": "string", "enum": ["GET", "POST", "PUT", "DELETE"]},
                "path": {"type": "string", "description": "Pfad unter /api/v1, z. B. /charts?section=docker"},
                "query": {"type": "object", "description": "Weitere Query-Parameter"},
                "body": {"description": "JSON-Inhalt für POST/PUT"}
            }}),
        },
    ]
}

pub fn list(s: &McpSettings) -> Vec<Value> {
    defs()
        .into_iter()
        .filter(|d| !d.write || s.allow_write)
        .map(|d| {
            json!({
                "name": d.name, "title": d.title, "description": d.description, "inputSchema": d.schema,
                "annotations": {"title": d.title, "readOnlyHint": !d.write, "openWorldHint": false, "idempotentHint": !d.write},
            })
        })
        .collect()
}

pub fn catalog() -> Vec<McpToolInfo> {
    defs()
        .into_iter()
        .map(|d| McpToolInfo { name: d.name.into(), title: d.title.into(), description: d.description.into(), write: d.write })
        .collect()
}

enum Out {
    Json(Value),
    Text(String),
}

/// Werkzeug ausführen; Fehler kommen als `isError` zurück (die KI kann sie lesen und es neu versuchen).
pub async fn call(ctx: &Ctx, name: &str, args: &Value) -> Value {
    match run(ctx, name, args).await {
        Ok(Out::Json(v)) => {
            let text = serde_json::to_string(&v).unwrap_or_default();
            let structured = if v.is_object() { v } else { json!({"result": v}) };
            json!({"content": [{"type": "text", "text": text}], "structuredContent": structured, "isError": false})
        }
        Ok(Out::Text(t)) => json!({"content": [{"type": "text", "text": t}], "isError": false}),
        Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
    }
}

type R = Result<Out, String>;

async fn run(ctx: &Ctx, name: &str, a: &Value) -> R {
    let core = &ctx.core;
    match name {
        "overview" => overview(ctx).await,
        "list_sections" => Ok(Out::Json(sections(core))),
        "list_charts" => list_charts(core, a),
        "get_chart" => get_chart(core, a).await,
        "live_values" => live_values(core, a),
        "query_timeseries" => query(core, a).await,
        "chart_stats" => {
            let charts = ids(&pick(core, a, 50, 200)?);
            let (after, before) = window(a)?;
            api(ctx, Method::POST, "/analyze/stats", Some(json!({"charts": charts, "after": after, "before": before}))).await
        }
        "top_instances" => {
            let (after, before) = window(a)?;
            let mut body = json!({"context": req_str(a, "context")?, "after": after, "before": before});
            copy(a, &mut body, &["agg", "dim", "limit"]);
            api(ctx, Method::POST, "/analyze/top", Some(body)).await
        }
        "what_changed" => {
            let (after, before) = window_default(a, -900)?;
            let mut body = json!({"after": after, "before": before});
            if let Some(b) = a.get("baseline_after") {
                body["baseline_after"] = json!(abs_time(core, b)?);
            }
            copy(a, &mut body, &["section", "limit", "points"]);
            api(ctx, Method::POST, "/analyze/correlate", Some(body)).await
        }
        "export_csv" => {
            let charts = ids(&pick(core, a, 20, 100)?).join(",");
            let (after, before) = window(a)?;
            let points = num(a, "points").unwrap_or(0);
            let path = format!("/export.csv?charts={}&after={after}&before={before}&points={points}", enc(&charts));
            let (status, body) = call_api(ctx.api(), Method::GET, &api_path(&path), None).await?;
            if !status.is_success() {
                return Err(error_text(status, &body));
            }
            Ok(Out::Text(String::from_utf8_lossy(&body).into_owned()))
        }
        "events" => events(ctx, a).await,
        "list_snapshots" => Ok(Out::Json(snapshots(core))),
        "get_snapshot" => get_snapshot(core, a),
        "docker_containers" => containers(core, a),
        "docker_container" => container(ctx, a).await,
        "docker_logs" => logs(ctx, a).await,
        "docker_processes" => api(ctx, Method::GET, &format!("/docker/containers/{}/top", enc(&req_str(a, "name")?)), None).await,
        "docker_resources" => {
            let kind = req_str(a, "kind")?;
            if !["info", "apps", "images", "volumes", "networks"].contains(&kind.as_str()) {
                return Err("`kind`: info, apps, images, volumes oder networks.".into());
            }
            get_snapshot(core, &json!({"key": format!("docker.{kind}"), "filter": a.get("filter")}))
        }
        "alerts" => {
            let active = api_json(ctx, Method::GET, "/alerts/active", None).await?;
            let mut out = json!({"active": active});
            if a.get("include_rules").and_then(Value::as_bool).unwrap_or(true) {
                out["rules"] = api_json(ctx, Method::GET, "/alerts/rules", None).await.unwrap_or(Value::Null);
            }
            Ok(Out::Json(out))
        }
        "system_info" => Ok(Out::Json(system_info(core))),
        "dns" => dns(ctx, a).await,
        "integrations" => {
            let view = api_json(ctx, Method::GET, "/integrations", None).await.unwrap_or(Value::Null);
            let mut out = json!({"settings": view});
            for key in ["dns.status", "cloudflare.status", "netbird.status", "probes.status"] {
                out[key] = snap(core, key).unwrap_or(Value::Null);
            }
            if bool_arg(a, "include_peers") {
                out["netbird.peers"] = snap(core, "netbird.peers").unwrap_or(Value::Null);
            }
            Ok(Out::Json(out))
        }
        "power_summary" => {
            api(ctx, Method::GET, &format!("/power/summary?tz={}", a.get("tz").and_then(Value::as_i64).unwrap_or(60)), None).await
        }
        "storage_stats" => api(ctx, Method::GET, "/storage/stats", None).await,
        "watch_live" => watch(ctx, a).await,
        "api_request" => api_request(ctx, a).await,
        _ => Err(format!("Unbekanntes Werkzeug `{name}`.")),
    }
}

// --- Hilfen -----------------------------------------------------------------------------------------

fn str_arg<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn req_str(a: &Value, k: &str) -> Result<String, String> {
    str_arg(a, k).map(String::from).ok_or_else(|| format!("`{k}` fehlt."))
}

fn bool_arg(a: &Value, k: &str) -> bool {
    a.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn num(a: &Value, k: &str) -> Option<u64> {
    a.get(k).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
}

/// Liste aus Array oder Komma-Text.
fn list_arg(a: &Value, k: &str) -> Vec<String> {
    match a.get(k) {
        Some(Value::Array(v)) => v.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        Some(Value::String(s)) => s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect(),
        _ => Vec::new(),
    }
}

fn copy(from: &Value, to: &mut Value, keys: &[&str]) {
    for k in keys {
        if let Some(v) = from.get(*k).filter(|v| !v.is_null()) {
            to[*k] = v.clone();
        }
    }
}

/// Prozent-Kodierung für Query-Parameter und Pfadteile.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn api_path(path: &str) -> String {
    format!("/api/v1{path}")
}

fn error_text(status: StatusCode, body: &[u8]) -> String {
    let msg = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| String::from_utf8_lossy(body).chars().take(500).collect());
    format!("Fehler {}: {msg}", status.as_u16())
}

async fn api_json(ctx: &Ctx, method: Method, path: &str, body: Option<Value>) -> Result<Value, String> {
    let (status, bytes) = call_api(ctx.api(), method, &api_path(path), body).await?;
    if !status.is_success() {
        return Err(error_text(status, &bytes));
    }
    if bytes.is_empty() {
        return Ok(json!({"ok": true}));
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Antwort nicht lesbar: {e}"))
}

async fn api(ctx: &Ctx, method: Method, path: &str, body: Option<Value>) -> R {
    api_json(ctx, method, path, body).await.map(Out::Json)
}

/// Tage seit 1970-01-01 (proleptischer gregorianischer Kalender).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Zeitangabe → Unix-Sekunden (> 0) oder relativ (≤ 0), wie es `QueryReq` erwartet.
pub fn parse_time(v: &Value) -> Result<i64, String> {
    if let Some(n) = v.as_i64() {
        // Millisekunden erkennen.
        return Ok(if n > 100_000_000_000 { n / 1000 } else { n });
    }
    if let Some(f) = v.as_f64() {
        return Ok(f as i64);
    }
    let s = v.as_str().ok_or("Zeit als Zahl oder Text angeben.")?.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("now") {
        return Ok(0);
    }
    if let Ok(n) = s.parse::<i64>() {
        return parse_time(&json!(n));
    }
    // Relativ: -15m, 1h, -7d, 30s, 2w
    let body = s.trim_start_matches(['-', '+']);
    if let Some(unit) = body.chars().last().filter(|c| c.is_ascii_alphabetic()) {
        let digits = &body[..body.len() - 1];
        if let Ok(n) = digits.trim().parse::<f64>() {
            let mult = match unit.to_ascii_lowercase() {
                's' => 1.0,
                'm' => 60.0,
                'h' => 3600.0,
                'd' => 86_400.0,
                'w' => 604_800.0,
                'y' => 31_536_000.0,
                _ => {
                    return Err(format!("Zeit `{s}` nicht verstanden (Einheit s, m, h, d, w, y)."));
                }
            };
            return Ok(-((n * mult) as i64));
        }
    }
    // ISO: YYYY-MM-DD[THH:MM[:SS]][Z]
    let s2 = s.trim_end_matches('Z').replace(' ', "T");
    let (date, time) = s2.split_once('T').unwrap_or((&s2, "00:00:00"));
    let d: Vec<i64> = date.split('-').filter_map(|x| x.parse().ok()).collect();
    let t: Vec<i64> = time.split(':').filter_map(|x| x.split('.').next().and_then(|x| x.parse().ok())).collect();
    if d.len() == 3 && !t.is_empty() {
        let secs = days_from_civil(d[0], d[1], d[2]) * 86_400 + t[0] * 3600 + t.get(1).unwrap_or(&0) * 60 + t.get(2).unwrap_or(&0);
        return Ok(secs);
    }
    Err(format!("Zeit `{s}` nicht verstanden."))
}

/// Zeitraum aus `after`/`before` (relativ erlaubt), Standard die letzte Stunde.
fn window(a: &Value) -> Result<(i64, i64), String> {
    window_default(a, -3600)
}

fn window_default(a: &Value, default_after: i64) -> Result<(i64, i64), String> {
    let after = a.get("after").filter(|v| !v.is_null()).map(parse_time).transpose()?.unwrap_or(default_after);
    let before = a.get("before").filter(|v| !v.is_null()).map(parse_time).transpose()?.unwrap_or(0);
    Ok((after, before))
}

fn abs_time(core: &Core, v: &Value) -> Result<i64, String> {
    let t = parse_time(v)?;
    Ok(if t <= 0 { core.store.now() + t } else { t })
}

fn abs_window(core: &Core, a: &Value, default_after: i64) -> Result<(i64, i64), String> {
    let (after, before) = window_default(a, default_after)?;
    crate::store::resolve(core.store.now(), after, before).map_err(|e| e.to_string())
}

/// Einfacher Platzhalter-Vergleich (`*` = beliebig viele Zeichen).
fn glob(pat: &str, s: &str) -> bool {
    if !pat.contains('*') {
        return pat == s;
    }
    let parts: Vec<&str> = pat.split('*').collect();
    let mut rest = s;
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() {
            continue;
        }
        if i == 0 {
            let Some(r) = rest.strip_prefix(p) else {
                return false;
            };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            let Some(pos) = rest.find(p) else {
                return false;
            };
            rest = &rest[pos + p.len()..];
        }
    }
    true
}

fn label_filter(a: &Value) -> Vec<(String, String)> {
    match a.get("label") {
        Some(Value::String(s)) => {
            s.split(',').filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect()
        }
        Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v.as_str().map_or_else(|| v.to_string(), String::from))).collect(),
        _ => Vec::new(),
    }
}

/// Diagramme nach den gemeinsamen Feldern auswählen.
pub(super) fn select(core: &Core, a: &Value) -> Vec<ChartMeta> {
    let ids = {
        let mut v = list_arg(a, "charts");
        v.extend(list_arg(a, "ids"));
        v.extend(list_arg(a, "id"));
        v
    };
    let mut list = core.registry.list(bool_arg(a, "include_retired"));
    if !ids.is_empty() {
        list.retain(|c| ids.iter().any(|p| glob(p, &c.id)));
    }
    if let Some(s) = str_arg(a, "section") {
        list.retain(|c| c.section == s);
    }
    if let Some(x) = str_arg(a, "context") {
        list.retain(|c| glob(x, &c.context));
    }
    if let Some(f) = str_arg(a, "family") {
        list.retain(|c| c.family == f);
    }
    for (k, v) in label_filter(a) {
        list.retain(|c| c.labels.get(&k).is_some_and(|x| glob(&v, x)));
    }
    if let Some(q) = str_arg(a, "search") {
        let q = q.to_lowercase();
        list.retain(|c| {
            c.id.to_lowercase().contains(&q)
                || c.title.to_lowercase().contains(&q)
                || c.family.to_lowercase().contains(&q)
                || c.labels.values().any(|v| v.to_lowercase().contains(&q))
        });
    }
    for c in &mut list {
        c.last = c.dims.iter().filter_map(|d| core.ring.last_ts(d.series)).max().unwrap_or(c.last);
    }
    list
}

fn has_selection(a: &Value) -> bool {
    ["charts", "ids", "id", "section", "context", "family", "label", "search"].iter().any(|k| a.get(*k).is_some_and(|v| !v.is_null()))
}

/// Auswahl mit Pflicht-Filter und Obergrenze.
fn pick(core: &Core, a: &Value, default: u64, max: u64) -> Result<Vec<ChartMeta>, String> {
    if !has_selection(a) {
        return Err("Diagramme auswählen: `charts`, `section`, `context`, `label` oder `search` (siehe list_charts).".into());
    }
    let mut list = select(core, a);
    if list.is_empty() {
        return Err("Keine passenden Diagramme – list_charts zeigt, was es gibt.".into());
    }
    list.truncate(num(a, "limit").unwrap_or(default).clamp(1, max) as usize);
    Ok(list)
}

fn ids(list: &[ChartMeta]) -> Vec<String> {
    list.iter().map(|c| c.id.clone()).collect()
}

pub(super) fn compact(c: &ChartMeta) -> Value {
    let mut v = json!({
        "id": c.id, "title": c.title, "context": c.context, "section": c.section, "unit": c.unit,
        "dims": c.dims.iter().map(|d| if d.name == d.id { json!(d.id) } else { json!({"id": d.id, "name": d.name}) }).collect::<Vec<_>>(),
        "interval": c.interval, "last": c.last,
    });
    if !c.family.is_empty() {
        v["family"] = json!(c.family);
    }
    if !c.labels.is_empty() {
        v["labels"] = json!(c.labels);
    }
    if c.retired {
        v["retired"] = json!(true);
    }
    v
}

/// Neueste Werte eines Diagramms, roh und formatiert.
pub fn latest(core: &Core, c: &ChartMeta) -> Value {
    let (mut values, mut text) = (Map::new(), Map::new());
    let mut ts = 0;
    for d in &c.dims {
        match core.ring.latest(d.series) {
            Some((t, v)) => {
                ts = ts.max(t);
                values.insert(d.id.clone(), json!(v));
                text.insert(d.id.clone(), json!(c.unit.format(v)));
            }
            None => {
                values.insert(d.id.clone(), Value::Null);
            }
        }
    }
    let mut v = json!({"id": c.id, "title": c.title, "unit": c.unit, "ts": ts, "values": values, "text": text});
    if !c.labels.is_empty() {
        v["labels"] = json!(c.labels);
    }
    if ts > 0 {
        v["age_s"] = json!(util::now() - ts);
    }
    v
}

fn snap(core: &Core, key: &str) -> Option<Value> {
    core.snaps.get(key).map(|s| s.data.clone())
}

// --- Werkzeuge --------------------------------------------------------------------------------------

async fn overview(ctx: &Ctx) -> R {
    let core = &ctx.core;
    let now = util::now();
    let kpi_ids = [
        "system.cpu",
        "system.load",
        "system.ram",
        "system.swap",
        "system.net",
        "system.io",
        "system.pressure",
        "system.uptime",
        "system.processes",
        "cpu.temp",
        "docker.containers",
    ];
    let kpis: Vec<Value> = kpi_ids.iter().filter_map(|id| core.registry.get(id)).map(|c| latest(core, &c)).collect();

    let mut docker = json!(null);
    if let Some(Value::Array(list)) = snap(core, "docker.containers") {
        let mut by_state: BTreeMap<String, u64> = BTreeMap::new();
        let mut unhealthy = Vec::new();
        let mut restarting = Vec::new();
        let mut oom = Vec::new();
        for c in &list {
            let name = c.get("name").and_then(Value::as_str).unwrap_or_default();
            let state = c.get("state").and_then(Value::as_str).unwrap_or("unknown");
            *by_state.entry(state.to_string()).or_default() += 1;
            if c.get("health").and_then(Value::as_str) == Some("unhealthy") {
                unhealthy.push(name);
            }
            if state == "restarting" {
                restarting.push(name);
            }
            if c.get("oom_killed").and_then(Value::as_bool) == Some(true) {
                oom.push(name);
            }
        }
        // Grösste Verbraucher jetzt.
        let top = |ctx_name: &str| -> Vec<Value> {
            let mut v: Vec<(String, f64)> = core
                .registry
                .by_context(ctx_name)
                .iter()
                .filter(|c| !c.retired)
                .map(|c| {
                    let sum: f64 =
                        c.dims.iter().filter_map(|d| core.ring.latest(d.series)).filter(|(t, _)| now - t < 120).map(|(_, v)| v).sum();
                    (c.labels.get("container").cloned().unwrap_or_else(|| c.id.clone()), sum)
                })
                .collect();
            v.sort_by(|a, b| b.1.total_cmp(&a.1));
            v.into_iter().take(5).map(|(n, x)| json!({"container": n, "value": x})).collect()
        };
        docker = json!({
            "total": list.len(), "by_state": by_state, "unhealthy": unhealthy, "restarting": restarting, "oom_killed": oom,
            "top_cpu_percent": top("docker.cpu"), "top_mem_bytes": top("docker.mem"),
        });
    }
    let alerts = snap(core, "alerts.active").unwrap_or(json!([]));
    let recent = api_json(ctx, Method::GET, &format!("/events?severity=warn&limit=10&after={}", (now - 86_400) * 1000), None)
        .await
        .ok()
        .and_then(|v| v.get("events").cloned())
        .unwrap_or(json!([]));
    let storage = snap(core, "storage.stats").map(|s| {
        json!({"total_bytes": s.get("total_bytes"), "max_bytes": s.get("max_bytes"), "samples_per_sec": s.get("samples_per_sec"),
            "series": s.get("series"), "disk_free": s.get("disk_free")})
    });
    let (active, retired) = core.registry.counts();
    Ok(Out::Json(json!({
        "host": {"hostname": core.hostname, "info": snap(core, "system.info"), "demo": core.cfg.demo},
        "now": now,
        "kpis": kpis,
        "docker": docker,
        "alerts": {"count": alerts.as_array().map_or(0, Vec::len), "active": alerts},
        "recent_warnings": recent,
        "storage": storage,
        "monitor": {
            "version": env!("CARGO_PKG_VERSION"), "started_at": core.started_at, "uptime_s": now - core.started_at,
            "interval": core.cfg.interval, "charts": active, "charts_retired": retired,
            "collector_ms": &*core.durations.lock(), "live_clients": core.live_clients(),
            "mcp_sessions": super::hub().map_or(0, |h| h.sessions()),
        },
    })))
}

fn sections(core: &Core) -> Value {
    let list = core.registry.list(false);
    let out: Vec<Value> = SECTIONS
        .iter()
        .map(|s| {
            let charts: Vec<&ChartMeta> = list.iter().filter(|c| c.section == s.id).collect();
            let mut contexts: BTreeMap<&str, u64> = BTreeMap::new();
            for c in &charts {
                *contexts.entry(c.context.as_str()).or_default() += 1;
            }
            json!({"id": s.id, "title": s.title, "charts": charts.len(), "contexts": contexts})
        })
        .collect();
    json!({"sections": out})
}

fn list_charts(core: &Core, a: &Value) -> R {
    let list = select(core, a);
    let total = list.len();
    let offset = num(a, "offset").unwrap_or(0) as usize;
    let limit = num(a, "limit").unwrap_or(200).clamp(1, 5000) as usize;
    let charts: Vec<Value> = list.iter().skip(offset).take(limit).map(compact).collect();
    Ok(Out::Json(json!({"total": total, "offset": offset, "count": charts.len(), "charts": charts})))
}

async fn get_chart(core: &Core, a: &Value) -> R {
    let id = req_str(a, "id")?;
    let mut c = core.registry.get(&id).ok_or_else(|| format!("Diagramm `{id}` gibt es nicht."))?;
    c.last = c.dims.iter().filter_map(|d| core.ring.last_ts(d.series)).max().unwrap_or(c.last);
    let req = QueryReq { charts: vec![id.clone()], after: -3600, before: 0, points: 60, agg: Agg::Avg, envelope: false };
    let hour = core.store.query(&req).await.ok().and_then(|r| r.charts.into_iter().next()).map(|d| summarize(&d.dims, &d.values));
    Ok(Out::Json(json!({"chart": c, "latest": latest(core, &c), "last_hour": hour, "unit_label": c.unit.label()})))
}

fn live_values(core: &Core, a: &Value) -> R {
    let mut list = select(core, a);
    let total = list.len();
    let limit = num(a, "limit").unwrap_or(100).clamp(1, 2000) as usize;
    list.truncate(limit);
    let charts: Vec<Value> = list.iter().map(|c| latest(core, c)).collect();
    Ok(Out::Json(json!({"now": util::now(), "total": total, "count": charts.len(), "truncated": total > charts.len(), "charts": charts})))
}

/// Min/Mittel/Max/Letzter je Dimension.
fn summarize(dims: &[String], values: &[Vec<Option<f64>>]) -> Value {
    let mut out = Map::new();
    for (d, vals) in dims.iter().zip(values) {
        let v: Vec<f64> = vals.iter().flatten().copied().collect();
        if v.is_empty() {
            out.insert(d.clone(), Value::Null);
            continue;
        }
        let min = v.iter().copied().fold(f64::INFINITY, f64::min);
        let max = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let avg = v.iter().sum::<f64>() / v.len() as f64;
        out.insert(d.clone(), json!({"min": min, "avg": util::round(avg, 3), "max": max, "last": v.last(), "points": v.len()}));
    }
    Value::Object(out)
}

async fn query(core: &Core, a: &Value) -> R {
    let list = pick(core, a, 20, 500)?;
    let (after, before) = window(a)?;
    let agg = match str_arg(a, "agg").unwrap_or("avg") {
        "min" => Agg::Min,
        "max" => Agg::Max,
        "sum" => Agg::Sum,
        "last" => Agg::Last,
        _ => Agg::Avg,
    };
    let req = QueryReq {
        charts: ids(&list),
        after,
        before,
        points: num(a, "points").unwrap_or(120).clamp(2, 5000) as u32,
        agg,
        envelope: bool_arg(a, "envelope"),
    };
    let resp = core.store.query(&req).await.map_err(|e| e.to_string())?;
    let meta: HashMap<&str, &ChartMeta> = list.iter().map(|c| (c.id.as_str(), c)).collect();
    let format = str_arg(a, "format").unwrap_or("series");
    let charts: Vec<Value> = resp
        .charts
        .iter()
        .map(|d| {
            let m = meta.get(d.id.as_str());
            let mut v = json!({
                "id": d.id, "title": m.map(|m| m.title.clone()), "unit": m.map(|m| m.unit), "dims": d.dims,
                "start": d.start, "step": d.step, "tier": match d.tier { 255 => "ram", 0 => "seconds", 1 => "minutes", _ => "hours" },
                "summary": summarize(&d.dims, &d.values),
            });
            match format {
                "summary" => {}
                "rows" => {
                    let rows: Vec<Value> = (0..d.len())
                        .map(|i| {
                            let mut r = vec![json!(d.time(i))];
                            r.extend(d.values.iter().map(|s| json!(s.get(i).copied().flatten())));
                            Value::Array(r)
                        })
                        .collect();
                    v["columns"] = json!(std::iter::once("ts".to_string()).chain(d.dims.iter().cloned()).collect::<Vec<_>>());
                    v["rows"] = json!(rows);
                }
                _ => {
                    v["values"] = json!(d.values);
                    if !d.min.is_empty() {
                        v["min"] = json!(d.min);
                        v["max"] = json!(d.max);
                    }
                }
            }
            v
        })
        .collect();
    Ok(Out::Json(json!({"now": resp.now, "after": after, "before": before, "charts": charts})))
}

async fn events(ctx: &Ctx, a: &Value) -> R {
    let (after, before) = abs_window(&ctx.core, a, -86_400)?;
    let mut q = format!("/events?after={}&before={}&limit={}", after * 1000, before * 1000, num(a, "limit").unwrap_or(50).clamp(1, 2000));
    for k in ["source", "kind", "subject", "severity", "q"] {
        if let Some(v) = str_arg(a, k) {
            q.push_str(&format!("&{k}={}", enc(v)));
        }
    }
    api(ctx, Method::GET, &q, None).await
}

fn snapshots(core: &Core) -> Value {
    let now = util::now_ms();
    let list: Vec<Value> = core
        .snaps
        .keys()
        .into_iter()
        .filter_map(|k| {
            let s = core.snaps.get(&k)?;
            let size = match &s.data {
                Value::Array(a) => json!(a.len()),
                Value::Object(o) => json!(o.len()),
                _ => json!(1),
            };
            Some(json!({"key": k, "ts": s.ts, "age_s": (now - s.ts) / 1000, "entries": size}))
        })
        .collect();
    json!({"snapshots": list})
}

fn project(v: &Value, fields: &[String]) -> Value {
    match v {
        Value::Object(m) if !fields.is_empty() => {
            Value::Object(m.iter().filter(|(k, _)| fields.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        _ => v.clone(),
    }
}

fn get_snapshot(core: &Core, a: &Value) -> R {
    let key = req_str(a, "key")?;
    let Some(s) = core.snaps.get(&key) else {
        return Err(format!("Keine Momentaufnahme `{key}`. Vorhanden: {}", core.snaps.keys().join(", ")));
    };
    let fields = list_arg(a, "fields");
    let filter = str_arg(a, "filter").map(str::to_lowercase);
    let limit = num(a, "limit").unwrap_or(500) as usize;
    let data = match &s.data {
        Value::Array(list) => {
            let items: Vec<Value> = list
                .iter()
                .filter(|v| filter.as_ref().is_none_or(|f| v.to_string().to_lowercase().contains(f)))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|v| project(v, &fields))
                .collect();
            let total = items.len();
            json!({"total": total, "items": items.into_iter().take(limit).collect::<Vec<_>>()})
        }
        other => project(other, &fields),
    };
    Ok(Out::Json(json!({"key": key, "ts": s.ts, "data": data})))
}

/// Live-Werte je Container aus den Diagrammen eines Kontexts (Label `container`).
fn per_container(core: &Core, context: &str) -> HashMap<String, Value> {
    core.registry
        .by_context(context)
        .into_iter()
        .filter(|c| !c.retired)
        .filter_map(|c| {
            let name = c.labels.get("container")?.clone();
            let mut vals = Map::new();
            let mut total = 0.0;
            for d in &c.dims {
                if let Some((_, v)) = core.ring.latest(d.series) {
                    vals.insert(d.id.clone(), json!(v));
                    total += v;
                }
            }
            let mut out = json!({"total": util::round(total, 2), "text": c.unit.format(total)});
            if c.dims.len() > 1 {
                out["dims"] = Value::Object(vals);
            }
            Some((name, out))
        })
        .collect()
}

const CONTAINER_FIELDS: &[&str] =
    &["id", "name", "image", "app", "service", "state", "health", "status", "started", "finished", "exit_code", "restarts", "oom_killed"];

fn containers(core: &Core, a: &Value) -> R {
    let Some(Value::Array(list)) = snap(core, "docker.containers") else {
        return Err("Noch keine Containerliste (Docker-Socket erreichbar?).".into());
    };
    let live = a.get("live").and_then(Value::as_bool).unwrap_or(true);
    let (cpu, mem, net) = if live {
        (per_container(core, "docker.cpu"), per_container(core, "docker.mem"), per_container(core, "docker.net"))
    } else {
        Default::default()
    };
    let eq = |c: &Value, k: &str| {
        str_arg(a, k).is_none_or(|want| c.get(k).and_then(Value::as_str).is_some_and(|x| x.eq_ignore_ascii_case(want)))
    };
    let search = str_arg(a, "search").map(str::to_lowercase);
    let full = bool_arg(a, "full");
    let mut by_state: BTreeMap<String, u64> = BTreeMap::new();
    let mut out: Vec<Value> = list
        .iter()
        .filter(|c| eq(c, "state") && eq(c, "health") && eq(c, "app"))
        .filter(|c| search.as_ref().is_none_or(|q| c.to_string().to_lowercase().contains(q)))
        .map(|c| {
            let state = c.get("state").and_then(Value::as_str).unwrap_or("unknown").to_string();
            *by_state.entry(state).or_default() += 1;
            let mut v = if full {
                c.clone()
            } else {
                Value::Object(
                    c.as_object()
                        .map(|m| {
                            m.iter().filter(|(k, _)| CONTAINER_FIELDS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
                        })
                        .unwrap_or_default(),
                )
            };
            if live {
                let name = c.get("name").and_then(Value::as_str).unwrap_or_default();
                v["cpu_percent"] = cpu.get(name).cloned().unwrap_or(Value::Null);
                v["mem_bytes"] = mem.get(name).cloned().unwrap_or(Value::Null);
                v["net_bits"] = net.get(name).cloned().unwrap_or(Value::Null);
            }
            v
        })
        .collect();
    let key = |v: &Value, k: &str| v.pointer(k).and_then(Value::as_f64).unwrap_or(0.0);
    match str_arg(a, "sort").unwrap_or("name") {
        "cpu" => out.sort_by(|x, y| key(y, "/cpu_percent/total").total_cmp(&key(x, "/cpu_percent/total"))),
        "mem" => out.sort_by(|x, y| key(y, "/mem_bytes/total").total_cmp(&key(x, "/mem_bytes/total"))),
        "restarts" => out.sort_by(|x, y| key(y, "/restarts").total_cmp(&key(x, "/restarts"))),
        _ => out.sort_by(|x, y| x.get("name").and_then(Value::as_str).cmp(&y.get("name").and_then(Value::as_str))),
    }
    Ok(Out::Json(json!({"count": out.len(), "by_state": by_state, "containers": out})))
}

async fn container(ctx: &Ctx, a: &Value) -> R {
    let name = req_str(a, "name")?;
    let details = api_json(ctx, Method::GET, &format!("/docker/containers/{}", enc(&name)), None).await?;
    let real = details.pointer("/container/name").and_then(Value::as_str).unwrap_or(&name).to_string();
    let charts: Vec<Value> = ctx
        .core
        .registry
        .list(false)
        .iter()
        .filter(|c| c.labels.get("container").is_some_and(|x| *x == real))
        .map(|c| latest(&ctx.core, c))
        .collect();
    let events = api_json(ctx, Method::GET, &format!("/events?subject={}&limit=20", enc(&real)), None)
        .await
        .ok()
        .and_then(|v| v.get("events").cloned())
        .unwrap_or(json!([]));
    Ok(Out::Json(json!({"details": details, "live": charts, "events": events})))
}

async fn logs(ctx: &Ctx, a: &Value) -> R {
    let name = req_str(a, "name")?;
    let tail = num(a, "tail").unwrap_or(200).clamp(1, 5000);
    let since = a.get("since").and_then(Value::as_i64).unwrap_or(0);
    let mut page = api_json(ctx, Method::GET, &format!("/docker/containers/{}/logs?tail={tail}&since={since}", enc(&name)), None).await?;
    let grep = str_arg(a, "grep").map(str::to_lowercase);
    let errors = bool_arg(a, "errors_only");
    if let Some(Value::Array(lines)) = page.get_mut("lines") {
        lines.retain(|l| {
            (!errors || l.get("err").and_then(Value::as_bool) == Some(true))
                && grep.as_ref().is_none_or(|g| l.get("text").and_then(Value::as_str).is_some_and(|t| t.to_lowercase().contains(g)))
        });
    }
    Ok(Out::Json(page))
}

fn system_info(core: &Core) -> Value {
    let charts: Vec<Value> = core
        .registry
        .list(false)
        .iter()
        .filter(|c| c.section == "system" || (c.section == "cpu" && c.family.is_empty()) || (c.section == "memory" && c.family.is_empty()))
        .map(|c| latest(core, c))
        .collect();
    let keys: Vec<String> = core.snaps.keys().into_iter().filter(|k| k.starts_with("system.")).collect();
    json!({
        "info": snap(core, "system.info"),
        "disks": snap(core, "system.disks"),
        "snapshots": keys,
        "charts": charts,
    })
}

async fn dns(ctx: &Ctx, a: &Value) -> R {
    let action = str_arg(a, "action").unwrap_or("top");
    if !["queries", "top", "timeline", "clients", "domain"].contains(&action) {
        return Err("`action`: queries, top, timeline, clients oder domain.".into());
    }
    let (from, to) = abs_window(&ctx.core, a, -86_400)?;
    let mut q = format!("/dns/{action}?from={from}&to={to}");
    for k in ["client", "name", "qtype", "kind", "rcode", "upstream"] {
        if let Some(v) = str_arg(a, k) {
            q.push_str(&format!("&{k}={}", enc(v)));
        }
    }
    for k in ["page", "per_page", "limit", "points"] {
        if let Some(v) = num(a, k) {
            q.push_str(&format!("&{k}={v}"));
        }
    }
    api(ctx, Method::GET, &q, None).await
}

async fn watch(ctx: &Ctx, a: &Value) -> R {
    let core = &ctx.core;
    let list = pick(core, a, 20, 200)?;
    let wanted: HashMap<String, &ChartMeta> = list.iter().map(|c| (c.id.clone(), c)).collect();
    let secs = num(a, "seconds").unwrap_or(10).clamp(1, 300);
    let mut rx = core.subscribe_ticks();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
    // Takte je Zeitpunkt sammeln (mehrere Sammler liefern zum selben Zeitpunkt).
    let mut samples: BTreeMap<i64, Map<String, Value>> = BTreeMap::new();
    let mut sent = 0u64;
    loop {
        let frame = tokio::select! {
            f = rx.recv() => f,
            () = tokio::time::sleep_until(deadline) => break,
        };
        let frame = match frame {
            Ok(f) => f,
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => break,
        };
        let mut c = Map::new();
        for row in &frame.rows {
            if wanted.contains_key(&*row.chart.id) {
                let vals: Vec<Option<f64>> = frame.values_of(row).iter().map(|v| v.is_finite().then_some(*v)).collect();
                c.insert(row.chart.id.to_string(), json!(vals));
            }
        }
        if c.is_empty() {
            continue;
        }
        sent += 1;
        let elapsed = secs.saturating_sub(deadline.saturating_duration_since(tokio::time::Instant::now()).as_secs());
        ctx.progress(elapsed, Some(secs), json!({"t": frame.ts, "c": c}).to_string()).await;
        samples.entry(frame.ts).or_default().extend(c);
    }
    let charts: Vec<Value> = list
        .iter()
        .map(|c| json!({"id": c.id, "title": c.title, "unit": c.unit, "dims": c.dims.iter().map(|d| d.id.clone()).collect::<Vec<_>>()}))
        .collect();
    let samples: Vec<Value> = samples.into_iter().map(|(t, c)| json!({"t": t, "c": c})).collect();
    Ok(Out::Json(json!({"seconds": secs, "ticks": sent, "charts": charts, "samples": samples})))
}

async fn api_request(ctx: &Ctx, a: &Value) -> R {
    let method = str_arg(a, "method").unwrap_or("GET").to_ascii_uppercase();
    let method = match method.as_str() {
        "GET" => Method::GET,
        "POST" => Method::POST,
        "PUT" => Method::PUT,
        "DELETE" => Method::DELETE,
        _ => return Err("`method`: GET, POST, PUT oder DELETE.".into()),
    };
    if method != Method::GET && !ctx.settings.allow_write {
        return Err("Ändernde Aufrufe sind aus (Einstellungen → MCP → „Ändern erlauben“ oder NRZMON_MCP_WRITE=1).".into());
    }
    let mut path = req_str(a, "path")?;
    path = path.trim_start_matches("/api/v1").to_string();
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    let bare = path.split('?').next().unwrap_or_default();
    if bare.contains("..") || ["/live", "/login", "/logout"].contains(&bare) {
        return Err(format!("`{bare}` ist über MCP nicht erreichbar."));
    }
    if let Some(Value::Object(q)) = a.get("query") {
        for (k, v) in q {
            let v = v.as_str().map_or_else(|| v.to_string(), String::from);
            path.push(if path.contains('?') { '&' } else { '?' });
            path.push_str(&format!("{}={}", enc(k), enc(&v)));
        }
    }
    let body = a.get("body").filter(|b| !b.is_null()).cloned();
    let (status, bytes) = call_api(ctx.api(), method, &api_path(&path), body).await?;
    if !status.is_success() {
        return Err(error_text(status, &bytes));
    }
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(v) => Ok(Out::Json(json!({"status": status.as_u16(), "data": v}))),
        Err(_) if bytes.is_empty() => Ok(Out::Json(json!({"status": status.as_u16()}))),
        Err(_) => {
            let text = String::from_utf8_lossy(&bytes);
            Ok(Out::Text(text.chars().take(500_000).collect()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time(&json!("-15m")), Ok(-900));
        assert_eq!(parse_time(&json!("1h")), Ok(-3600));
        assert_eq!(parse_time(&json!("now")), Ok(0));
        assert_eq!(parse_time(&json!(1_700_000_000)), Ok(1_700_000_000));
        assert_eq!(parse_time(&json!(1_700_000_000_000_i64)), Ok(1_700_000_000));
        assert_eq!(parse_time(&json!("2023-11-14T22:13:20Z")), Ok(1_700_000_000));
        assert_eq!(parse_time(&json!("1970-01-02")), Ok(86_400));
        assert!(parse_time(&json!("bald")).is_err());
    }

    #[test]
    fn globs() {
        assert!(glob("docker.cpu.*", "docker.cpu.nginx"));
        assert!(glob("*.cpu.*", "docker.cpu.nginx"));
        assert!(glob("*nginx", "docker.cpu.nginx"));
        assert!(!glob("docker.mem.*", "docker.cpu.nginx"));
        assert!(glob("system.cpu", "system.cpu"));
        assert!(!glob("system.cpu", "system.cpus"));
    }

    #[test]
    fn encoding() {
        assert_eq!(enc("a b/c"), "a%20b%2Fc");
    }
}
