//! Abfrageprotokoll: jede einzelne DNS-Anfrage mit Filtern, Blättern, Live-Nachladen und Details.

use dioxus::prelude::*;
use shared::{
    dns::{DnsStatus, DnsTop, DomainInfo, QueryFilter, QueryPage, Timeline},
    model::{ChartKind, Unit},
};

use super::{
    QUERY_FILTER,
    common::{BarList, KindBadge, ListCard, NotConfigured, Notice, pct, range_of, range_params, use_api, use_api_live, use_status, when_ms},
};
use crate::{
    api,
    components::{
        chart::{Plot, PlotData, PlotSeries},
        ui::{
            app_shell::{PageContainer, PageHeader, PageWidth},
            button::{Button, ButtonSize, ButtonVariant},
            checkbox::Checkbox,
            input::Input,
        },
        widgets::StatTile,
    },
    fmt,
    state::use_app,
};

const SELECT: &str = "h-9 min-w-0 flex-1 sm:flex-none rounded-md border border-input bg-transparent px-2 text-sm text-foreground outline-none focus-visible:border-ring dark:bg-input/30";
const QTYPES: &[&str] = &["A", "AAAA", "HTTPS", "PTR", "TXT", "SRV", "CNAME", "MX", "NS", "SOA"];
const RCODES: &[&str] = &["NOERROR", "NXDOMAIN", "SERVFAIL", "REFUSED"];

#[derive(Debug, Clone, PartialEq)]
enum Detail {
    Domain(String),
    Client(String),
}

#[component]
pub fn DnsQueries() -> Element {
    let app = use_app();
    let status = use_status::<DnsStatus>("dns.status");
    let start = QUERY_FILTER.peek().clone();
    let first_detail = if !start.client.is_empty() {
        Some(Detail::Client(start.client.clone()))
    } else if !start.name.is_empty() && start.name.contains('.') {
        Some(Detail::Domain(start.name.clone()))
    } else {
        None
    };
    let mut filter = use_signal(|| start);
    let mut detail = use_signal(|| first_detail);
    let mut page = use_signal(|| 1u32);
    let mut live = use_signal(|| true);
    use_drop(|| *QUERY_FILTER.write() = QueryFilter::default());
    let per_page = 100u32;
    // Nur Seite 1 läuft live mit (sonst verschieben sich die Seiten unter dem Leser).
    let mut active = use_signal(|| true);
    use_effect(move || active.set(live() && page() == 1));
    let (data, err, _) = use_api_live::<QueryPage>(
        move || {
            let f = filter();
            let mut p = range_params(app);
            p.extend([
                ("client", f.client),
                ("name", f.name),
                ("qtype", f.qtype),
                ("kind", f.kind),
                ("rcode", f.rcode),
                ("upstream", f.upstream),
                ("page", page().to_string()),
                ("per_page", per_page.to_string()),
            ]);
            api::qs("/dns/queries", &p)
        },
        5000,
        active,
    );
    let mut set = move |edit: &dyn Fn(&mut QueryFilter)| {
        edit(&mut filter.write());
        page.set(1);
    };
    if status().is_some_and(|s| !s.configured && !s.demo) {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "Abfrageprotokoll", description: "Jede einzelne DNS-Anfrage" }
                NotConfigured {
                    title: "DNS ist noch nicht verbunden",
                    description: "Hinterlege in den Einstellungen das MCP-Token von nrzNetwork – danach erscheint hier jede Anfrage.",
                    icon: rsx! { icons::ListFilter {} },
                }
            }
        };
    }
    let f = filter();
    let d = data();
    let total = d.as_ref().map_or(0, |d| d.total);
    let pages = total.div_ceil(u64::from(per_page)).max(1);
    let any_filter = f != QueryFilter::default();
    let actions = rsx! {
        label { class: "flex cursor-pointer items-center gap-2 text-sm",
            Checkbox { checked: live(), aria_label: "Live nachladen", on_checked_change: move |v: bool| live.set(v) }
            "Live"
        }
    };
    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "Abfrageprotokoll", description: "Jede einzelne DNS-Anfrage im gewählten Zeitraum", actions }
            div { class: "grid grid-cols-1 gap-4",
                // Filter
                div { class: "flex flex-wrap items-center gap-2",
                    div { class: "w-full sm:w-44",
                        Input { placeholder: "Client (IP)", value: f.client.clone(),
                            oninput: move |e: FormEvent| { let v = e.value(); set(&move |f: &mut QueryFilter| f.client = v.trim().to_string()); } }
                    }
                    div { class: "w-full sm:w-64",
                        Input { placeholder: "Name enthält …", value: f.name.clone(),
                            oninput: move |e: FormEvent| { let v = e.value(); set(&move |f: &mut QueryFilter| f.name = v.trim().to_lowercase()); } }
                    }
                    select { class: SELECT, value: "{f.qtype}", "aria-label": "Typ",
                        onchange: move |e| { let v = e.value(); set(&move |f: &mut QueryFilter| f.qtype = v.clone()); },
                        option { value: "", "Alle Typen" }
                        for q in QTYPES { option { value: *q, selected: f.qtype == *q, "{q}" } }
                    }
                    select { class: SELECT, value: "{f.kind}", "aria-label": "Quelle",
                        onchange: move |e| { let v = e.value(); set(&move |f: &mut QueryFilter| f.kind = v.clone()); },
                        option { value: "", "Alle Quellen" }
                        for (k, l) in shared::dns::KINDS { option { value: *k, selected: f.kind == *k, "{l}" } }
                    }
                    select { class: SELECT, value: "{f.rcode}", "aria-label": "Antwortcode",
                        onchange: move |e| { let v = e.value(); set(&move |f: &mut QueryFilter| f.rcode = v.clone()); },
                        option { value: "", "Alle Codes" }
                        for r in RCODES { option { value: *r, selected: f.rcode == *r, "{r}" } }
                    }
                    if !f.upstream.is_empty() {
                        Button { variant: ButtonVariant::Secondary, size: ButtonSize::Sm,
                            onclick: move |_| set(&|f: &mut QueryFilter| f.upstream.clear()),
                            {format!("Weiterleitung: {}", f.upstream)}
                            icons::X { class: "size-3.5" }
                        }
                    }
                    if any_filter {
                        Button { variant: ButtonVariant::Ghost, size: ButtonSize::Sm,
                            onclick: move |_| { filter.set(QueryFilter::default()); detail.set(None); page.set(1); },
                            icons::X { class: "size-4" }
                            "Filter zurücksetzen"
                        }
                    }
                }
                if let Some(e) = err() {
                    Notice { text: e, error: true }
                }
                match detail() {
                    Some(Detail::Domain(n)) => rsx! { DomainDetail { key: "{n}", name: n.clone(), onclose: move |_| detail.set(None) } },
                    Some(Detail::Client(c)) => rsx! { ClientDetail { key: "{c}", client: c.clone(), onclose: move |_| detail.set(None) } },
                    None => rsx! {},
                }
                // Tabelle
                div { class: "rounded-xl border bg-card shadow-sm",
                    div { class: "overflow-x-auto",
                        table { class: "w-full text-sm",
                            thead { class: "border-b text-left text-xs text-muted-foreground",
                                tr {
                                    th { class: "px-3 py-2 font-medium", "Zeit" }
                                    th { class: "hidden px-3 py-2 font-medium sm:table-cell", "Client" }
                                    th { class: "px-3 py-2 font-medium", "Name" }
                                    th { class: "hidden px-3 py-2 font-medium md:table-cell", "Typ" }
                                    th { class: "hidden px-3 py-2 font-medium sm:table-cell", "Quelle" }
                                    th { class: "hidden px-3 py-2 font-medium lg:table-cell", "Code" }
                                    th { class: "hidden px-3 py-2 font-medium xl:table-cell", "Antwort" }
                                    th { class: "hidden px-3 py-2 text-right font-medium md:table-cell", "Dauer" }
                                }
                            }
                            tbody { class: "divide-y",
                                if let Some(d) = &d {
                                    for e in d.entries.iter() {
                                        tr { key: "{e.id}-{e.ts}", class: "hover:bg-muted/40",
                                            td { class: "px-3 py-1.5 align-top whitespace-nowrap tabular-nums text-muted-foreground sm:align-middle", title: "{fmt::full_ms(e.ts)}", {when_ms(e.ts)} }
                                            td { class: "hidden px-3 py-1.5 whitespace-nowrap sm:table-cell",
                                                button { class: "tabular-nums hover:underline",
                                                    onclick: { let c = e.client.clone(); move |_| { let c2 = c.clone(); set(&move |f: &mut QueryFilter| f.client = c2.clone()); detail.set(Some(Detail::Client(c.clone()))); } },
                                                    "{e.client}"
                                                }
                                            }
                                            td { class: "max-w-[15rem] px-3 py-1.5 sm:max-w-md",
                                                button { class: "block max-w-full truncate text-left hover:underline", title: "{e.name}",
                                                    onclick: { let n = e.name.clone(); move |_| { let n2 = n.clone(); set(&move |f: &mut QueryFilter| f.name = n2.clone()); detail.set(Some(Detail::Domain(n.clone()))); } },
                                                    "{e.name}"
                                                }
                                                span { class: "flex items-center gap-1.5 text-xs text-muted-foreground tabular-nums sm:hidden",
                                                    span { class: "size-2 shrink-0 rounded-full", style: "background:{crate::components::chart::color(super::common::kind_slot(&e.kind))}" }
                                                    {format!("{} · {} · {}", shared::dns::kind_label(&e.kind), e.client, e.qtype)}
                                                }
                                            }
                                            td { class: "hidden px-3 py-1.5 md:table-cell", "{e.qtype}" }
                                            td { class: "hidden px-3 py-1.5 sm:table-cell",
                                                button { onclick: { let k = e.kind.clone(); move |_| { let k = k.clone(); set(&move |f: &mut QueryFilter| f.kind = k.clone()); } },
                                                    KindBadge { kind: e.kind.clone() }
                                                }
                                            }
                                            td { class: "hidden px-3 py-1.5 lg:table-cell",
                                                span { class: if e.rcode == "NOERROR" { "text-muted-foreground" } else { "text-warning" }, "{e.rcode}" }
                                            }
                                            td { class: "hidden max-w-xs truncate px-3 py-1.5 text-muted-foreground xl:table-cell", title: "{e.answer}", "{e.answer}" }
                                            td { class: "hidden px-3 py-1.5 text-right tabular-nums md:table-cell", {Unit::Millis.format(f64::from(e.ms))} }
                                        }
                                    }
                                    if d.entries.is_empty() {
                                        tr { td { colspan: "8", class: "px-3 py-10 text-center text-muted-foreground", "Keine Anfragen für diese Filter im Zeitraum" } }
                                    }
                                } else {
                                    tr { td { colspan: "8", class: "px-3 py-10 text-center text-muted-foreground", "Lade …" } }
                                }
                            }
                        }
                    }
                    div { class: "flex flex-wrap items-center justify-between gap-2 border-t px-3 py-2 text-sm",
                        span { class: "text-muted-foreground tabular-nums",
                            {format!("{} Anfragen · Seite {} von {}", Unit::Count.format(total as f64), page(), pages)}
                        }
                        div { class: "flex items-center gap-2",
                            Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, disabled: page() <= 1,
                                onclick: move |_| page.set(page().saturating_sub(1).max(1)),
                                icons::ChevronLeft { class: "size-4" }
                                "Neuere"
                            }
                            Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, disabled: u64::from(page()) >= pages,
                                onclick: move |_| page.set(page() + 1),
                                "Ältere"
                                icons::ChevronRight { class: "size-4" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Verlauf als gestapeltes Diagramm (weitergeleitet, Cache, gesperrt, Fehler – Farben wie `dns.queries`).
fn timeline_plot(t: &Timeline) -> PlotData {
    let s = |id: &str, name: &str, slot: usize, f: fn(&shared::dns::TimelinePoint) -> u64| PlotSeries {
        id: id.into(),
        name: name.into(),
        slot,
        negate: false,
        values: t.points.iter().map(|p| Some(f(p) as f64)).collect(),
    };
    PlotData {
        start: t.points.first().map_or(0, |p| p.ts),
        step: t.step.max(1),
        series: vec![
            s("forwarded", "Weitergeleitet", 0, |p| p.forwarded),
            s("cached", "Cache", 1, |p| p.cached),
            s("blocked", "Gesperrt", 2, |p| p.blocked),
            s("failed", "Fehler", 5, |p| p.failed),
        ],
    }
}

#[component]
fn DomainDetail(name: String, onclose: EventHandler<()>) -> Element {
    let app = use_app();
    let n = name.clone();
    let (info, err, _) = use_api::<DomainInfo>(
        move || {
            let mut p = range_params(app);
            p.push(("name", n.clone()));
            api::qs("/dns/domain", &p)
        },
        0,
    );
    let plot = use_memo(move || info().map(|i| timeline_plot(&i.timeline)).unwrap_or_default());
    let i = info().unwrap_or_default();
    rsx! {
        DetailFrame { title: format!("Domain {name}"), onclose,
            if let Some(e) = err() { Notice { text: e, error: true } }
            div { class: "grid grid-cols-2 gap-3 lg:grid-cols-4",
                StatTile { label: "Anfragen", value: Unit::Count.format(i.queries as f64) }
                StatTile { label: "Gesperrt", value: pct(i.blocked, i.queries) }
                StatTile { label: "Antwortzeit", value: Unit::Millis.format(i.avg_ms) }
                StatTile { label: "Zuletzt", value: if i.last > 0 { fmt::ago_ms(i.last) } else { "–".into() },
                    hint: if i.first > 0 { format!("zuerst {}", when_ms(i.first)) } else { String::new() } }
            }
            div { class: "rounded-xl border bg-card p-3", Plot { data: plot, kind: ChartKind::Stacked, unit: Unit::Count, height: 140 } }
            div { class: "grid gap-3 md:grid-cols-3",
                ListCard { title: "Clients", BarList { items: i.clients.clone(), show_blocked: true } }
                ListCard { title: "Antworten", BarList { items: i.answers.clone(), empty: "Keine Antworten" } }
                ListCard { title: "Typen und Codes",
                    BarList { items: i.qtypes.clone() }
                    BarList { items: i.rcodes.clone() }
                }
            }
        }
    }
}

#[component]
fn ClientDetail(client: String, onclose: EventHandler<()>) -> Element {
    let app = use_app();
    let c = client.clone();
    let (top, err, _) = use_api::<DnsTop>(
        move || {
            let mut p = range_params(app);
            p.extend([("client", c.clone()), ("limit", "8".into())]);
            api::qs("/dns/top", &p)
        },
        0,
    );
    let c2 = client.clone();
    let (tl, _, _) = use_api::<Timeline>(
        move || {
            let (a, b) = range_of(app);
            api::qs("/dns/timeline", &[("from", a.to_string()), ("to", b.to_string()), ("client", c2.clone()), ("points", "120".into())])
        },
        0,
    );
    let plot = use_memo(move || tl().map(|t| timeline_plot(&t)).unwrap_or_default());
    let t = top().unwrap_or_default();
    rsx! {
        DetailFrame { title: format!("Client {client}"), onclose,
            if let Some(e) = err() { Notice { text: e, error: true } }
            div { class: "grid grid-cols-2 gap-3 lg:grid-cols-4",
                StatTile { label: "Anfragen", value: Unit::Count.format(t.total as f64) }
                StatTile { label: "Gesperrt", value: pct(t.blocked, t.total) }
                StatTile { label: "Domains", value: Unit::Count.format(t.domains_count as f64) }
                StatTile { label: "Antwortzeit", value: Unit::Millis.format(t.avg_ms) }
            }
            div { class: "rounded-xl border bg-card p-3", Plot { data: plot, kind: ChartKind::Stacked, unit: Unit::Count, height: 140 } }
            div { class: "grid gap-3 md:grid-cols-3",
                ListCard { title: "Häufigste Domains", BarList { items: t.domains.clone(), show_blocked: true } }
                ListCard { title: "Gesperrte Domains", BarList { items: t.blocked_domains.clone(), empty: "Nichts gesperrt" } }
                ListCard { title: "Abfragetypen", BarList { items: t.qtypes.clone() } }
            }
        }
    }
}

#[component]
fn DetailFrame(#[props(into)] title: String, onclose: EventHandler<()>, children: Element) -> Element {
    rsx! {
        section { class: "grid gap-3 rounded-xl border bg-muted/30 p-3 sm:p-4",
            div { class: "flex items-center justify-between gap-2",
                h2 { class: "min-w-0 truncate font-semibold", "{title}" }
                Button { variant: ButtonVariant::Ghost, size: ButtonSize::IconSm, aria_label: "Schliessen", onclick: move |_| onclose.call(()),
                    icons::X { class: "size-4" }
                }
            }
            {children}
        }
    }
}
