//! Übersicht: das Wichtigste des Hosts auf einen Blick – Eckdaten, Kennzahlen mit Mini-Verlauf,
//! die wichtigsten Verläufe, grösste Container, Dateisysteme, Alarme, Ereignisse und Netz-Dienste.
//! Jede Kachel verschwindet ohne Daten (oder zeigt einen ruhigen Leerzustand).

use dioxus::prelude::*;
use shared::{
    alerts::ActiveAlert,
    catalog::{dns::*, docker::*, netbird::*, probes::*, system::*},
    events::Severity,
    model::{ChartMeta, Unit},
    query::ChartData,
    system::HostInfo,
};

use crate::{
    components::{
        chart::{
            BarItem, BarList, ChartCard, ChartView,
            marks::{EVENTS, tone_of},
            section::ChartGrid,
            use_chart,
        },
        lazy::Lazy,
        ui::app_shell::{PageContainer, PageHeader, PageSection, PageWidth},
        widgets::{Meter, Sparkline, StatTile, StatusDot, Tone, dim, max_of, show, sum, use_latest},
    },
    fmt,
    live::use_snap,
    state::use_app,
};

#[component]
pub fn Overview() -> Element {
    let app = use_app();
    let has = |id: &str| app.chart(id).is_some_and(|c| !c.retired);
    let charts = app.charts.read();
    let filesystems: Vec<ChartMeta> = charts.iter().filter(|c| c.context == DISK_SPACE.context && !c.retired).cloned().collect();
    let probes: Vec<ChartMeta> = charts.iter().filter(|c| c.context == PROBE_UP.context && !c.retired).take(8).cloned().collect();
    drop(charts);
    let alerts = use_snap::<Vec<ActiveAlert>>("alerts.active");
    let containers = has(DOCKER_CPU_ALL.context) || has(DOCKER_MEM_ALL.context);
    let net_services = has(DNS_QUERIES.context) || !probes.is_empty() || has(NETBIRD_PEERS.context);
    let main_charts: Vec<(&str, u32)> = [
        (SYSTEM_CPU.context, 200),
        (SYSTEM_RAM.context, 200),
        (SYSTEM_NET.context, 200),
        (SYSTEM_IO.context, 200),
        (SYSTEM_LOAD.context, 170),
        (CPU_TEMP.context, 170),
        (SYSTEM_PRESSURE.context, 170),
    ]
    .into_iter()
    .filter(|(id, _)| has(id))
    .collect();

    rsx! {
        PageContainer { width: PageWidth::Full,
            HostHeader {}
            // Kennzahlen mit Mini-Verlauf
            div { class: "grid grid-cols-2 gap-3 sm:grid-cols-[repeat(auto-fit,minmax(190px,1fr))]",
                if has(SYSTEM_CPU.context) {
                    Kpi { kind: KpiKind::Cpu }
                }
                if has(SYSTEM_RAM.context) {
                    Kpi { kind: KpiKind::Mem }
                }
                if has(SYSTEM_LOAD.context) {
                    Kpi { kind: KpiKind::Load }
                }
                if has(CPU_TEMP.context) {
                    Kpi { kind: KpiKind::Temp }
                }
                if has(SYSTEM_NET.context) {
                    Kpi { kind: KpiKind::Net }
                }
                if has(SYSTEM_IO.context) {
                    Kpi { kind: KpiKind::Disk }
                }
                if has(DOCKER_CONTAINERS.context) {
                    Kpi { kind: KpiKind::Containers }
                }
                if has(DNS_QUERIES.context) {
                    Kpi { kind: KpiKind::Dns }
                }
            }

            if !main_charts.is_empty() {
                SectionTitle { title: "Verläufe", href: "/metrics", link: "Alle Metriken" }
                ChartGrid {
                    for (id, height) in main_charts {
                        ChartCard { key: "{id}", id: id.to_string(), height }
                    }
                }
            }

            div { class: "mt-6 grid gap-4 lg:grid-cols-2 2xl:grid-cols-3",
                if containers {
                    TopContainers {}
                }
                if !filesystems.is_empty() {
                    PageSection { title: "Dateisysteme", description: "Belegung je Einhängepunkt",
                        div { class: "grid gap-4",
                            for c in filesystems {
                                FsRow { key: "{c.id}", meta: c }
                            }
                        }
                    }
                }
                if let Some(list) = alerts() {
                    ActiveAlerts { list }
                }
                RecentEvents {}
            }

            if net_services {
                SectionTitle { title: "Netz-Dienste", href: "/dns", link: "DNS" }
                ChartGrid {
                    if has(DNS_QUERIES.context) {
                        ChartCard { id: DNS_QUERIES.context.to_string(), height: 160 }
                    }
                    if has(DNS_LATENCY.context) {
                        ChartCard { id: DNS_LATENCY.context.to_string(), height: 160 }
                    }
                    if has(NETBIRD_PEERS.context) {
                        ChartCard { id: NETBIRD_PEERS.context.to_string(), height: 160 }
                    }
                    if !probes.is_empty() {
                        Lazy { min_height: 200,
                            ProbeStates { probes }
                        }
                    }
                }
            }
        }
    }
}

/// Zwischenüberschrift mit Link rechts.
#[component]
fn SectionTitle(title: &'static str, href: &'static str, link: &'static str) -> Element {
    rsx! {
        div { class: "mt-8 mb-3 flex items-baseline justify-between gap-3",
            h2 { class: "text-xs font-medium tracking-wider text-muted-foreground uppercase", "{title}" }
            Link { to: href, class: "inline-flex items-center gap-1 rounded text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 [&_svg]:size-3.5",
                "{link}"
                icons::ChevronRight {}
            }
        }
    }
}

/// Kopf mit Host-Eckdaten (aus `system.info`, sonst aus `/info`).
#[component]
fn HostHeader() -> Element {
    let app = use_app();
    let host = use_snap::<HostInfo>("system.info");
    let info = app.info.read();
    let h = host();
    let name = h.as_ref().map(|h| h.hostname.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| info.hostname.clone());
    let mut facts: Vec<(&str, String)> = Vec::new();
    if let Some(h) = h.as_ref() {
        if !h.os.is_empty() {
            facts.push(("System", h.os.clone()));
        }
        if !h.kernel.is_empty() {
            facts.push(("Kernel", if h.arch.is_empty() { h.kernel.clone() } else { format!("{} · {}", h.kernel, h.arch) }));
        }
        if !h.cpu_model.is_empty() || h.cpu_cores > 0 {
            let cores = if h.cpu_cores > 0 { format!("{} Kerne", h.cpu_cores) } else { String::new() };
            facts.push(("Prozessor", [h.cpu_model.trim(), cores.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")));
        }
        if h.mem_total > 0 {
            let swap = if h.swap_total > 0 { format!(" · Swap {}", Unit::Bytes.format(h.swap_total as f64)) } else { String::new() };
            facts.push(("Arbeitsspeicher", format!("{}{swap}", Unit::Bytes.format(h.mem_total as f64))));
        }
        if h.boot_time > 0 {
            facts.push(("Läuft seit", shared::model::duration((fmt::now() - h.boot_time) as f64)));
        }
        if !h.virtualization.is_empty() {
            facts.push(("Virtualisierung", h.virtualization.clone()));
        }
    }
    if info.started_at > 0 {
        facts.push(("Überwachung seit", fmt::ago(info.started_at).replace("vor ", "")));
    }
    let desc = if info.demo { "Beispieldaten – echte Werte liefert der Container unter Linux" } else { "Host, Container und Netzwerk auf einen Blick" };
    rsx! {
        PageHeader { title: "Übersicht", eyebrow: name, description: desc }
        if facts.len() > 1 {
            dl { class: "-mt-2 mb-6 grid grid-cols-2 gap-x-6 gap-y-3 rounded-xl border bg-card px-4 py-3 shadow-sm sm:grid-cols-3 lg:flex lg:flex-wrap lg:gap-x-8",
                for (k, v) in facts {
                    div { key: "{k}", class: "min-w-0",
                        dt { class: "text-[11px] text-muted-foreground", "{k}" }
                        dd { class: "truncate text-sm font-medium", title: "{v}", "{v}" }
                    }
                }
            }
        }
    }
}

/// Art einer Kennzahl-Kachel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KpiKind {
    Cpu,
    Mem,
    Load,
    Temp,
    Net,
    Disk,
    Containers,
    Dns,
}

fn mem_pct(d: &ChartData, i: usize) -> Option<f64> {
    let total = sum(d, i)?;
    (total > 0.0).then(|| dim(d, "used", i).unwrap_or(0.0) / total * 100.0)
}

fn load1(d: &ChartData, i: usize) -> Option<f64> {
    dim(d, "load1", i)
}

fn running(d: &ChartData, i: usize) -> Option<f64> {
    dim(d, "running", i)
}

/// Neuester Wert in den letzten Punkten.
fn last_at(d: &ChartData) -> Option<usize> {
    (0..d.len()).rev().take(8).find(|i| d.values.iter().any(|c| c.get(*i).copied().flatten().is_some()))
}

/// Wert eines Punkts aus den Daten eines Diagramms.
type Pick = fn(&ChartData, usize) -> Option<f64>;
/// Anzeige einer Kachel: Wert, Hinweis, Zustand, fester Bereich des Mini-Verlaufs.
type Shown = (String, Option<String>, Option<Tone>, Option<(f64, f64)>);

/// Kennzahl mit Mini-Verlauf über den gewählten Zeitraum.
#[component]
fn Kpi(kind: KpiKind) -> Element {
    let (id, label, href, f): (&str, &str, &str, Pick) = match kind {
        KpiKind::Cpu => (SYSTEM_CPU.context, "CPU", "/system/cpu", sum),
        KpiKind::Mem => (SYSTEM_RAM.context, "Arbeitsspeicher", "/system/memory", mem_pct),
        KpiKind::Load => (SYSTEM_LOAD.context, "Systemlast", "/system/cpu", load1),
        KpiKind::Temp => (CPU_TEMP.context, "Temperatur", "/system/sensors", max_of),
        KpiKind::Net => (SYSTEM_NET.context, "Netzwerk", "/system/network", sum),
        KpiKind::Disk => (SYSTEM_IO.context, "Datenträger", "/system/disks", sum),
        KpiKind::Containers => (DOCKER_CONTAINERS.context, "Container", "/containers", running),
        KpiKind::Dns => (DNS_QUERIES.context, "DNS-Anfragen", "/dns", sum),
    };
    let data = use_chart(id);
    let series = use_memo(move || data.read().as_ref().map(|d| (0..d.len()).map(|i| f(d, i)).collect::<Vec<_>>()).unwrap_or_default());
    // Die Art bleibt je Kachel gleich – der Hook steht also immer an derselben Stelle.
    let blocked = (kind == KpiKind::Dns).then(|| use_latest(DNS_BLOCKED_PCT.context));

    let guard = data.read();
    let d = guard.as_ref();
    let at = d.and_then(last_at);
    let now = d.zip(at).and_then(|(d, i)| f(d, i));
    let get = |name: &str| d.zip(at).and_then(|(d, i)| dim(d, name, i));
    let (value, hint, tone, unit_range): Shown = match kind {
        KpiKind::Cpu => (show(Unit::Percent, now), get("iowait").map(|w| format!("E/A-Wartezeit {}", Unit::Percent.format(w))), now.map(|v| level(v, 85.0, 95.0)), Some((0.0, 100.0))),
        KpiKind::Mem => {
            let used = get("used");
            let total = d.zip(at).and_then(|(d, i)| sum(d, i));
            let hint = used.zip(total).map(|(u, t)| format!("{} von {}", Unit::Bytes.format(u), Unit::Bytes.format(t)));
            (show(Unit::Percent, now), hint, now.map(|v| level(v, 85.0, 95.0)), Some((0.0, 100.0)))
        }
        KpiKind::Load => {
            let hint = get("load5").zip(get("load15")).map(|(a, b)| format!("5 min {} · 15 min {}", Unit::Count.format(a), Unit::Count.format(b)));
            (now.map_or_else(|| "–".to_string(), |v| format!("{v:.2}")), hint, None, None)
        }
        KpiKind::Temp => (show(Unit::Celsius, now), Some("wärmster Fühler".to_string()), now.map(|v| level(v, 80.0, 90.0)), None),
        KpiKind::Net => {
            let hint = get("received").zip(get("sent")).map(|(r, s)| format!("↓ {} · ↑ {}", Unit::BitsPerSec.format(r), Unit::BitsPerSec.format(s)));
            (show(Unit::BitsPerSec, now), hint, None, None)
        }
        KpiKind::Disk => {
            let hint = get("read").zip(get("write")).map(|(r, w)| format!("Lesen {} · Schreiben {}", Unit::BytesPerSec.format(r), Unit::BytesPerSec.format(w)));
            (show(Unit::BytesPerSec, now), hint, None, None)
        }
        KpiKind::Containers => {
            let bad = get("unhealthy").unwrap_or(0.0) + get("restarting").unwrap_or(0.0);
            let exited = get("exited").unwrap_or(0.0);
            let hint = if bad > 0.0 {
                format!("{bad:.0} ungesund oder im Neustart")
            } else if exited > 0.0 {
                format!("{exited:.0} beendet")
            } else {
                "alle laufen".to_string()
            };
            (now.map_or_else(|| "–".to_string(), |v| format!("{v:.0} laufen")), Some(hint), Some(if bad > 0.0 { Tone::Warn } else { Tone::Good }), None)
        }
        KpiKind::Dns => {
            let b = blocked.and_then(|m| m.read().as_ref().and_then(|v| v.first().copied().flatten()));
            (show(Unit::PerSec, now), b.map(|b| format!("{} gesperrt", Unit::Percent.format(b))), None, None)
        }
    };
    drop(guard);
    let icon = match kind {
        KpiKind::Cpu => rsx! { icons::Cpu {} },
        KpiKind::Mem => rsx! { icons::MemoryStick {} },
        KpiKind::Load => rsx! { icons::Gauge {} },
        KpiKind::Temp => rsx! { icons::Thermometer {} },
        KpiKind::Net => rsx! { icons::Network {} },
        KpiKind::Disk => rsx! { icons::HardDrive {} },
        KpiKind::Containers => rsx! { icons::Container {} },
        KpiKind::Dns => rsx! { icons::Globe {} },
    };
    let tone = tone.filter(|t| *t != Tone::Good || kind == KpiKind::Containers);
    rsx! {
        StatTile { label, value, hint, icon, tone, href, class: "p-3.5",
            Sparkline { values: series(), height: 30, area: true, min: unit_range.map(|r| r.0), max: unit_range.map(|r| r.1) }
        }
    }
}

fn level(v: f64, warn: f64, crit: f64) -> Tone {
    if v >= crit {
        Tone::Bad
    } else if v >= warn {
        Tone::Warn
    } else {
        Tone::Good
    }
}

/// Grösste Container nach CPU oder Speicher (aus den Summen-Diagrammen).
#[component]
fn TopContainers() -> Element {
    let app = use_app();
    let mut by_mem = use_signal(|| false);
    let id = if by_mem() { DOCKER_MEM_ALL.context } else { DOCKER_CPU_ALL.context };
    rsx! {
        PageSection {
            title: "Grösste Container",
            description: if by_mem() { "nach Arbeitsspeicher" } else { "nach CPU (100 % = ein Kern)" },
            actions: rsx! {
                div { class: "inline-flex rounded-md border bg-background p-0.5 text-xs",
                    for (label, mem) in [("CPU", false), ("Speicher", true)] {
                        button {
                            key: "{label}",
                            r#type: "button",
                            class: "rounded px-2 py-0.5 outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                            class: if by_mem() == mem { "bg-accent font-medium" } else { "text-muted-foreground hover:text-foreground" },
                            onclick: move |_| by_mem.set(mem),
                            "{label}"
                        }
                    }
                }
            },
            if app.chart(id).is_some() {
                TopList { key: "{id}", id: id.to_string() }
            } else {
                p { class: "py-6 text-center text-xs text-muted-foreground", "Keine Werte" }
            }
        }
    }
}

#[component]
fn TopList(id: String) -> Element {
    let app = use_app();
    let latest = use_latest(&id);
    let Some(meta) = app.chart(&id) else { return rsx! {} };
    let values = latest().unwrap_or_default();
    let mut items: Vec<BarItem> = meta
        .dims
        .iter()
        .zip(values.iter())
        .filter_map(|(d, v)| {
            let v = (*v)?;
            let other = d.id == "other";
            Some(BarItem {
                label: if other { "Andere".to_string() } else { d.name.clone() },
                value: v,
                href: (!other).then(|| format!("/containers/{}", d.name)),
                ..BarItem::default()
            })
        })
        .collect();
    items.sort_by(|a, b| b.value.total_cmp(&a.value));
    rsx! {
        BarList { items, unit: meta.unit, limit: 8, empty: "Noch keine Werte" }
    }
}

/// Ein Dateisystem: Einhängepunkt, Füllstand, belegt/gesamt.
#[component]
fn FsRow(meta: ChartMeta) -> Element {
    let latest = use_latest(&meta.id);
    let v = latest().unwrap_or_default();
    let get = |k: &str| meta.dim_index(k).and_then(|i| v.get(i).copied().flatten());
    let (used, avail, reserved) = (get("used"), get("avail"), get("reserved").unwrap_or(0.0));
    let mount = meta.label("mount").map(str::to_string).unwrap_or_else(|| meta.family.clone());
    let Some((used, avail)) = used.zip(avail) else {
        return rsx! {
            div { class: "text-xs text-muted-foreground", "{mount} – warte auf Werte" }
        };
    };
    let total = used + avail + reserved;
    let pct = if used + avail > 0.0 { used / (used + avail) * 100.0 } else { 0.0 };
    let device = meta.label("device").unwrap_or("");
    rsx! {
        div { class: "grid gap-1.5",
            div { class: "flex items-baseline justify-between gap-3 text-xs",
                span { class: "flex min-w-0 items-baseline gap-2",
                    span { class: "truncate font-medium font-mono", "{mount}" }
                    if !device.is_empty() {
                        span { class: "truncate text-muted-foreground", "{device}" }
                    }
                }
                span { class: "shrink-0 tabular-nums",
                    span { class: "font-medium", "{Unit::Percent.format(pct)}" }
                    span { class: "text-muted-foreground", " · {Unit::Bytes.format(avail)} frei von {Unit::Bytes.format(total)}" }
                }
            }
            Meter { value: pct, warn: 85.0, crit: 95.0 }
        }
    }
}

/// Laufende Alarme (Momentaufnahme `alerts.active`).
#[component]
fn ActiveAlerts(list: Vec<ActiveAlert>) -> Element {
    let n = list.len();
    rsx! {
        PageSection {
            title: "Aktive Alarme",
            description: if n == 0 { "alles in Ordnung".to_string() } else { format!("{n} laufen gerade") },
            actions: rsx! {
                Link { to: "/alerts", class: "text-xs text-muted-foreground hover:text-foreground", "Alle" }
            },
            if list.is_empty() {
                div { class: "flex items-center gap-2 py-4 text-sm text-muted-foreground",
                    icons::CircleCheck { class: "size-4 text-success" }
                    "Keine aktiven Alarme"
                }
            } else {
                ul { class: "-my-2 divide-y",
                    for a in list.iter().take(8) {
                        li { key: "{a.id}", class: "flex items-start justify-between gap-3 py-2.5",
                            div { class: "min-w-0",
                                StatusDot { tone: tone_of(a.severity), label: a.title.clone() }
                                p { class: "mt-0.5 truncate pl-3.5 text-xs text-muted-foreground",
                                    "{a.subject}"
                                    if !a.value.is_empty() {
                                        " · {a.value}"
                                    }
                                }
                            }
                            span { class: "shrink-0 text-xs text-muted-foreground tabular-nums", "{fmt::ago_ms(a.since)}" }
                        }
                    }
                }
            }
        }
    }
}

/// Neueste Ereignisse im gewählten Zeitraum.
#[component]
fn RecentEvents() -> Element {
    let events = EVENTS.read();
    let list: Vec<_> = events.iter().take(8).cloned().collect();
    let total = events.len();
    drop(events);
    rsx! {
        PageSection {
            title: "Neueste Ereignisse",
            description: if total == 0 { "im gewählten Zeitraum".to_string() } else { format!("{total} im gewählten Zeitraum") },
            actions: rsx! {
                Link { to: "/events", class: "text-xs text-muted-foreground hover:text-foreground", "Alle" }
            },
            if list.is_empty() {
                p { class: "py-4 text-sm text-muted-foreground", "Keine Ereignisse im Zeitraum." }
            } else {
                ul { class: "-my-2 divide-y",
                    for e in list {
                        li { key: "{e.id}-{e.ts}", class: "flex items-start justify-between gap-3 py-2.5",
                            div { class: "min-w-0",
                                StatusDot { tone: if e.severity == Severity::Info { Tone::Neutral } else { tone_of(e.severity) }, label: e.title.clone() }
                                p { class: "mt-0.5 truncate pl-3.5 text-xs text-muted-foreground",
                                    "{e.source}"
                                    if !e.subject.is_empty() {
                                        " · {e.subject}"
                                    }
                                }
                            }
                            span { class: "shrink-0 text-xs text-muted-foreground tabular-nums", title: "{fmt::full_ms(e.ts)}", "{fmt::ago_ms(e.ts)}" }
                        }
                    }
                }
            }
        }
    }
}

/// Erreichbarkeit der Ziele als Zustandsbänder in einer Karte.
#[component]
fn ProbeStates(probes: Vec<ChartMeta>) -> Element {
    rsx! {
        div { class: "rounded-xl border bg-card p-4 shadow-sm",
            div { class: "mb-3 flex items-baseline justify-between",
                p { class: "text-sm font-medium", "Erreichbarkeit" }
                Link { to: "/probes", class: "text-xs text-muted-foreground hover:text-foreground", "Alle Ziele" }
            }
            div { class: "grid gap-3",
                for p in probes {
                    ChartCard {
                        key: "{p.id}",
                        id: p.id.clone(),
                        title: p.label("target").unwrap_or(&p.family).to_string(),
                        view: ChartView::State,
                        compact: true,
                        expandable: false,
                        class: "border-0 bg-transparent p-0 shadow-none",
                    }
                }
            }
        }
    }
}
