//! Detailseite eines Containers: Kennzahlen, Diagramme, Eckdaten, Logs (mit Folgen), Prozesse, Ereignisse.

use dioxus::prelude::*;
use shared::{
    catalog::docker::*,
    docker::{Container as Ctr, ContainerDetails, LogLine, LogPage, ProcessList},
    events::{Event, EventPage, Severity},
};

use super::{use_snapshot, Segmented, bits, bps, bytes, pct, state_of, uptime};
use crate::{
    api,
    components::{
        chart::{ChartCard, section::ChartGrid},
        ui::{
            app_shell::{EmptyState, LoadingState, PageContainer, PageHeader, PageSection, PageWidth},
            button::{Button, ButtonSize, ButtonVariant},
            input::Input,
        },
        widgets::{Meter, StatTile, StatusDot, Tone},
    },
    live::use_events,
    route::Route,
};

#[component]
pub fn Container(name: String) -> Element {
    rsx! { ContainerView { key: "{name}", name } }
}

#[component]
fn ContainerView(name: String) -> Element {
    let list = use_snapshot::<Vec<Ctr>>("docker.containers");
    let n = name.clone();
    let details = use_resource(move || {
        let n = n.clone();
        async move { api::get::<ContainerDetails>(&format!("/docker/containers/{n}")).await }
    });
    let c = list.read().as_ref().and_then(|l| l.iter().find(|c| c.name == name).cloned());
    let Some(c) = c else {
        let loading = list.read().is_none();
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: name.clone(), eyebrow: "Container" }
                if loading {
                    LoadingState {}
                } else {
                    EmptyState { title: "Container nicht gefunden", description: "Er wurde entfernt oder umbenannt.", icon: rsx! { icons::Container {} },
                        Link { to: Route::Containers {}, class: "text-sm underline", "Zu allen Containern" }
                    }
                }
            }
        };
    };
    let (label, tone) = state_of(&c);
    let running = c.running();
    let cpu_hint = if c.cpu_limit > 0.0 { format!("Grenze {}", pct(c.cpu_limit * 100.0)) } else { "100 % = ein Kern".to_string() };
    let mem_pct = c.mem_pct();
    let id = |t: &shared::catalog::Tpl| t.chart_id(Some(&name));
    let app_name = c.app.clone();
    let multi = list.read().as_ref().is_some_and(|l| l.iter().filter(|x| x.app == c.app).count() > 1);

    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader {
                title: name.clone(),
                eyebrow: "Container",
                description: c.image.clone(),
                actions: rsx! {
                    span { class: "inline-flex items-center gap-2 rounded-md border px-2.5 py-1.5",
                        StatusDot { tone, label: label.clone() }
                    }
                    if multi {
                        Link { to: Route::AppDetail { name: app_name.clone() },
                            Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, icons::LayoutGrid {} "App {app_name}" }
                        }
                    }
                },
            }
            div { class: "grid grid-cols-[minmax(0,1fr)] gap-6",
                div { class: "grid grid-cols-2 gap-4 md:grid-cols-3 2xl:grid-cols-6",
                    StatTile { label: "CPU", value: if running { pct(c.cpu) } else { "–".into() }, hint: cpu_hint, icon: rsx! { icons::Cpu {} } }
                    StatTile {
                        label: "Speicher",
                        value: if running { bytes(c.mem) } else { "–".into() },
                        hint: if c.mem_limit > 0 { format!("von {} Grenze", bytes(c.mem_limit as f64)) } else { format!("mit Cache {}", bytes(c.mem_total)) },
                        icon: rsx! { icons::MemoryStick {} },
                        if let Some(p) = mem_pct {
                            Meter { value: p, warn: 85.0, crit: 95.0 }
                        }
                    }
                    StatTile {
                        label: "Netzwerk",
                        value: if c.host_net { "Host-Netz".to_string() } else if running { format!("↓ {}", bits(c.net_rx)) } else { "–".into() },
                        hint: if c.host_net { "keine eigenen Netzwerte".to_string() } else { format!("↑ {}", bits(c.net_tx)) },
                        icon: rsx! { icons::ArrowDownUp {} },
                    }
                    StatTile { label: "Datenträger", value: if running { bps(c.io_read + c.io_write) } else { "–".into() }, hint: "{bps(c.io_read)} lesen · {bps(c.io_write)} schreiben", icon: rsx! { icons::HardDrive {} } }
                    StatTile {
                        label: "Prozesse",
                        value: if running { c.pids.to_string() } else { "–".into() },
                        hint: if c.pids_limit > 0 { format!("Grenze {}", c.pids_limit) } else { "ohne Grenze".to_string() },
                        icon: rsx! { icons::ListTree {} },
                    }
                    StatTile {
                        label: if running { "Läuft seit" } else { "Beendet" },
                        value: uptime(&c),
                        hint: format!("{} Neustarts{}", c.restarts, if c.oom_kills > 0 || c.oom_killed { " · OOM" } else { "" }),
                        icon: rsx! { icons::Clock {} },
                    }
                }
                ChartGrid {
                    ChartCard { id: id(&DOCKER_CPU), height: 200 }
                    ChartCard { id: id(&DOCKER_MEM), height: 200 }
                    if !c.host_net {
                        ChartCard { id: id(&DOCKER_NET), height: 200 }
                    }
                    ChartCard { id: id(&DOCKER_IO), height: 200 }
                    ChartCard { id: id(&DOCKER_PIDS), height: 200 }
                    ChartCard { id: id(&DOCKER_THROTTLE), height: 200 }
                }
                div { class: "grid grid-cols-[minmax(0,1fr)] gap-6 2xl:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]",
                    Facts { c: c.clone(), details: details.read().clone().and_then(Result::ok) }
                    Logs { name: name.clone() }
                }
                div { class: "grid grid-cols-[minmax(0,1fr)] gap-6 xl:grid-cols-2",
                    Processes { name: name.clone(), running }
                    ContainerEvents { name: name.clone() }
                }
            }
        }
    }
}

#[component]
fn Row(label: String, children: Element) -> Element {
    rsx! {
        div { class: "grid grid-cols-[8rem_minmax(0,1fr)] gap-3 border-b py-2 text-sm last:border-0",
            dt { class: "text-muted-foreground", "{label}" }
            dd { class: "min-w-0 break-words", {children} }
        }
    }
}

/// Eckdaten aus Momentaufnahme und Details.
#[component]
fn Facts(c: Ctr, details: Option<ContainerDetails>) -> Element {
    let tab = use_signal(|| "facts".to_string());
    let d = details.unwrap_or_default();
    let full = |ts: i64| if ts > 0 { format!("{} ({})", crate::fmt::full(ts), crate::fmt::ago(ts)) } else { "–".to_string() };
    let cmdline = format!("{} {}", d.entrypoint.join(" "), d.cmd.join(" "));
    let limits = {
        let mut v = Vec::new();
        if c.cpu_limit > 0.0 {
            v.push(format!("CPU {} Kerne", shared::model::trim(c.cpu_limit, 2)));
        }
        if c.mem_limit > 0 {
            v.push(format!("Speicher {}", bytes(c.mem_limit as f64)));
        }
        if c.pids_limit > 0 {
            v.push(format!("{} Prozesse", c.pids_limit));
        }
        if v.is_empty() { "keine".to_string() } else { v.join(" · ") }
    };
    rsx! {
        PageSection {
            title: "Eckdaten",
            actions: rsx! {
                Segmented {
                    options: vec![("facts".into(), "Allgemein".into()), ("env".into(), "Umgebung".into()), ("labels".into(), "Labels".into())],
                    value: tab,
                }
            },
            match tab().as_str() {
                "env" => rsx! {
                    if d.env.is_empty() {
                        p { class: "text-sm text-muted-foreground", "Keine Umgebungsvariablen (oder noch nicht geladen)." }
                    }
                    dl { class: "max-h-[28rem] overflow-auto",
                        for (k, v) in d.env.iter() {
                            Row { key: "{k}", label: k.clone(), span { class: "font-mono text-xs", "{v}" } }
                        }
                    }
                    p { class: "mt-3 text-xs text-muted-foreground", "Werte mit pass, secret, token, key … im Namen sind maskiert." }
                },
                "labels" => rsx! {
                    dl { class: "max-h-[28rem] overflow-auto",
                        for (k, v) in d.labels.iter() {
                            div { key: "{k}", class: "border-b py-2 text-sm last:border-0",
                                p { class: "font-mono text-xs text-muted-foreground break-all", "{k}" }
                                p { class: "font-mono text-xs break-all", "{v}" }
                            }
                        }
                    }
                },
                _ => rsx! {
                    dl {
                        Row { label: "ID", span { class: "font-mono text-xs", "{c.id}" } }
                        Row { label: "Image", span { class: "break-all", "{c.image}" } }
                        Row { label: "App",
                            Link { to: Route::AppDetail { name: c.app.clone() }, class: "underline-offset-2 hover:underline", "{c.app}" }
                            if !c.service.is_empty() { span { class: "text-muted-foreground", " · Dienst {c.service}" } }
                        }
                        Row { label: "Erstellt", "{full(c.created)}" }
                        Row { label: if c.running() { "Gestartet" } else { "Beendet" }, if c.running() { "{full(c.started)}" } else { "{full(c.finished)} · Code {c.exit_code}" } }
                        Row { label: "Neustart", "{c.restart_policy} · {c.restarts} Neustarts" }
                        if !c.health.is_empty() {
                            Row { label: "Healthcheck",
                                "{c.health}"
                                for h in d.health_log.iter().take(1) {
                                    p { class: "mt-1 font-mono text-xs text-muted-foreground break-all", "Code {h.exit_code}: {h.output.trim()}" }
                                }
                            }
                        }
                        Row { label: "Netzwerk",
                            p { "{c.network_mode}" }
                            for n in c.networks.iter() {
                                p { key: "{n.name}", class: "text-xs text-muted-foreground",
                                    "{n.name}"
                                    if !n.ip.is_empty() { span { class: "font-mono", " · {n.ip}" } }
                                }
                            }
                        }
                        Row { label: "Ports",
                            if c.ports.is_empty() { "–" }
                            for p in c.ports.iter() {
                                p { class: "font-mono text-xs",
                                    if p.public > 0 { "{p.public} → {p.private}/{p.proto}" } else { span { class: "text-muted-foreground", "{p.private}/{p.proto} (intern)" } }
                                }
                            }
                        }
                        Row { label: "Mounts",
                            if c.mounts.is_empty() { "–" }
                            for m in c.mounts.iter() {
                                p { class: "text-xs break-all",
                                    span { class: "font-mono", "{m.destination}" }
                                    span { class: "text-muted-foreground", " ← {m.kind} {m.source}" }
                                    if !m.rw { span { class: "text-muted-foreground", " (nur lesen)" } }
                                }
                            }
                        }
                        Row { label: "Grenzen", "{limits}" }
                        if c.size_rw > 0 {
                            Row { label: "Schicht", "{bytes(c.size_rw as f64)} beschreibbar" }
                        }
                        if !d.cmd.is_empty() || !d.entrypoint.is_empty() {
                            Row { label: "Befehl", span { class: "font-mono text-xs break-all", "{cmdline}" } }
                        }
                        if !d.cgroup.is_empty() {
                            Row { label: "cgroup", span { class: "font-mono text-xs break-all text-muted-foreground", "{d.cgroup}" } }
                        }
                    }
                },
            }
        }
    }
}

/// Logs mit Folgen (alle 2 s neue Zeilen), Suche und Länge.
#[component]
fn Logs(name: String) -> Element {
    let mut lines = use_signal(Vec::<LogLine>::new);
    let mut last = use_signal(|| 0i64);
    let mut follow = use_signal(|| true);
    let tail = use_signal(|| "200".to_string());
    let mut filter = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut loading = use_signal(|| true);
    let n1 = name.clone();
    use_effect(move || {
        let t = tail();
        let n = n1.clone();
        loading.set(true);
        spawn(async move {
            match api::get::<LogPage>(&api::qs(&format!("/docker/containers/{n}/logs"), &[("tail", t)])).await {
                Ok(p) => {
                    lines.set(p.lines);
                    last.set(p.last);
                    error.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
            loading.set(false);
        });
    });
    let n2 = name.clone();
    use_future(move || {
        let n = n2.clone();
        async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(2000).await;
                if !follow() || loading() {
                    continue;
                }
                let since = last();
                let url = api::qs(&format!("/docker/containers/{n}/logs"), &[("tail", "500".into()), ("since", since.to_string())]);
                if let Ok(p) = api::get::<LogPage>(&url).await {
                    if !p.lines.is_empty() {
                        let mut l = lines.write();
                        l.extend(p.lines.into_iter().filter(|x| x.ts > since));
                        let over = l.len().saturating_sub(5000);
                        if over > 0 {
                            l.drain(..over);
                        }
                        drop(l);
                        last.set(p.last.max(since));
                    }
                }
            }
        }
    });
    // Beim Folgen ans Ende scrollen.
    use_effect(move || {
        let _ = lines.read().len();
        if follow() {
            if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("nrz-logbox")) {
                el.set_scroll_top(el.scroll_height());
            }
        }
    });
    let f = filter().to_lowercase();
    let all = lines.read();
    let shown: Vec<&LogLine> = all.iter().filter(|l| f.is_empty() || l.text.to_lowercase().contains(&f)).collect();
    let count = shown.len();
    let fmt_ts = |ns: i64| {
        let ms = ns / 1_000_000;
        format!("{}.{:03}", crate::fmt::clock(ms / 1000), ms % 1000)
    };
    rsx! {
        PageSection {
            title: "Logs",
            description: if follow() { format!("{count} Zeilen · folgt") } else { format!("{count} Zeilen") },
            actions: rsx! {
                Button {
                    variant: if follow() { ButtonVariant::Secondary } else { ButtonVariant::Outline },
                    size: ButtonSize::Sm,
                    onclick: move |_| follow.toggle(),
                    if follow() { icons::Pause {} "Anhalten" } else { icons::Play {} "Folgen" }
                }
            },
            div { class: "mb-3 flex flex-wrap items-center gap-2",
                div { class: "w-full sm:w-64",
                    Input { placeholder: "In Logs suchen", value: filter(), oninput: move |e: FormEvent| filter.set(e.value()) }
                }
                Segmented {
                    options: vec![("200".into(), "200".into()), ("1000".into(), "1000".into()), ("5000".into(), "5000".into())],
                    value: tail,
                }
            }
            if let Some(e) = error() {
                p { class: "text-sm text-destructive", "{e}" }
            }
            div {
                id: "nrz-logbox",
                class: "h-[28rem] overflow-auto rounded-lg border bg-background p-2 font-mono text-xs leading-5",
                if loading() && all.is_empty() {
                    LoadingState {}
                } else if shown.is_empty() {
                    p { class: "p-2 text-muted-foreground", "Keine Zeilen." }
                }
                for (i, l) in shown.iter().enumerate() {
                    div { key: "{l.ts}-{i}", class: "flex gap-3 whitespace-pre-wrap break-all hover:bg-muted/50",
                        span { class: "shrink-0 text-muted-foreground select-none", "{fmt_ts(l.ts)}" }
                        span { class: if l.err { "text-destructive" } else { "" }, "{l.text}" }
                    }
                }
            }
            p { class: "mt-2 text-xs text-muted-foreground", "Fehlerausgabe (stderr) ist rot markiert." }
        }
    }
}

/// Prozesse im Container (`docker top`), alle 5 s neu.
#[component]
fn Processes(name: String, running: bool) -> Element {
    let mut list = use_signal(|| None::<Result<ProcessList, String>>);
    use_future(move || {
        let n = name.clone();
        async move {
            loop {
                if running {
                    list.set(Some(api::get::<ProcessList>(&format!("/docker/containers/{n}/top")).await));
                }
                gloo_timers::future::TimeoutFuture::new(5000).await;
            }
        }
    });
    rsx! {
        PageSection { title: "Prozesse", description: "docker top – alle 5 Sekunden",
            if !running {
                p { class: "text-sm text-muted-foreground", "Der Container läuft nicht." }
            } else {
                match list() {
                    None => rsx! { LoadingState {} },
                    Some(Err(e)) => rsx! { p { class: "text-sm text-destructive", "{e}" } },
                    Some(Ok(p)) => rsx! {
                        div { class: "max-h-80 overflow-auto",
                            table { class: "w-full text-xs",
                                thead { class: "sticky top-0 border-b bg-card",
                                    tr {
                                        for (i, t) in p.titles.iter().enumerate() {
                                            th { key: "{i}", class: "h-8 px-2 text-left font-medium whitespace-nowrap text-muted-foreground", "{t}" }
                                        }
                                    }
                                }
                                tbody {
                                    for (k, r) in p.rows.iter().enumerate() {
                                        tr { key: "{k}", class: "border-b last:border-0",
                                            for (i, v) in r.iter().enumerate() {
                                                td { key: "{i}", class: if i + 1 == r.len() { "px-2 py-1 font-mono break-all" } else { "px-2 py-1 font-mono whitespace-nowrap tabular-nums" }, "{v}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}

pub fn severity_tone(s: Severity) -> Tone {
    match s {
        Severity::Crit => Tone::Bad,
        Severity::Warn => Tone::Warn,
        Severity::Ok => Tone::Good,
        Severity::Info => Tone::Neutral,
    }
}

/// Ereignisliste (neueste zuerst).
#[component]
pub fn EventList(events: Vec<Event>) -> Element {
    rsx! {
        if events.is_empty() {
            p { class: "text-sm text-muted-foreground", "Keine Ereignisse." }
        }
        ul { class: "max-h-80 divide-y overflow-auto",
            for e in events {
                li { key: "{e.id}-{e.ts}", class: "flex items-start gap-3 py-2 text-sm",
                    span { class: "mt-0.5", StatusDot { tone: severity_tone(e.severity), label: "" } }
                    div { class: "min-w-0 flex-1",
                        p { class: "truncate", "{e.title}" }
                        if !e.message.is_empty() {
                            p { class: "truncate text-xs text-muted-foreground", "{e.message}" }
                        }
                    }
                    span { class: "shrink-0 text-xs text-muted-foreground tabular-nums", title: "{crate::fmt::full_ms(e.ts)}", "{crate::fmt::ago_ms(e.ts)}" }
                }
            }
        }
    }
}

#[component]
fn ContainerEvents(name: String) -> Element {
    let live = use_events();
    let n = name.clone();
    let past = use_resource(move || {
        let n = n.clone();
        async move { api::get::<EventPage>(&api::qs("/events", &[("subject", n), ("limit", "50".into())])).await }
    });
    let mut list: Vec<Event> = live.read().iter().filter(|e| e.subject == name || e.labels.get("container") == Some(&name)).cloned().collect();
    if let Some(Ok(p)) = past.read().as_ref() {
        for e in &p.events {
            if !list.iter().any(|x| x.id == e.id && x.ts == e.ts) {
                list.push(e.clone());
            }
        }
    }
    list.sort_by(|a, b| b.ts.cmp(&a.ts));
    list.truncate(50);
    rsx! {
        PageSection { title: "Ereignisse", description: "Start, Ende, Health, OOM und Alarme",
            EventList { events: list }
        }
    }
}
