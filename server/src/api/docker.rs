//! API des Bereichs `docker` – nur lesend: Einzelheiten, Logs und Prozesse eines Containers.
//!
//! - `GET /docker/containers/{name}` → [`ContainerDetails`] (Umgebungsvariablen maskiert)
//! - `GET /docker/containers/{name}/logs?tail=200&since=<ns>` → [`LogPage`]
//! - `GET /docker/containers/{name}/top` → [`ProcessList`]

use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::Deserialize;
use serde_json::Value;
use shared::{
    alerts::MASK,
    docker::{Container, ContainerDetails, HealthLog, LogLine, LogPage, ProcessList},
};

use super::{ApiErr, ApiResult};
use crate::{
    collectors::docker::{
        self, Entry,
        api::{ApiStatus, Docker, demux, enc, parse_ts, parse_ts_ns},
        inventory,
    },
    core::Core,
};

pub fn routes() -> Router<Core> {
    Router::new()
        .route("/docker/containers/{name}", get(details))
        .route("/docker/containers/{name}/logs", get(logs))
        .route("/docker/containers/{name}/top", get(top))
}

/// Namen von Umgebungsvariablen mit Geheimnissen.
fn secret_name(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    ["pass", "secret", "token", "key", "pwd", "credential", "auth", "private", "salt", "cookie", "session"].iter().any(|w| k.contains(w))
}

/// Wert einer Umgebungsvariable maskieren: ganz bei geheimen Namen, sonst nur Passwörter in URLs
/// (`postgres://user:geheim@host` → `postgres://user:••••@host`).
pub fn mask_env(k: &str, v: &str) -> String {
    if v.is_empty() {
        return String::new();
    }
    if secret_name(k) {
        return MASK.to_string();
    }
    if let Some(scheme) = v.find("://") {
        let rest = &v[scheme + 3..];
        if let Some(at) = rest.find('@') {
            let creds = &rest[..at];
            if let Some(colon) = creds.find(':')
                && !creds.contains('/') {
                    return format!("{}{}:{MASK}{}", &v[..scheme + 3], &creds[..colon], &rest[at..]);
                }
        }
    }
    v.to_string()
}

/// Farb- und Steuercodes (`ESC[…m`) aus Logzeilen entfernen.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                // Bis zum abschliessenden Buchstaben überspringen.
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn entry(name: &str) -> Option<Entry> {
    let st = docker::shared()?.read();
    st.entries.iter().find(|e| e.c.name == name || e.c.id == name || e.full_id == name).cloned()
}

/// Container mit aktuellen Werten aus der Momentaufnahme.
fn current(core: &Core, name: &str) -> Option<Container> {
    let snap = core.snaps.get("docker.containers")?;
    let list: Vec<Container> = serde_json::from_value(snap.data.clone()).ok()?;
    list.into_iter().find(|c| c.name == name || c.id == name)
}

fn docker_err(e: anyhow::Error) -> ApiErr {
    match e.downcast_ref::<ApiStatus>() {
        Some(s) if s.status == 404 => ApiErr::not_found("Container nicht gefunden"),
        _ => ApiErr::from(e),
    }
}

/// Name für die Docker-API (Pfadteil).
fn target(name: &str) -> String {
    entry(name).map_or_else(|| enc(name), |e| e.full_id)
}

async fn details(State(core): State<Core>, Path(name): Path<String>) -> ApiResult<ContainerDetails> {
    if core.cfg.demo {
        let e = entry(&name).ok_or_else(|| ApiErr::not_found("Container nicht gefunden"))?;
        let c = current(&core, &name).unwrap_or_else(|| e.c.clone());
        let env = [
            ("TZ", "Europe/Zurich"),
            ("DATABASE_URL", "postgres://app:geheim123@db:5432/app"),
            ("POSTGRES_PASSWORD", "geheim123"),
            ("API_TOKEN", "abc"),
            ("LOG_LEVEL", "info"),
            ("PUID", "1000"),
        ];
        return Ok(Json(ContainerDetails {
            full_id: e.full_id.clone(),
            image_id: e.image_id.clone(),
            cmd: vec!["/app/server".into(), "--port".into(), "8080".into()],
            entrypoint: vec!["/sbin/tini".into(), "--".into()],
            working_dir: "/app".into(),
            user: "1000:1000".into(),
            hostname: c.id.clone(),
            env: env.iter().map(|(k, v)| ((*k).to_string(), mask_env(k, v))).collect(),
            labels: e.labels.clone(),
            healthcheck: if c.health.is_empty() { Vec::new() } else { vec!["CMD-SHELL".into(), "wget -qO- http://localhost:8080/health || exit 1".into()] },
            health_log: if c.health.is_empty() {
                Vec::new()
            } else {
                vec![HealthLog { start: crate::util::now() - 30, exit_code: i64::from(c.unhealthy()), output: if c.unhealthy() { "wget: server returned error: HTTP/1.1 503".into() } else { "ok".into() } }]
            },
            cgroup: docker::shared().and_then(|s| s.read().cgroups.get(&e.full_id).cloned()).unwrap_or_default(),
            container: c,
            ..ContainerDetails::default()
        }));
    }
    let d = Docker::new(core.cfg.docker_sock.clone());
    let v: Value = d.json(&format!("/containers/{}/json", target(&name))).await.map_err(docker_err)?;
    let parsed = inventory::parse_inspect(&v, None);
    let c = current(&core, &parsed.c.name).unwrap_or(parsed.c.clone());
    let cfg = v.get("Config").cloned().unwrap_or(Value::Null);
    let host = v.get("HostConfig").cloned().unwrap_or(Value::Null);
    let strs = |x: Option<&Value>| -> Vec<String> {
        x.and_then(Value::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    let sget = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let env = strs(cfg.get("Env"))
        .into_iter()
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((&kv, ""));
            (k.to_string(), mask_env(k, v))
        })
        .collect();
    let health_log = v
        .pointer("/State/Health/Log")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .rev()
                .take(5)
                .map(|h| HealthLog {
                    start: parse_ts(&sget(h, "Start")),
                    exit_code: h.get("ExitCode").and_then(Value::as_i64).unwrap_or(0),
                    output: sget(h, "Output").chars().take(1000).collect(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(ContainerDetails {
        full_id: parsed.full_id.clone(),
        image_id: parsed.image_id.clone(),
        cmd: strs(cfg.get("Cmd")),
        entrypoint: strs(cfg.get("Entrypoint")),
        working_dir: sget(&cfg, "WorkingDir"),
        user: sget(&cfg, "User"),
        hostname: sget(&cfg, "Hostname"),
        privileged: host.get("Privileged").and_then(Value::as_bool).unwrap_or(false),
        cap_add: strs(host.get("CapAdd")),
        env,
        labels: parsed.labels.clone(),
        healthcheck: strs(cfg.pointer("/Healthcheck/Test")),
        health_log,
        cgroup: docker::shared().and_then(|s| s.read().cgroups.get(&parsed.full_id).cloned()).unwrap_or_default(),
        container: c,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct LogParams {
    tail: usize,
    /// Unix-Nanosekunden: nur neuere Zeilen (zum Folgen).
    since: i64,
}

impl Default for LogParams {
    fn default() -> Self {
        Self { tail: 200, since: 0 }
    }
}

async fn logs(State(core): State<Core>, Path(name): Path<String>, Query(p): Query<LogParams>) -> ApiResult<LogPage> {
    let tail = p.tail.clamp(1, 5000);
    let lines = if core.cfg.demo {
        entry(&name).ok_or_else(|| ApiErr::not_found("Container nicht gefunden"))?;
        docker::demo::logs(&name, if p.since > 0 { 20 } else { tail }, p.since)
    } else {
        let tty = entry(&name).is_some_and(|e| e.tty);
        let mut path = format!("/containers/{}/logs?stdout=1&stderr=1&timestamps=1&tail={tail}", target(&name));
        if p.since > 0 {
            path.push_str(&format!("&since={}.{:09}", p.since / 1_000_000_000, p.since % 1_000_000_000));
        }
        let d = Docker::new(core.cfg.docker_sock.clone());
        let data = d.get(&path, Duration::from_secs(15)).await.map_err(docker_err)?;
        demux(&data, tty)
            .into_iter()
            .filter_map(|(err, line)| {
                let (ts, text) = line.split_once(' ').unwrap_or((&line, ""));
                let ts = parse_ts_ns(ts)?;
                (ts > p.since).then(|| LogLine { ts, err, text: strip_ansi(text).chars().take(4000).collect() })
            })
            .collect()
    };
    let last = lines.last().map_or(p.since, |l: &LogLine| l.ts);
    Ok(Json(LogPage { lines, last }))
}

async fn top(State(core): State<Core>, Path(name): Path<String>) -> ApiResult<ProcessList> {
    if core.cfg.demo {
        let e = entry(&name).ok_or_else(|| ApiErr::not_found("Container nicht gefunden"))?;
        if !e.c.running() {
            return Err(ApiErr::bad("Container läuft nicht"));
        }
        let row = |pid: i32, cmd: &str| vec!["1000".into(), pid.to_string(), "1".into(), "0".into(), "08:12".into(), "?".into(), "00:00:12".into(), cmd.to_string()];
        return Ok(Json(ProcessList {
            titles: ["UID", "PID", "PPID", "C", "STIME", "TTY", "TIME", "CMD"].iter().map(|s| (*s).to_string()).collect(),
            rows: vec![row(e.pid, "/sbin/tini -- /app/server --port 8080"), row(e.pid + 1, "/app/server --port 8080"), row(e.pid + 2, "/app/worker")],
        }));
    }
    let d = Docker::new(core.cfg.docker_sock.clone());
    let v: Value = d.json(&format!("/containers/{}/top", target(&name))).await.map_err(docker_err)?;
    let titles = v
        .get("Titles")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let rows = v
        .get("Processes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|r| r.as_array().map(|c| c.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect()))
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(ProcessList { titles, rows }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi() {
        assert_eq!(strip_ansi("\u{1b}[2m12:00\u{1b}[0m \u{1b}[34mDEBUG\u{1b}[0m x"), "12:00 DEBUG x");
    }

    #[test]
    fn masks_secrets() {
        assert_eq!(mask_env("POSTGRES_PASSWORD", "x"), MASK);
        assert_eq!(mask_env("api_key", "x"), MASK);
        assert_eq!(mask_env("TZ", "Europe/Zurich"), "Europe/Zurich");
        assert_eq!(mask_env("DATABASE_URL", "postgres://app:geheim@db:5432/app"), format!("postgres://app:{MASK}@db:5432/app"));
        assert_eq!(mask_env("URL", "https://example.com/a@b"), "https://example.com/a@b");
    }
}
