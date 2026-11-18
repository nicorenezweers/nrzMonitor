//! Seite „Ereignisse“: was wann passiert ist – gefiltert, live ergänzt, ältere nachladbar.

use std::collections::HashSet;

use dioxus::prelude::*;
use gloo_timers::future::TimeoutFuture;
use shared::events::{Event, EventPage, Severity};

use crate::{
    api,
    components::{
        ui::{
            app_shell::{EmptyState, LoadingState, PageContainer, PageHeader, PageWidth},
            button::{Button, ButtonSize, ButtonVariant},
            input::{Input, InputType},
            select::{Select, SelectContent, SelectGroup, SelectOption, SelectTrigger, SelectValue},
            table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow, TableWrapper},
        },
        widgets::{StatusDot, Tone},
    },
    fmt,
    live::use_events,
    state::use_app,
};

/// Einträge je Abruf.
const PAGE: u32 = 100;
/// Bekannte Quellen (ergänzt um die, die in der Liste vorkommen).
const SOURCES: &[&str] = &["alert", "docker", "system", "dns", "cloudflare", "netbird", "probe", "monitor"];

const ALL_SOURCES: &str = "Alle Quellen";
/// Auswahl der Mindest-Schwere: (Beschriftung, Wert).
const SEVERITIES: &[(&str, Option<Severity>)] =
    &[("Alle Schweren", None), ("Ab Warnung", Some(Severity::Warn)), ("Nur kritisch", Some(Severity::Crit))];

pub(crate) fn severity_text(s: Severity) -> (&'static str, Tone) {
    match s {
        Severity::Info => ("Info", Tone::Neutral),
        Severity::Ok => ("OK", Tone::Good),
        Severity::Warn => ("Warnung", Tone::Warn),
        Severity::Crit => ("Kritisch", Tone::Bad),
    }
}

fn severity_rank(s: Severity) -> u8 {
    match s {
        Severity::Info | Severity::Ok => 0,
        Severity::Warn => 1,
        Severity::Crit => 2,
    }
}

/// Aktive Filter.
#[derive(Clone, PartialEq)]
struct Filter {
    severity: Option<Severity>,
    source: String,
    q: String,
    /// Zeitraum der Seite in Unix-ms.
    after: i64,
    before: i64,
    live: bool,
}

impl Filter {
    fn path(&self, before: Option<i64>) -> String {
        let sev = self.severity.map(|s| serde_json::to_value(s).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default());
        api::qs(
            "/events",
            &[
                ("after", self.after.to_string()),
                ("before", before.or((!self.live).then_some(self.before)).map(|b| b.to_string()).unwrap_or_default()),
                ("source", self.source.clone()),
                ("severity", sev.unwrap_or_default()),
                ("q", self.q.clone()),
                ("limit", PAGE.to_string()),
            ],
        )
    }

    /// Passt ein neues (live gemeldetes) Ereignis zum Filter?
    fn matches(&self, e: &Event) -> bool {
        if !self.live || e.ts < self.after {
            return false;
        }
        if self.severity.is_some_and(|s| severity_rank(e.severity) < severity_rank(s)) {
            return false;
        }
        if !self.source.is_empty() && e.source != self.source {
            return false;
        }
        let q = self.q.trim().to_lowercase();
        q.is_empty() || [&e.title, &e.message, &e.subject, &e.kind].iter().any(|t| t.to_lowercase().contains(&q))
    }
}

#[component]
pub fn Events() -> Element {
    let app = use_app();
    let live_events = use_events();
    let mut severity = use_signal(|| None::<Severity>);
    let mut source = use_signal(String::new);
    let mut q_raw = use_signal(String::new);
    let mut q = use_signal(String::new);

    // Freitext entprellen.
    use_effect(move || {
        let v = q_raw();
        spawn(async move {
            TimeoutFuture::new(350).await;
            if *q_raw.peek() == v && *q.peek() != v {
                q.set(v);
            }
        });
    });

    let filter = use_memo(move || {
        let range = app.range.read();
        let (a, b) = range.resolve(fmt::now());
        Filter { severity: severity(), source: source(), q: q(), after: a * 1000, before: b * 1000, live: range.is_live() }
    });

    // Erste Seite: bei jedem neuen Filter neu.
    let page = use_resource(move || {
        let f = filter();
        async move { api::get::<EventPage>(&f.path(None)).await.map(|p| (f, p)) }
    });
    // Nachgeladene ältere Einträge (gehören zum Filter von `page`).
    let mut older = use_signal(Vec::<Event>::new);
    let mut more = use_signal(|| true);
    let mut loading_more = use_signal(|| false);
    use_effect(move || {
        if let Some(Ok((_, p))) = page.read().as_ref() {
            older.set(Vec::new());
            more.set(p.events.len() as u32 >= PAGE);
        }
    });

    let list = use_memo(move || {
        let page = page.read();
        let Some(Ok((f, p))) = page.as_ref() else { return None };
        let newest = p.events.first().map_or(f.after, |e| e.ts);
        let mut seen: HashSet<(i64, String, String, String, String)> = HashSet::new();
        let key = |e: &Event| (e.ts, e.source.clone(), e.kind.clone(), e.subject.clone(), e.title.clone());
        let mut out = Vec::new();
        for e in live_events.read().iter().filter(|e| e.ts >= newest && f.matches(e)) {
            if seen.insert(key(e)) {
                out.push(e.clone());
            }
        }
        for e in p.events.iter().chain(older.read().iter()) {
            if seen.insert(key(e)) {
                out.push(e.clone());
            }
        }
        let added = out.len().saturating_sub(p.events.len() + older.read().len());
        Some((out, p.total + added as u64))
    });

    let load_more = move |_| {
        let Some(Ok((f, p))) = page.read().as_ref().cloned() else { return };
        let oldest = older.read().last().or(p.events.last()).map(|e| e.ts);
        let Some(oldest) = oldest else { return };
        loading_more.set(true);
        spawn(async move {
            if let Ok(next) = api::get::<EventPage>(&f.path(Some(oldest))).await {
                more.set(next.events.len() as u32 >= PAGE);
                older.write().extend(next.events);
            }
            loading_more.set(false);
        });
    };

    // Quellen für den Filter: bekannte + vorkommende.
    let sources = use_memo(move || {
        let mut s: Vec<String> = SOURCES.iter().map(|s| (*s).to_string()).collect();
        if let Some((l, _)) = list.read().as_ref() {
            for e in l {
                if !s.contains(&e.source) {
                    s.push(e.source.clone());
                }
            }
        }
        s
    });

    let range = app.range.read();
    let range_text = if range.is_live() {
        format!("letzte {}", super::storage::span_label(range.span() as u64))
    } else {
        let (a, b) = range.resolve(fmt::now());
        format!("{} – {}", fmt::point(a, 60), fmt::point(b, 60))
    };

    rsx! {
        PageContainer { width: PageWidth::Wide,
            PageHeader { title: "Ereignisse", description: "Was wann passiert ist" }
            // Filter
            div { class: "mb-4 flex flex-wrap items-center gap-2",
                div { class: "w-full sm:w-72",
                    Input {
                        r#type: InputType::Search,
                        placeholder: "Suchen in Titel, Meldung, Objekt …",
                        value: q_raw(),
                        oninput: move |e: FormEvent| q_raw.set(e.value()),
                    }
                }
                Select {
                    class: "w-[calc(50%-0.25rem)] sm:w-44",
                    default_value: SEVERITIES[0].0.to_string(),
                    on_change: move |v: Option<String>| severity.set(SEVERITIES.iter().find(|(l, _)| Some(*l) == v.as_deref()).and_then(|(_, s)| *s)),
                    SelectTrigger { SelectValue {} }
                    SelectContent { class: "w-44",
                        SelectGroup { aria_label: "Schwere",
                            for (l, _) in SEVERITIES.iter() {
                                SelectOption { key: "{l}", value: l.to_string(), "{l}" }
                            }
                        }
                    }
                }
                Select {
                    class: "w-[calc(50%-0.25rem)] sm:w-44",
                    default_value: ALL_SOURCES.to_string(),
                    on_change: move |v: Option<String>| source.set(v.filter(|v| v != ALL_SOURCES).unwrap_or_default()),
                    SelectTrigger { SelectValue {} }
                    SelectContent { class: "w-44",
                        SelectGroup { aria_label: "Quelle",
                            SelectOption { value: ALL_SOURCES.to_string(), "{ALL_SOURCES}" }
                            for s in sources.read().iter() {
                                SelectOption { key: "{s}", value: s.clone(), "{s}" }
                            }
                        }
                    }
                }
                span { class: "text-xs text-muted-foreground sm:ml-auto",
                    "Zeitraum: {range_text}"
                    if let Some((l, total)) = list.read().as_ref() {
                        " · {l.len()} von {total}"
                    }
                }
            }
            match (list.read().as_ref(), page.read().as_ref()) {
                (Some((l, _)), _) if l.is_empty() => rsx! {
                    EmptyState { title: "Keine Ereignisse", description: "Im gewählten Zeitraum passt nichts zum Filter.", icon: rsx! { icons::History {} } }
                },
                (Some((l, _)), _) => rsx! {
                    EventTable { events: l.clone() }
                    EventCards { events: l.clone() }
                    if more() {
                        div { class: "mt-4 flex justify-center",
                            Button { variant: ButtonVariant::Outline, size: ButtonSize::Sm, disabled: loading_more(), onclick: load_more,
                                if loading_more() { "Lade …" } else { "Ältere laden" }
                            }
                        }
                    }
                },
                (None, Some(Err(e))) => rsx! {
                    EmptyState { title: "Ereignisse nicht geladen", description: "{e}", icon: rsx! { icons::TriangleAlert {} } }
                },
                _ => rsx! { LoadingState {} },
            }
        }
    }
}

fn when(ts_ms: i64) -> String {
    fmt::point(ts_ms / 1000, 1)
}

#[component]
fn SevDot(s: Severity) -> Element {
    let (label, tone) = severity_text(s);
    rsx! { StatusDot { tone, label } }
}

/// Desktop: Tabelle.
#[component]
fn EventTable(events: Vec<Event>) -> Element {
    rsx! {
        div { class: "hidden md:block",
            TableWrapper { class: "max-h-none rounded-xl bg-card shadow-sm",
                Table { class: "max-w-none",
                    TableHeader {
                        TableRow {
                            TableHead { class: "w-36", "Zeit" }
                            TableHead { class: "w-24", "Schwere" }
                            TableHead { class: "w-24", "Quelle" }
                            TableHead { class: "w-40", "Art" }
                            TableHead { class: "w-44", "Objekt" }
                            TableHead { "Ereignis" }
                        }
                    }
                    TableBody {
                        for e in events.iter() {
                            TableRow { key: "{e.id}-{e.ts}",
                                TableCell { class: "px-2 py-2.5 align-top whitespace-nowrap text-xs tabular-nums text-muted-foreground",
                                    span { title: "{fmt::full_ms(e.ts)}", "{when(e.ts)}" }
                                }
                                TableCell { class: "px-2 py-2.5 align-top", SevDot { s: e.severity } }
                                TableCell { class: "px-2 py-2.5 align-top text-xs", "{e.source}" }
                                TableCell { class: "px-2 py-2.5 align-top font-mono text-xs text-muted-foreground", "{e.kind}" }
                                TableCell { class: "px-2 py-2.5 align-top text-xs break-all", "{e.subject}" }
                                TableCell { class: "px-2 py-2.5 align-top",
                                    p { class: "font-medium", "{e.title}" }
                                    if !e.message.is_empty() {
                                        p { class: "mt-0.5 text-xs text-muted-foreground", "{e.message}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Handy: Karten.
#[component]
fn EventCards(events: Vec<Event>) -> Element {
    rsx! {
        div { class: "grid gap-2 md:hidden",
            for e in events.iter() {
                div { key: "{e.id}-{e.ts}", class: "rounded-xl border bg-card p-3 shadow-sm",
                    div { class: "flex items-center justify-between gap-2",
                        SevDot { s: e.severity }
                        span { class: "text-xs tabular-nums text-muted-foreground", title: "{fmt::full_ms(e.ts)}", "{when(e.ts)}" }
                    }
                    p { class: "mt-1.5 text-sm font-medium", "{e.title}" }
                    if !e.message.is_empty() {
                        p { class: "mt-0.5 text-xs text-muted-foreground", "{e.message}" }
                    }
                    p { class: "mt-2 truncate text-xs text-muted-foreground",
                        "{e.source} · "
                        span { class: "font-mono", "{e.kind}" }
                        if !e.subject.is_empty() {
                            " · {e.subject}"
                        }
                    }
                }
            }
        }
    }
}
