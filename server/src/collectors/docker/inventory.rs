//! Bestand über die Docker-API: Container (Inspect), Images, Volumes, Netzwerke, Info.
//! Aktuell gehalten über den Ereignisstrom; wichtige Ereignisse gehen als `core.event` hinaus.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::{Duration, Instant},
};

use serde_json::Value;
use shared::{
    docker::{Container, DockerInfo, Image, Mount, NetAttach, Network, Port, Volume},
    events::{Event, Severity},
};

use super::{
    Entry, Shared,
    api::{Docker, enc, parse_ts},
};
use crate::core::Core;

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn i(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

fn strs(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// Image-Name ohne angehängten Digest (`app:1.2@sha256:…` → `app:1.2`).
pub fn short_image(img: &str) -> String {
    let base = img.split('@').next().unwrap_or(img);
    if base.starts_with("sha256:") { base.chars().take(19).collect() } else { base.to_string() }
}

/// Inspect-Antwort → Eintrag. `list` = passender Eintrag aus `/containers/json` (für den Statustext).
pub fn parse_inspect(v: &Value, list: Option<&Value>) -> Entry {
    let full_id = s(v, "Id");
    let state = v.get("State").cloned().unwrap_or(Value::Null);
    let config = v.get("Config").cloned().unwrap_or(Value::Null);
    let host = v.get("HostConfig").cloned().unwrap_or(Value::Null);
    let labels: BTreeMap<String, String> = config
        .get("Labels")
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default();
    let name = s(v, "Name").trim_start_matches('/').to_string();
    let project = labels.get("com.docker.compose.project").cloned().unwrap_or_default();
    let network_mode = s(&host, "NetworkMode");
    let host_net = network_mode == "host" || network_mode.starts_with("container:");
    let mut ports = Vec::new();
    if let Some(map) = v.pointer("/NetworkSettings/Ports").and_then(Value::as_object) {
        for (key, binds) in map {
            let (port, proto) = key.split_once('/').unwrap_or((key, "tcp"));
            let private = port.parse().unwrap_or(0);
            match binds.as_array() {
                Some(list) if !list.is_empty() => {
                    for b in list {
                        ports.push(Port {
                            ip: s(b, "HostIp"),
                            private,
                            public: s(b, "HostPort").parse().unwrap_or(0),
                            proto: proto.to_string(),
                        });
                    }
                }
                _ => ports.push(Port { private, proto: proto.to_string(), ..Port::default() }),
            }
        }
    }
    ports.sort_by_key(|p| (p.public == 0, p.private, p.ip.clone()));
    // IPv4 und IPv6 derselben Veröffentlichung nur einmal zeigen.
    ports.dedup_by(|a, b| a.private == b.private && a.public == b.public && a.proto == b.proto);
    let mounts = v
        .get("Mounts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|m| Mount {
                    kind: s(m, "Type"),
                    source: if s(m, "Type") == "volume" { s(m, "Name") } else { s(m, "Source") },
                    destination: s(m, "Destination"),
                    rw: m.get("RW").and_then(Value::as_bool).unwrap_or(true),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut networks: Vec<NetAttach> = v
        .pointer("/NetworkSettings/Networks")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(n, x)| NetAttach {
                    name: n.clone(),
                    ip: s(x, "IPAddress"),
                    ipv6: s(x, "GlobalIPv6Address"),
                    mac: s(x, "MacAddress"),
                })
                .collect()
        })
        .unwrap_or_default();
    networks.sort_by(|a, b| a.name.cmp(&b.name));
    let nano = i(&host, "NanoCpus");
    let (quota, period) = (i(&host, "CpuQuota"), i(&host, "CpuPeriod"));
    let cpu_limit = if nano > 0 {
        nano as f64 / 1e9
    } else if quota > 0 && period > 0 {
        quota as f64 / period as f64
    } else {
        0.0
    };
    let health = state.pointer("/Health/Status").and_then(Value::as_str).unwrap_or("").to_string();
    let exit_code = i(&state, "ExitCode");
    let oom_killed = state.get("OOMKilled").and_then(Value::as_bool).unwrap_or(false);
    let st = s(&state, "Status");
    let c = Container {
        id: full_id.chars().take(12).collect(),
        name: name.clone(),
        image: short_image(&s(&config, "Image")),
        app: if project.is_empty() { name } else { project },
        service: labels.get("com.docker.compose.service").cloned().unwrap_or_default(),
        status: list.map(|l| s(l, "Status")).unwrap_or_default(),
        created: parse_ts(&s(v, "Created")),
        started: parse_ts(&s(&state, "StartedAt")),
        finished: parse_ts(&s(&state, "FinishedAt")),
        exit_code,
        restarts: i(v, "RestartCount").max(0) as u32,
        oom_killed,
        // Ohne Vorgeschichte: Code 0 und die Signale von `docker stop` gelten als gewollt.
        expected_exit: matches!(exit_code, 0 | 130 | 143) || (exit_code == 137 && !oom_killed),
        restart_policy: host.pointer("/RestartPolicy/Name").and_then(Value::as_str).unwrap_or("").to_string(),
        ports,
        mounts,
        networks,
        network_mode,
        host_net,
        cpu_limit,
        mem_limit: i(&host, "Memory").max(0) as u64,
        pids_limit: i(&host, "PidsLimit").max(0) as u64,
        state: st,
        health,
        ..Container::default()
    };
    Entry {
        c,
        full_id,
        pid: i(&state, "Pid") as i32,
        tty: config.get("Tty").and_then(Value::as_bool).unwrap_or(false),
        image_id: s(v, "Image"),
        labels,
    }
}

/// Docker-Ereignis → Ereignis für die Zeitleiste (`None` = nicht wichtig genug).
pub fn describe(action: &str, attrs: &Value, name: &str, expected: bool) -> Option<(&'static str, Severity, String, String)> {
    let code: i64 = attrs.get("exitCode").and_then(Value::as_str).and_then(|c| c.parse().ok()).unwrap_or(0);
    Some(match action {
        "start" => ("container.start", Severity::Info, format!("{name} gestartet"), String::new()),
        "die" if code == 0 || expected => {
            ("container.stop", Severity::Info, format!("{name} beendet"), format!("Code {code}"))
        }
        "die" => (
            "container.die",
            if code == 137 { Severity::Crit } else { Severity::Warn },
            format!("{name} unerwartet beendet"),
            format!("Code {code}{}", if code == 137 { " (SIGKILL)" } else { "" }),
        ),
        "oom" => ("container.oom", Severity::Crit, format!("{name}: Speicher erschöpft (OOM)"), "Der Kernel hat einen Prozess beendet.".to_string()),
        "health_status: unhealthy" => ("container.unhealthy", Severity::Warn, format!("{name} ist ungesund"), "Healthcheck schlägt fehl.".to_string()),
        "health_status: healthy" => ("container.healthy", Severity::Ok, format!("{name} ist wieder gesund"), String::new()),
        "destroy" => ("container.destroy", Severity::Info, format!("{name} entfernt"), String::new()),
        "create" => ("container.create", Severity::Info, format!("{name} erstellt"), String::new()),
        "pause" => ("container.pause", Severity::Info, format!("{name} pausiert"), String::new()),
        "unpause" => ("container.unpause", Severity::Info, format!("{name} fortgesetzt"), String::new()),
        "rename" => ("container.rename", Severity::Info, format!("{name} umbenannt"), format!("vorher {}", attrs.get("oldName").and_then(Value::as_str).unwrap_or("").trim_start_matches('/'))),
        _ => return None,
    })
}

/// Endlosschleife: verbinden, Bestand laden, Ereignissen folgen; bei Fehlern nach kurzer Pause neu.
pub async fn run(core: Core, shared: Shared) {
    let docker = Docker::new(core.cfg.docker_sock.clone());
    let mut wait = 5;
    loop {
        match follow(&core, &shared, &docker).await {
            Ok(()) => wait = 5,
            Err(e) => {
                let msg = format!("{e:#}");
                let changed = {
                    let mut st = shared.write();
                    let changed = st.info.available || st.info.error.as_deref() != Some(msg.as_str());
                    st.info.available = false;
                    st.info.error = Some(msg.clone());
                    changed
                };
                if changed {
                    tracing::warn!("Docker: {msg}");
                    core.snapshot("docker.info", &shared.read().info);
                }
                wait = (wait * 2).min(60);
            }
        }
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
}

async fn follow(core: &Core, shared: &Shared, docker: &Docker) -> anyhow::Result<()> {
    refresh_info(core, shared, docker).await?;
    refresh_all(core, shared, docker).await?;
    refresh_lists(core, shared, docker).await;
    spawn_df(core.clone(), shared.clone(), docker.clone());

    let filters = enc(r#"{"type":["container","image","volume","network"]}"#);
    let mut body = docker.open(&format!("/events?filters={filters}")).await?;
    anyhow::ensure!(body.status == 200, "Ereignisstrom: Status {}", body.status);
    let mut pending = Vec::<u8>::new();
    let mut full = tokio::time::interval(Duration::from_secs(60));
    full.tick().await;
    let mut lists_dirty = false;
    let mut lists_timer = tokio::time::interval(Duration::from_secs(3));
    // Wann zuletzt `stop`/`kill` je Container kam (für „gewolltes“ Ende).
    let mut stopping: HashMap<String, Instant> = HashMap::new();
    loop {
        tokio::select! {
            chunk = body.chunk() => {
                let Some(chunk) = chunk? else { anyhow::bail!("Ereignisstrom beendet") };
                pending.extend_from_slice(&chunk);
                while let Some(p) = pending.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=p).collect();
                    let Ok(ev) = serde_json::from_slice::<Value>(&line) else { continue };
                    if handle_event(core, shared, docker, &ev, &mut stopping).await {
                        lists_dirty = true;
                    }
                }
            }
            _ = full.tick() => {
                refresh_all(core, shared, docker).await?;
                refresh_info(core, shared, docker).await?;
                stopping.retain(|_, t| t.elapsed() < Duration::from_secs(120));
            }
            _ = lists_timer.tick() => {
                if lists_dirty {
                    lists_dirty = false;
                    refresh_lists(core, shared, docker).await;
                }
            }
        }
    }
}

/// Ein Ereignis verarbeiten. `true` = Images/Volumes/Netzwerke neu laden.
async fn handle_event(core: &Core, shared: &Shared, docker: &Docker, ev: &Value, stopping: &mut HashMap<String, Instant>) -> bool {
    let kind = s(ev, "Type");
    let action = s(ev, "Action");
    if kind != "container" {
        return matches!(action.as_str(), "pull" | "delete" | "tag" | "untag" | "create" | "destroy" | "prune" | "connect" | "disconnect" | "remove");
    }
    if action.starts_with("exec_") || action == "top" || action == "attach" || action == "resize" || action == "copy" || action == "export" || action == "archive-path" || action == "extract-to-dir" {
        return false;
    }
    let Some(id) = ev.pointer("/Actor/ID").and_then(Value::as_str).map(str::to_string) else { return false };
    let attrs = ev.pointer("/Actor/Attributes").cloned().unwrap_or(Value::Null);
    let name = s(&attrs, "name");
    if action == "kill" || action == "stop" {
        stopping.insert(id.clone(), Instant::now());
    }
    let expected = stopping.get(&id).is_some_and(|t| t.elapsed() < Duration::from_secs(60));
    if let Some((k, sev, title, msg)) = describe(&action, &attrs, &name, expected) {
        let app = attrs.get("com.docker.compose.project").and_then(Value::as_str).unwrap_or(&name).to_string();
        let image = short_image(&s(&attrs, "image"));
        let mut e = Event::new("docker", k, sev, &name, title).message(msg).label("container", &name).label("app", app);
        if !image.is_empty() {
            e = e.label("image", image);
        }
        core.event(e);
    }
    if action == "destroy" {
        let mut st = shared.write();
        st.entries.retain(|e| e.full_id != id);
        st.version += 1;
        stopping.remove(&id);
        return true;
    }
    // Zustand neu lesen (Start, Ende, Health, Umbenennung …).
    match docker.json::<Value>(&format!("/containers/{id}/json")).await {
        Ok(v) => {
            let mut entry = parse_inspect(&v, None);
            if action == "die" {
                entry.c.expected_exit = entry.c.exit_code == 0 || expected;
            }
            let mut st = shared.write();
            match st.entries.iter_mut().find(|e| e.full_id == id) {
                Some(old) => {
                    entry.c.size_rw = old.c.size_rw;
                    entry.c.oom_kills = old.c.oom_kills;
                    if action != "die" && action != "start" {
                        entry.c.expected_exit = old.c.expected_exit;
                    }
                    *old = entry;
                }
                None => st.entries.push(entry),
            }
            st.entries.sort_by(|a, b| a.c.name.cmp(&b.c.name));
            st.version += 1;
        }
        Err(e) => tracing::debug!("Docker: Inspect {id}: {e:#}"),
    }
    action == "create" || action == "start"
}

/// Alle Container neu laden (beim Start und jede Minute zur Sicherheit).
async fn refresh_all(_core: &Core, shared: &Shared, docker: &Docker) -> anyhow::Result<()> {
    let list: Vec<Value> = docker.json("/containers/json?all=1").await?;
    let old: HashMap<String, (u64, u64, bool)> =
        shared.read().entries.iter().map(|e| (e.full_id.clone(), (e.c.size_rw, e.c.oom_kills, e.c.expected_exit))).collect();
    let mut entries = Vec::with_capacity(list.len());
    // Inspect parallel, aber begrenzt (der Socket verträgt das gut).
    let mut set = tokio::task::JoinSet::new();
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
    for item in list {
        let id = s(&item, "Id");
        let docker = docker.clone();
        let sem = sem.clone();
        set.spawn(async move {
            let _p = sem.acquire().await;
            docker.json::<Value>(&format!("/containers/{id}/json")).await.map(|v| parse_inspect(&v, Some(&item)))
        });
    }
    while let Some(r) = set.join_next().await {
        if let Ok(Ok(mut e)) = r {
            if let Some((size, ooms, expected)) = old.get(&e.full_id) {
                e.c.size_rw = *size;
                e.c.oom_kills = *ooms;
                if !e.c.running() {
                    e.c.expected_exit = *expected;
                }
            }
            entries.push(e);
        }
    }
    entries.sort_by(|a, b| a.c.name.cmp(&b.c.name));
    let mut st = shared.write();
    let same = st.entries.len() == entries.len()
        && st.entries.iter().zip(&entries).all(|(a, b)| a.c == b.c && a.pid == b.pid);
    if !same {
        st.entries = entries;
        st.version += 1;
    } else {
        // Statustexte („Up 3 hours“) trotzdem aktualisieren, ohne neue Generation.
        for (a, b) in st.entries.iter_mut().zip(entries) {
            a.c.status = b.c.status;
        }
    }
    Ok(())
}

async fn refresh_info(core: &Core, shared: &Shared, docker: &Docker) -> anyhow::Result<()> {
    let info: Value = docker.json("/info").await?;
    let version: Value = docker.json("/version").await.unwrap_or(Value::Null);
    let snap = {
        let mut st = shared.write();
        let d = &mut st.info;
        d.available = true;
        d.error = None;
        d.version = s(&version, "Version");
        d.api_version = s(&version, "ApiVersion");
        d.os = s(&info, "OperatingSystem");
        d.kernel = s(&info, "KernelVersion");
        d.cgroup_version = s(&info, "CgroupVersion");
        d.cgroup_driver = s(&info, "CgroupDriver");
        d.storage_driver = s(&info, "Driver");
        d.root_dir = s(&info, "DockerRootDir");
        d.ncpu = i(&info, "NCPU").max(0) as u32;
        d.mem_total = i(&info, "MemTotal").max(0) as u64;
        d.containers = i(&info, "Containers").max(0) as u32;
        d.running = i(&info, "ContainersRunning").max(0) as u32;
        d.images = i(&info, "Images").max(0) as u32;
        d.clone()
    };
    core.snapshot("docker.info", &snap);
    Ok(())
}

/// Images, Volumes und Netzwerke (nach passenden Ereignissen und beim Start).
async fn refresh_lists(core: &Core, shared: &Shared, docker: &Docker) {
    let (used_images, used_volumes, used_nets) = {
        let st = shared.read();
        let mut imgs: HashMap<String, u32> = HashMap::new();
        let mut vols: HashMap<String, (u32, String)> = HashMap::new();
        let mut nets: HashMap<String, u32> = HashMap::new();
        for e in &st.entries {
            *imgs.entry(e.image_id.clone()).or_default() += 1;
            for m in e.c.mounts.iter().filter(|m| m.kind == "volume") {
                let v = vols.entry(m.source.clone()).or_insert((0, e.c.app.clone()));
                v.0 += 1;
            }
            for n in &e.c.networks {
                *nets.entry(n.name.clone()).or_default() += 1;
            }
        }
        (imgs, vols, nets)
    };
    let sizes: HashMap<String, i64> = shared.read().volume_sizes.clone();
    if let Ok(list) = docker.json::<Vec<Value>>("/images/json").await {
        let mut images: Vec<Image> = list
            .iter()
            .map(|v| {
                let id = s(v, "Id");
                let mut tags = strs(v.get("RepoTags"));
                tags.retain(|t| t != "<none>:<none>");
                if tags.is_empty() {
                    tags = strs(v.get("RepoDigests")).iter().map(|d| short_image(d.split('@').next().unwrap_or(d))).collect();
                }
                Image {
                    containers: used_images.get(&id).copied().unwrap_or(0),
                    id: id.trim_start_matches("sha256:").chars().take(12).collect(),
                    tags,
                    size: i(v, "Size").max(0) as u64,
                    created: i(v, "Created"),
                }
            })
            .collect();
        images.sort_by_key(|x| std::cmp::Reverse(x.size));
        core.snapshot("docker.images", &images);
    }
    if let Ok(v) = docker.json::<Value>("/volumes").await {
        let mut vols: Vec<Volume> = v
            .get("Volumes")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|x| {
                        let name = s(x, "Name");
                        let labels = x.get("Labels");
                        let label_app = labels.and_then(|l| l.get("com.docker.compose.project")).and_then(Value::as_str).unwrap_or("");
                        let (n, app) = used_volumes.get(&name).cloned().unwrap_or_default();
                        Volume {
                            size: sizes.get(&name).copied().unwrap_or(-1),
                            driver: s(x, "Driver"),
                            mountpoint: s(x, "Mountpoint"),
                            containers: n,
                            app: if label_app.is_empty() { app } else { label_app.to_string() },
                            name,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        vols.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
        core.snapshot("docker.volumes", &vols);
    }
    if let Ok(list) = docker.json::<Vec<Value>>("/networks").await {
        let mut nets: Vec<Network> = list
            .iter()
            .map(|v| {
                let cfg = v.pointer("/IPAM/Config/0").cloned().unwrap_or(Value::Null);
                let name = s(v, "Name");
                Network {
                    id: s(v, "Id").chars().take(12).collect(),
                    containers: used_nets.get(&name).copied().unwrap_or(0),
                    driver: s(v, "Driver"),
                    scope: s(v, "Scope"),
                    subnet: s(&cfg, "Subnet"),
                    gateway: s(&cfg, "Gateway"),
                    internal: v.get("Internal").and_then(Value::as_bool).unwrap_or(false),
                    name,
                }
            })
            .collect();
        nets.sort_by(|a, b| b.containers.cmp(&a.containers).then_with(|| a.name.cmp(&b.name)));
        core.snapshot("docker.networks", &nets);
    }
}

/// Speicherplatz (`system/df`, teuer) alle 5 Minuten in eigener Aufgabe.
fn spawn_df(core: Core, shared: Shared, docker: Docker) {
    {
        let mut st = shared.write();
        if st.df_running {
            return;
        }
        st.df_running = true;
    }
    tokio::spawn(async move {
        let chart = core.chart(crate::registry::ChartDef::tpl(&shared::catalog::docker::DOCKER_IMAGES));
        loop {
            match docker.get("/system/df", Duration::from_secs(120)).await.and_then(|d| Ok(serde_json::from_slice::<Value>(&d)?)) {
                Ok(df) => {
                    let sum = |k: &str, f: &str| -> u64 {
                        df.get(k).and_then(Value::as_array).map_or(0, |a| a.iter().map(|x| i(x, f).max(0) as u64).sum())
                    };
                    let images = df.get("LayersSize").and_then(Value::as_i64).unwrap_or(0).max(0) as u64;
                    let containers = sum("Containers", "SizeRw");
                    let volumes: u64 = df
                        .get("Volumes")
                        .and_then(Value::as_array)
                        .map_or(0, |a| a.iter().map(|v| v.pointer("/UsageData/Size").and_then(Value::as_i64).unwrap_or(0).max(0) as u64).sum());
                    let cache = sum("BuildCache", "Size");
                    let mut f = crate::frame::Frame::new(crate::util::now());
                    f.put(&chart, &[images as f64, containers as f64, volumes as f64, cache as f64]);
                    core.submit(f);
                    let sizes: HashMap<String, u64> = df
                        .get("Containers")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().map(|c| (s(c, "Id"), i(c, "SizeRw").max(0) as u64)).collect())
                        .unwrap_or_default();
                    let vsizes: HashMap<String, i64> = df
                        .get("Volumes")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().map(|v| (s(v, "Name"), v.pointer("/UsageData/Size").and_then(Value::as_i64).unwrap_or(-1))).collect())
                        .unwrap_or_default();
                    let info = {
                        let mut st = shared.write();
                        for e in &mut st.entries {
                            if let Some(sz) = sizes.get(&e.full_id) {
                                e.c.size_rw = *sz;
                            }
                        }
                        st.volume_sizes = vsizes;
                        st.info.images_size = images;
                        st.info.containers_size = containers;
                        st.info.volumes_size = volumes;
                        st.info.build_cache_size = cache;
                        st.info.clone()
                    };
                    core.snapshot("docker.info", &info);
                    refresh_lists(&core, &shared, &docker).await;
                }
                Err(e) => tracing::debug!("Docker: system/df: {e:#}"),
            }
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });
}

/// Info-Momentaufnahme für die Beispieldaten und Tests.
pub fn demo_info() -> DockerInfo {
    DockerInfo {
        available: true,
        version: "27.3.1".into(),
        api_version: "1.47".into(),
        os: "umbrelOS 1.3 (Beispiel)".into(),
        kernel: "6.1.0-25-amd64".into(),
        cgroup_version: "2".into(),
        cgroup_driver: "systemd".into(),
        storage_driver: "overlay2".into(),
        root_dir: "/var/lib/docker".into(),
        ncpu: 4,
        mem_total: 16 * 1024 * 1024 * 1024,
        ..DockerInfo::default()
    }
}

/// Namen aller Container (für Tests und Diagnose).
pub fn names(entries: &[Entry]) -> HashSet<String> {
    entries.iter().map(|e| e.c.name.clone()).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_inspect() {
        let v = json!({
            "Id": "0123456789abcdef0123", "Name": "/nrz-git_web_1", "Created": "2024-05-01T10:00:00Z",
            "RestartCount": 2,
            "State": {"Status": "running", "Pid": 77, "ExitCode": 0, "OOMKilled": false,
                      "StartedAt": "2024-05-01T10:00:05Z", "FinishedAt": "0001-01-01T00:00:00Z",
                      "Health": {"Status": "healthy"}},
            "Config": {"Image": "nrz/git:1.2@sha256:abc", "Tty": false,
                       "Labels": {"com.docker.compose.project": "nrz-git", "com.docker.compose.service": "web"}},
            "HostConfig": {"NetworkMode": "nrz-git_default", "NanoCpus": 1500000000, "Memory": 536870912,
                           "RestartPolicy": {"Name": "unless-stopped"}},
            "NetworkSettings": {"Ports": {"80/tcp": [{"HostIp": "0.0.0.0", "HostPort": "8080"}, {"HostIp": "::", "HostPort": "8080"}], "22/tcp": null},
                                "Networks": {"nrz-git_default": {"IPAddress": "172.18.0.2", "MacAddress": "02:42"}}},
            "Mounts": [{"Type": "volume", "Name": "git-data", "Source": "/var/lib/docker/volumes/git-data/_data", "Destination": "/data", "RW": true}]
        });
        let e = parse_inspect(&v, None);
        assert_eq!(e.c.name, "nrz-git_web_1");
        assert_eq!(e.c.app, "nrz-git");
        assert_eq!(e.c.image, "nrz/git:1.2");
        assert_eq!(e.c.id, "0123456789ab");
        assert_eq!(e.c.started, 1_714_557_605);
        assert_eq!(e.c.finished, 0);
        assert_eq!(e.c.cpu_limit, 1.5);
        assert_eq!(e.c.mem_limit, 512 * 1024 * 1024);
        assert_eq!(e.c.ports.len(), 2, "IPv4/IPv6 zusammengefasst, unveröffentlichter Port bleibt");
        assert_eq!(e.c.ports[0].public, 8080);
        assert_eq!(e.c.mounts[0].source, "git-data");
        assert_eq!(e.c.networks[0].ip, "172.18.0.2");
        assert_eq!(e.pid, 77);
        assert!(!e.c.host_net);
        assert_eq!(e.c.restarts, 2);
    }

    #[test]
    fn describes_events() {
        let a = json!({"exitCode": "1"});
        assert_eq!(describe("die", &a, "x", false).map(|d| d.1), Some(Severity::Warn));
        assert_eq!(describe("die", &a, "x", true).map(|d| d.1), Some(Severity::Info));
        assert_eq!(describe("oom", &a, "x", false).map(|d| d.1), Some(Severity::Crit));
        assert!(describe("exec_start", &a, "x", false).is_none());
    }
}
