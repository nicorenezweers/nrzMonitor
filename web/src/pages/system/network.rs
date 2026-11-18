//! Netzwerk: Kennzahlen, Schnittstellen (Tabelle bzw. Karten am Handy), Verläufe je Schnittstelle, TCP/UDP.

use dioxus::prelude::*;
use shared::{
    catalog::system::*,
    model::Unit,
    system::{IfaceInfo, snap},
};

use super::common::{Charts, GroupTitle, KpiRow, dim, pct, use_dims};
use crate::{
    components::{
        chart::ChartCard,
        lazy::Lazy,
        ui::{
            app_shell::{EmptyState, LoadingState, PageContainer, PageHeader, PageWidth},
            badge::{Badge, BadgeSize, BadgeVariant},
            table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow, TableWrapper},
        },
        widgets::{Meter, StatTile, StatusDot, Tone, show},
    },
    live::use_snap,
    state::use_app,
};

#[component]
pub fn Network() -> Element {
    let app = use_app();
    let ifaces = use_snap::<Vec<IfaceInfo>>(snap::INTERFACES);
    let net = use_dims(&SYSTEM_NET.chart_id(None));
    let tcp = use_dims(&NET_TCP_CONN.chart_id(None));
    let ct = use_dims(&NET_CONNTRACK.chart_id(None));
    let sockets = use_dims(&NET_SOCKETS.chart_id(None));
    let mut selected = use_signal(|| None::<String>);

    let loaded = ifaces.read().is_some();
    let list = ifaces().unwrap_or_default();
    let n = net.read();
    let (rx, tx) = (dim(&n, "received"), dim(&n, "sent"));
    let c = ct.read();
    let (ct_count, ct_max) = (dim(&c, "count"), dim(&c, "max"));
    let up = list.iter().filter(|i| i.up).count();
    // Ausgewählte Schnittstelle für die Details (Standard: die mit dem meisten Verkehr).
    let current = selected().filter(|s| list.iter().any(|i| &i.name == s)).or_else(|| {
        list.iter().max_by(|a, b| (a.rx + a.tx).total_cmp(&(b.rx + b.tx))).map(|i| i.name.clone())
    });

    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "Netzwerk", description: "Schnittstellen, TCP/UDP und Verbindungen" }
            div { class: "grid gap-6",
                KpiRow { cols: 5,
                    StatTile { label: "Empfang", value: show(Unit::BitsPerSec, rx), hint: "alle Schnittstellen" }
                    StatTile { label: "Senden", value: show(Unit::BitsPerSec, tx), hint: "alle Schnittstellen" }
                    StatTile {
                        label: "TCP offen",
                        value: show(Unit::Count, dim(&tcp.read(), "established")),
                        hint: dim(&sockets.read(), "tw").map_or_else(|| "Verbindungen".to_string(), |t| format!("{} in Time-Wait", Unit::Count.format(t))),
                    }
                    StatTile {
                        label: "conntrack",
                        value: show(Unit::Count, ct_count),
                        hint: ct_max.map_or_else(|| "Einträge".to_string(), |m| format!("von {}", Unit::Count.format(m))),
                        if let (Some(c), Some(m)) = (ct_count, ct_max) {
                            Meter { value: pct(c, m), warn: 70.0, crit: 90.0 }
                        }
                    }
                    StatTile { label: "Schnittstellen", value: "{up} / {list.len()}", hint: "aktiv / gesamt" }
                }
                if app.chart(&SYSTEM_NET.chart_id(None)).is_some() {
                    ChartCard { id: SYSTEM_NET.chart_id(None), height: 200 }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Schnittstellen", hint: "Klick auf den Namen zeigt Pakete, Fehler und Verworfenes" }
                    if !loaded {
                        LoadingState {}
                    } else if list.is_empty() {
                        EmptyState { title: "Keine Schnittstellen", description: "Der Sammler hat noch keine Schnittstellen gemeldet.", icon: rsx! { icons::Network {} } }
                    } else {
                        IfaceTable { list: list.clone(), current: current.clone(), onselect: move |n| selected.set(Some(n)) }
                        IfaceCards { list: list.clone(), current: current.clone(), onselect: move |n| selected.set(Some(n)) }
                    }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Durchsatz je Schnittstelle" }
                    IfaceTraffic { names: list.iter().map(|i| i.name.clone()).collect::<Vec<_>>() }
                }
                if let Some(name) = current {
                    div { class: "grid gap-3",
                        div { class: "flex flex-wrap items-center gap-2",
                            GroupTitle { title: "Pakete, Fehler, Verworfen" }
                            for i in list.iter() {
                                {
                                    let n = i.name.clone();
                                    let on = n == name;
                                    rsx! {
                                        button {
                                            key: "{n}",
                                            r#type: "button",
                                            class: "rounded-md border px-2 py-0.5 font-mono text-xs transition-colors hover:bg-accent",
                                            class: if on { "bg-accent text-accent-foreground" } else { "text-muted-foreground" },
                                            onclick: move |_| selected.set(Some(n.clone())),
                                            "{i.name}"
                                        }
                                    }
                                }
                            }
                        }
                        IfaceDetail { key: "{name}", name: name.clone() }
                    }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "TCP" }
                    Charts { ids: vec![NET_TCP_CONN.chart_id(None), NET_TCP_OPENS.chart_id(None), NET_TCP_ERRORS.chart_id(None)], height: 160 }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "UDP, Sockets und Verbindungsverfolgung" }
                    Charts { ids: vec![NET_UDP_PACKETS.chart_id(None), NET_UDP_ERRORS.chart_id(None), NET_SOCKETS.chart_id(None), NET_CONNTRACK.chart_id(None)], height: 160, pairs: true }
                }
            }
        }
    }
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "physical" => "Kabel",
        "wireless" => "WLAN",
        "bridge" => "Bridge",
        "vpn" => "VPN",
        "docker" => "Docker",
        "virtual" => "Virtuell",
        k => k,
    }
}

fn state(i: &IfaceInfo) -> (Tone, &'static str) {
    match (i.up, i.state.as_str()) {
        (true, _) => (Tone::Good, "aktiv"),
        (false, "down") => (Tone::Bad, "getrennt"),
        _ => (Tone::Neutral, "unbekannt"),
    }
}

fn speed(i: &IfaceInfo) -> String {
    match i.speed {
        0 => "–".to_string(),
        s if s >= 1000 && s % 1000 == 0 => format!("{} Gbit/s", s / 1000),
        s => format!("{s} Mbit/s"),
    }
}

#[component]
fn IfaceTable(list: Vec<IfaceInfo>, current: Option<String>, onselect: EventHandler<String>) -> Element {
    rsx! {
        TableWrapper { class: "hidden max-h-none rounded-xl bg-card shadow-sm md:block",
            Table { class: "max-w-none",
                TableHeader {
                    TableRow {
                        TableHead { "Name" }
                        TableHead { "Art" }
                        TableHead { "Zustand" }
                        TableHead { "Adressen" }
                        TableHead { class: "text-right", "Tempo" }
                        TableHead { class: "text-right", "MTU" }
                        TableHead { class: "text-right", "Empfang" }
                        TableHead { class: "text-right", "Senden" }
                        TableHead { class: "hidden text-right xl:table-cell", "Summe empfangen" }
                        TableHead { class: "hidden text-right xl:table-cell", "Summe gesendet" }
                    }
                }
                TableBody {
                    for i in list.iter() {
                        {
                            let (tone, label) = state(i);
                            let n = i.name.clone();
                            let sel = current.as_deref() == Some(i.name.as_str());
                            rsx! {
                                TableRow { key: "{i.name}", data_state: if sel { "selected" } else { "" }, class: "cursor-pointer",
                                    TableCell { class: "px-2 py-2.5",
                                        button {
                                            r#type: "button",
                                            class: "font-mono font-medium outline-none hover:underline focus-visible:underline",
                                            onclick: move |_| onselect.call(n.clone()),
                                            "{i.name}"
                                        }
                                        if i.ports > 0 {
                                            span { class: "ml-2 text-xs text-muted-foreground", "{i.ports} Ports" }
                                        }
                                    }
                                    TableCell { class: "px-2 py-2.5",
                                        Badge { variant: BadgeVariant::Secondary, size: BadgeSize::Sm, "{kind_label(&i.kind)}" }
                                    }
                                    TableCell { class: "px-2 py-2.5", StatusDot { tone, label } }
                                    TableCell { class: "px-2 py-2.5 font-mono text-xs",
                                        if i.addrs.is_empty() {
                                            span { class: "text-muted-foreground", "–" }
                                        }
                                        for a in i.addrs.iter() {
                                            div { key: "{a}", class: "max-w-72 truncate", title: "{a}", "{a}" }
                                        }
                                    }
                                    TableCell { class: "px-2 py-2.5 text-right tabular-nums", "{speed(i)}" }
                                    TableCell { class: "px-2 py-2.5 text-right tabular-nums", "{i.mtu}" }
                                    TableCell { class: "px-2 py-2.5 text-right font-medium tabular-nums", "{Unit::BitsPerSec.format(i.rx)}" }
                                    TableCell { class: "px-2 py-2.5 text-right font-medium tabular-nums", "{Unit::BitsPerSec.format(i.tx)}" }
                                    TableCell { class: "hidden px-2 py-2.5 text-right text-muted-foreground tabular-nums xl:table-cell", "{Unit::Bytes.format(i.rx_total as f64)}" }
                                    TableCell { class: "hidden px-2 py-2.5 text-right text-muted-foreground tabular-nums xl:table-cell", "{Unit::Bytes.format(i.tx_total as f64)}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Handy: eine kompakte Karte je Schnittstelle.
#[component]
fn IfaceCards(list: Vec<IfaceInfo>, current: Option<String>, onselect: EventHandler<String>) -> Element {
    rsx! {
        div { class: "grid gap-3 md:hidden",
            for i in list.iter() {
                {
                    let (tone, label) = state(i);
                    let n = i.name.clone();
                    let sel = current.as_deref() == Some(i.name.as_str());
                    rsx! {
                        button {
                            key: "{i.name}",
                            r#type: "button",
                            class: "rounded-xl border bg-card p-3 text-left shadow-sm transition-colors",
                            class: if sel { "border-foreground/30" },
                            onclick: move |_| onselect.call(n.clone()),
                            div { class: "flex items-center justify-between gap-2",
                                div { class: "flex min-w-0 items-center gap-2",
                                    span { class: "truncate font-mono text-sm font-medium", "{i.name}" }
                                    Badge { variant: BadgeVariant::Secondary, size: BadgeSize::Sm, "{kind_label(&i.kind)}" }
                                }
                                StatusDot { tone, label }
                            }
                            if !i.addrs.is_empty() {
                                p { class: "mt-1 truncate font-mono text-xs text-muted-foreground", "{i.addrs.join(\", \")}" }
                            }
                            div { class: "mt-2 grid grid-cols-3 gap-2 text-xs",
                                div {
                                    p { class: "text-muted-foreground", "Empfang" }
                                    p { class: "font-medium tabular-nums", "{Unit::BitsPerSec.format(i.rx)}" }
                                }
                                div {
                                    p { class: "text-muted-foreground", "Senden" }
                                    p { class: "font-medium tabular-nums", "{Unit::BitsPerSec.format(i.tx)}" }
                                }
                                div {
                                    p { class: "text-muted-foreground", "Tempo · MTU" }
                                    p { class: "truncate tabular-nums", "{speed(i)} · {i.mtu}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Pakete, Fehler und Verworfenes einer Schnittstelle (der Durchsatz steht schon darüber).
#[component]
fn IfaceDetail(name: String) -> Element {
    let n = Some(name.as_str());
    rsx! {
        Charts {
            ids: vec![NET_PACKETS.chart_id(n), NET_ERRORS.chart_id(n), NET_DROPS.chart_id(n)],
            height: 150,
        }
    }
}

/// Durchsatz je Schnittstelle in der Reihenfolge der Tabelle; vier Stück stehen in einer Reihe.
#[component]
fn IfaceTraffic(names: Vec<String>) -> Element {
    let app = use_app();
    let mut ids: Vec<String> = names.iter().map(|n| NET_TRAFFIC.chart_id(Some(n))).filter(|id| app.chart(id).is_some()).collect();
    // Schnittstellen ohne Eintrag in der Momentaufnahme (z. B. gerade erst aufgetaucht) hinten anhängen.
    for c in app.by_context(NET_TRAFFIC.context) {
        if !c.retired && !ids.contains(&c.id) {
            ids.push(c.id);
        }
    }
    let grid = match ids.len() {
        2 | 4 => "grid gap-4 lg:grid-cols-2",
        _ => "grid gap-4 lg:grid-cols-2 2xl:grid-cols-3",
    };
    rsx! {
        div { class: "{grid}",
            for id in ids {
                Lazy { key: "{id}", min_height: 260,
                    ChartCard { id: id.clone(), height: 150 }
                }
            }
        }
    }
}
