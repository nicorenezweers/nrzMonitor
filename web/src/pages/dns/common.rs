//! Gemeinsame Bausteine der Netzwerk-Seiten (DNS, Cloudflare, NetBird, Erreichbarkeit).

use dioxus::prelude::*;
use serde::de::DeserializeOwned;
use shared::{dns::TopEntry, model::Unit};

use crate::{
    api,
    components::{
        chart::color,
        ui::{
            app_shell::EmptyState,
            badge::{Badge, BadgeVariant},
            button::{Button, ButtonVariant},
        },
    },
    fmt,
    route::Route,
    state::App,
};

/// Zeitraum der Seite als (von, bis) in Unix-Sekunden (liest das Signal → reaktiv).
pub fn range_of(app: App) -> (i64, i64) {
    let r = *app.range.read();
    r.resolve(fmt::now())
}

/// `from`/`to` als Query-Parameter.
pub fn range_params(app: App) -> Vec<(&'static str, String)> {
    let (a, b) = range_of(app);
    vec![("from", a.to_string()), ("to", b.to_string())]
}

/// Daten eines eigenen Endpunkts laden. `path` wird reaktiv gelesen (Signale darin lösen neu
/// laden aus); `every_ms > 0` lädt zusätzlich regelmässig. Beim Neuladen bleibt das alte Bild stehen.
pub fn use_api<T: DeserializeOwned + Clone + 'static>(
    path: impl FnMut() -> String + 'static,
    every_ms: u32,
) -> (Signal<Option<T>>, Signal<Option<String>>, Signal<u64>) {
    let active = use_signal(|| true);
    use_api_live(path, every_ms, active)
}

/// Wie [`use_api`]; das regelmässige Neuladen läuft nur, solange `active` wahr ist.
pub fn use_api_live<T: DeserializeOwned + Clone + 'static>(
    path: impl FnMut() -> String + 'static,
    every_ms: u32,
    active: Signal<bool>,
) -> (Signal<Option<T>>, Signal<Option<String>>, Signal<u64>) {
    let mut data = use_signal(|| None::<T>);
    let mut err = use_signal(|| None::<String>);
    let mut tick = use_signal(|| 0u64);
    use_future(move || async move {
        if every_ms == 0 {
            return;
        }
        loop {
            gloo_timers::future::TimeoutFuture::new(every_ms).await;
            if *active.peek() {
                tick += 1;
            }
        }
    });
    let mut path = path;
    let _ = use_resource(move || {
        let p = path();
        let _ = tick();
        async move {
            match api::get::<T>(&p).await {
                Ok(v) => {
                    data.set(Some(v));
                    err.set(None);
                }
                Err(e) => err.set(Some(e)),
            }
        }
    });
    (data, err, tick)
}

/// Freundlicher Leerzustand: Integration ist noch nicht eingerichtet.
#[component]
pub fn NotConfigured(#[props(into)] title: String, #[props(into)] description: String, icon: Element) -> Element {
    rsx! {
        EmptyState { title, description, icon,
            Button { variant: ButtonVariant::Default, onclick: move |_| { navigator().push(Route::Settings {}); },
                icons::Settings { class: "size-4" }
                "Zu den Einstellungen"
            }
        }
    }
}

/// Hinweis- oder Fehlerkasten (Zustandsfarbe mit Symbol und Text).
#[component]
pub fn Notice(#[props(into)] text: String, #[props(default)] error: bool) -> Element {
    let tone = if error { "border-destructive/40 bg-destructive/10" } else { "border-warning/40 bg-warning/10" };
    rsx! {
        div { class: "flex items-start gap-2 rounded-lg border px-3 py-2 text-sm {tone}",
            if error {
                icons::CircleAlert { class: "mt-0.5 size-4 shrink-0 text-destructive" }
            } else {
                icons::TriangleAlert { class: "mt-0.5 size-4 shrink-0 text-warning" }
            }
            span { class: "min-w-0 break-words", "{text}" }
        }
    }
}

/// Farbplatz einer Antwortquelle – wie die Dimensionen von `dns.queries`.
pub fn kind_slot(kind: &str) -> usize {
    shared::dns::KINDS.iter().position(|(k, _)| *k == kind).unwrap_or(8)
}

/// Antwortquelle als Marke: Farbmarke der Reihe + Text.
#[component]
pub fn KindBadge(#[props(into)] kind: String) -> Element {
    let label = shared::dns::kind_label(&kind).to_string();
    let c = color(kind_slot(&kind));
    rsx! {
        Badge { variant: BadgeVariant::Outline, class: "gap-1.5 font-normal whitespace-nowrap",
            span { class: "size-2 rounded-full", style: "background:{c}" }
            "{label}"
        }
    }
}

/// Rangliste als Balken. `blocked_slot`: Anteil „gesperrt“ in dieser Reihenfarbe eingefärbt.
#[component]
pub fn BarList(
    items: Vec<TopEntry>,
    #[props(default)] show_blocked: bool,
    #[props(optional)] onpick: Option<EventHandler<String>>,
    #[props(into, default = "Keine Daten im Zeitraum".to_string())] empty: String,
) -> Element {
    if items.is_empty() {
        return rsx! { p { class: "py-6 text-center text-sm text-muted-foreground", "{empty}" } };
    }
    let max = items.iter().map(|i| i.count).max().unwrap_or(1).max(1) as f64;
    let blocked_color = color(kind_slot("blocked"));
    rsx! {
        ul { class: "grid gap-1",
            for item in items.into_iter() {
                {
                    let w = item.count as f64 * 100.0 / max;
                    let bw = if item.count == 0 { 0.0 } else { item.blocked as f64 * 100.0 / item.count as f64 };
                    let name = item.name.clone();
                    let shown = if item.name.is_empty() { "(leer)".to_string() } else { item.name.clone() };
                    let clickable = onpick.is_some();
                    rsx! {
                        li { key: "{item.name}",
                            button {
                                class: "group relative flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted/60 disabled:cursor-default disabled:hover:bg-transparent",
                                disabled: !clickable,
                                title: "{item.name}",
                                onclick: move |_| if let Some(h) = &onpick { h.call(name.clone()) },
                                span { class: "absolute inset-y-0.5 left-0 overflow-hidden rounded-md bg-muted", style: "width:{w:.1}%",
                                    if show_blocked && item.blocked > 0 {
                                        span { class: "absolute inset-y-0 left-0 opacity-35", style: "width:{bw:.1}%; background:{blocked_color}" }
                                    }
                                }
                                span { class: "relative min-w-0 flex-1 truncate", "{shown}" }
                                span { class: "relative shrink-0 tabular-nums text-muted-foreground", {Unit::Count.format(item.count as f64)} }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Karte mit Titel für Ranglisten und Listen.
#[component]
pub fn ListCard(#[props(into)] title: String, #[props(into, optional)] hint: Option<String>, #[props(optional)] actions: Option<Element>, children: Element) -> Element {
    rsx! {
        div { class: "flex min-w-0 flex-col rounded-xl border bg-card shadow-sm",
            div { class: "flex items-center justify-between gap-2 border-b px-4 py-3",
                div { class: "min-w-0",
                    h3 { class: "truncate text-sm font-semibold", "{title}" }
                    if let Some(h) = hint {
                        p { class: "truncate text-xs text-muted-foreground", "{h}" }
                    }
                }
                if let Some(a) = actions {
                    div { class: "flex shrink-0 items-center gap-2", {a} }
                }
            }
            div { class: "min-w-0 p-2", {children} }
        }
    }
}

/// Prozent mit einer Nachkommastelle, `–` ohne Basis.
pub fn pct(part: u64, total: u64) -> String {
    if total == 0 { "–".into() } else { Unit::Percent.format(part as f64 * 100.0 / total as f64) }
}

/// Unix-Millisekunden als Uhrzeit (heute) oder Datum + Uhrzeit.
pub fn when_ms(ts_ms: i64) -> String {
    let ts = ts_ms / 1000;
    if fmt::now() - ts < 20 * 3600 { fmt::clock(ts) } else { fmt::full(ts) }
}

/// Momentaufnahme: live über den Kanal, bis dahin einmal per HTTP geholt (schneller erster Aufbau,
/// geht auch ohne WebSocket).
pub fn use_status<T: DeserializeOwned + PartialEq + Clone + 'static>(key: &'static str) -> Memo<Option<T>> {
    let live = crate::live::use_snap::<T>(key);
    let mut fetched = use_signal(|| None::<T>);
    let _ = use_resource(move || async move {
        if let Ok(s) = api::snap::<T>(key).await {
            fetched.set(Some(s.data));
        }
    });
    use_memo(move || live().or_else(|| fetched()))
}
