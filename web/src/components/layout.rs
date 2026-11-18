//! Rahmen der App: Seitenleiste mit allen Bereichen (mit Zählern), Zeitraum-Leiste über dem Inhalt,
//! Befehlspalette (⌘K / Strg+K), Leiste unten am Handy, Fenstertitel je Seite und die globalen
//! Ebenen der Diagramme (Vergrösserungs-Dialog, Ereignis-Marken).

use std::{cell::RefCell, rc::Rc};

use dioxus::{prelude::*, web::WebEventExt};
use futures::{StreamExt, channel::mpsc};
use shared::{
    alerts::ActiveAlert,
    catalog::{self, docker::*, system::*},
    model::ChartMeta,
};
use wasm_bindgen::{JsCast, closure::Closure};

use crate::{
    api,
    components::{
        chart::{ChartDialog, expand, marks::MarksSource, nav},
        ui::{app_shell::*, kbd::Kbd},
    },
    fmt,
    live::{use_connected, use_snap},
    pages,
    route::Route,
    state::{App, PRESETS, TimeRange, use_app},
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Befehlspalette geöffnet.
pub static PALETTE: GlobalSignal<bool> = Signal::global(|| false);

#[component]
fn Logo() -> Element {
    rsx! {
        svg { view_box: "0 0 64 64", class: "size-7 rounded-lg",
            rect { width: "64", height: "64", rx: "14", fill: "var(--muted)" }
            path {
                d: "M10 38h10l5-16 8 28 7-20 4 8h10",
                fill: "none",
                stroke: "var(--foreground)",
                stroke_width: "4.5",
                stroke_linecap: "round",
                stroke_linejoin: "round",
            }
        }
    }
}

const SPAN_KEY: &str = "nrzmon.range";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// Standard-Zeitraum beim Öffnen (Sekunden, im Browser gespeichert).
pub fn default_span() -> i64 {
    storage().and_then(|s| s.get_item(SPAN_KEY).ok().flatten()).and_then(|v| v.parse().ok()).filter(|v| *v >= 60).unwrap_or(900)
}

pub fn set_default_span(secs: i64) {
    if let Some(s) = storage() {
        let _ = s.set_item(SPAN_KEY, &secs.to_string());
    }
}

/// Name einer Seite (für Fenstertitel und Palette).
fn page_name(r: &Route) -> String {
    match r {
        Route::Overview {} => "Übersicht".into(),
        Route::Metrics {} => "Alle Metriken".into(),
        Route::Settings {} => "Einstellungen".into(),
        Route::Cpu {} => "CPU".into(),
        Route::Memory {} => "Arbeitsspeicher".into(),
        Route::Disks {} => "Datenträger".into(),
        Route::Network {} => "Netzwerk".into(),
        Route::Sensors {} => "Sensoren".into(),
        Route::Processes {} => "Prozesse".into(),
        Route::Power {} => "Strom".into(),
        Route::Containers {} => "Container".into(),
        Route::Container { name } => format!("{name} · Container"),
        Route::Apps {} => "Apps".into(),
        Route::AppDetail { name } => format!("{name} · Apps"),
        Route::Alerts {} => "Alarme".into(),
        Route::Dns {} => "DNS".into(),
        Route::DnsQueries {} => "DNS-Anfragen".into(),
        Route::Cloudflare {} => "Cloudflare".into(),
        Route::Netbird {} => "NetBird".into(),
        Route::Probes {} => "Erreichbarkeit".into(),
        Route::Explore {} => "Erkunden".into(),
        Route::Events {} => "Ereignisse".into(),
        Route::Storage {} => "Speicher".into(),
        _ => "nrzMonitor".into(),
    }
}

/// Seiten für Palette und Navigation: (Name, Pfad, Bereich, Suchwörter).
const PAGES: &[(&str, &str, &str, &str)] = &[
    ("Übersicht", "/", "", "start dashboard host"),
    ("CPU", "/system/cpu", "System", "prozessor kerne takt last"),
    ("Arbeitsspeicher", "/system/memory", "System", "ram speicher swap memory"),
    ("Datenträger", "/system/disks", "System", "disk festplatte ssd nvme belegung"),
    ("Netzwerk", "/system/network", "System", "network schnittstelle tcp udp"),
    ("Sensoren", "/system/sensors", "System", "temperatur lüfter spannung"),
    ("Prozesse", "/system/processes", "System", "programme process"),
    ("Strom", "/system/power", "System", "power watt kwh kosten euro verbrauch energie"),
    ("Container", "/containers", "Container", "docker"),
    ("Apps", "/apps", "Container", "compose umbrel"),
    ("DNS", "/dns", "Netzwerk", "nrznetwork anfragen"),
    ("DNS-Anfragen", "/dns/queries", "Netzwerk", "query log"),
    ("Cloudflare", "/cloudflare", "Netzwerk", "tunnel"),
    ("NetBird", "/netbird", "Netzwerk", "vpn peers"),
    ("Erreichbarkeit", "/probes", "Netzwerk", "ping http probe"),
    ("Alle Metriken", "/metrics", "Analyse", "diagramme metrics"),
    ("Erkunden", "/explore", "Analyse", "explore abfrage"),
    ("Ereignisse", "/events", "Analyse", "events log"),
    ("Alarme", "/alerts", "Analyse", "alerts warnung"),
    ("Speicher", "/storage", "Verwaltung", "duckdb parquet aufbewahrung"),
    ("Einstellungen", "/settings", "Verwaltung", "settings konfiguration"),
];

/// Layout aller Seiten. Ohne gültige Anmeldung erscheint die Anmeldeseite.
#[component]
pub fn Shell() -> Element {
    let app = use_app();
    if api::UNAUTHORIZED() {
        return rsx! { pages::login::Login {} };
    }
    let route = use_route::<Route>();
    let alerts = use_snap::<Vec<ActiveAlert>>("alerts.active");
    // Gespeicherten Standard-Zeitraum einmal beim Start anwenden.
    use_hook(move || {
        let span = default_span();
        if span != 900 && *app.range.peek() == TimeRange::live(900) {
            app.set_range(TimeRange::live(span));
        }
    });
    let info = app.info.read();
    let host = if info.hostname.is_empty() { "verbinde …".to_string() } else { info.hostname.clone() };
    let subtitle = if info.demo { format!("{host} · Beispieldaten") } else { host };
    let r = &route;
    let title = format!("{} · nrzMonitor", page_name(r));

    // Zähler aus dem Katalog: nur, was gerade liefert.
    let charts = app.charts.read();
    let count = |ctx: &str| {
        let n = charts.iter().filter(|c| c.context == ctx && !c.retired).count();
        (n > 0).then_some(n)
    };
    let (n_containers, n_apps, n_disks, n_ifaces) = (count(DOCKER_CPU.context), count(APP_CPU.context), count(DISK_IO.context), count(NET_TRAFFIC.context));
    let n_charts = (!charts.is_empty()).then(|| charts.iter().filter(|c| !c.retired).count());
    drop(charts);
    let n_alerts = alerts.read().as_ref().map_or(0, Vec::len);

    rsx! {
        document::Title { "{title}" }
        AppShell {
            header_brand: rsx! {
                Logo {}
                span { class: "truncate", "nrzMonitor" }
            },
            header_actions: rsx! {
                button {
                    r#type: "button",
                    class: "inline-flex size-9 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 [&_svg]:size-[18px]",
                    title: "Suchen",
                    "aria-label": "Suchen",
                    onclick: move |_| *PALETTE.write() = true,
                    icons::Search {}
                }
                ConnectionDot {}
            },
            sidebar: rsx! {
                AppBrand { logo: rsx! { Logo {} }, name: "nrzMonitor", subtitle, href: "/" }
                div { class: "px-3 pb-2",
                    button {
                        r#type: "button",
                        class: "flex h-8 w-full items-center gap-2 rounded-md border bg-background/40 px-2.5 text-xs text-muted-foreground outline-none transition-colors hover:bg-sidenav-accent/60 hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 [&_svg]:size-3.5",
                        onclick: move |_| *PALETTE.write() = true,
                        icons::Search {}
                        span { class: "flex-1 text-left", "Suchen …" }
                        Kbd { "⌘K" }
                    }
                }
                AppNav {
                    AppNavItem { label: "Übersicht", href: "/", active: matches!(r, Route::Overview {}), icon: rsx! { icons::LayoutDashboard {} } }

                    AppNavHeading { label: "System" }
                    AppNavItem { label: "CPU", href: "/system/cpu", active: matches!(r, Route::Cpu {}), icon: rsx! { icons::Cpu {} } }
                    AppNavItem { label: "Arbeitsspeicher", href: "/system/memory", active: matches!(r, Route::Memory {}), icon: rsx! { icons::MemoryStick {} } }
                    AppNavItem { label: "Datenträger", href: "/system/disks", count: n_disks, active: matches!(r, Route::Disks {}), icon: rsx! { icons::HardDrive {} } }
                    AppNavItem { label: "Netzwerk", href: "/system/network", count: n_ifaces, active: matches!(r, Route::Network {}), icon: rsx! { icons::Network {} } }
                    AppNavItem { label: "Sensoren", href: "/system/sensors", active: matches!(r, Route::Sensors {}), icon: rsx! { icons::Thermometer {} } }
                    AppNavItem { label: "Prozesse", href: "/system/processes", active: matches!(r, Route::Processes {}), icon: rsx! { icons::ListTree {} } }
                    AppNavItem { label: "Strom", href: "/system/power", active: matches!(r, Route::Power {}), icon: rsx! { icons::Zap {} } }

                    AppNavHeading { label: "Container" }
                    AppNavItem { label: "Container", href: "/containers", count: n_containers, active: matches!(r, Route::Containers {} | Route::Container { .. }), icon: rsx! { icons::Container {} } }
                    AppNavItem { label: "Apps", href: "/apps", count: n_apps, active: matches!(r, Route::Apps {} | Route::AppDetail { .. }), icon: rsx! { icons::LayoutGrid {} } }

                    AppNavHeading { label: "Netzwerk" }
                    AppNavItem { label: "DNS", href: "/dns", active: matches!(r, Route::Dns {} | Route::DnsQueries {}), icon: rsx! { icons::Globe {} } }
                    AppNavItem { label: "Cloudflare", href: "/cloudflare", active: matches!(r, Route::Cloudflare {}), icon: rsx! { icons::Cloud {} } }
                    AppNavItem { label: "NetBird", href: "/netbird", active: matches!(r, Route::Netbird {}), icon: rsx! { icons::Waypoints {} } }
                    AppNavItem { label: "Erreichbarkeit", href: "/probes", active: matches!(r, Route::Probes {}), icon: rsx! { icons::Radar {} } }

                    AppNavHeading { label: "Analyse" }
                    AppNavItem { label: "Alle Metriken", href: "/metrics", count: n_charts, active: matches!(r, Route::Metrics {}), icon: rsx! { icons::ChartLine {} } }
                    AppNavItem { label: "Erkunden", href: "/explore", active: matches!(r, Route::Explore {}), icon: rsx! { icons::Telescope {} } }
                    AppNavItem { label: "Ereignisse", href: "/events", active: matches!(r, Route::Events {}), icon: rsx! { icons::History {} } }
                    AppNavItem { label: "Alarme", href: "/alerts", badge: n_alerts, active: matches!(r, Route::Alerts {}), icon: rsx! { icons::Bell {} } }
                }
                AppSidebarFooter {
                    AppNavItem { label: "Speicher", href: "/storage", active: matches!(r, Route::Storage {}), icon: rsx! { icons::Database {} } }
                    AppNavItem { label: "Einstellungen", href: "/settings", active: matches!(r, Route::Settings {}), icon: rsx! { icons::Settings {} } }
                    AppVersion { text: "nrzMonitor {VERSION}" }
                }
            },
            TimeBar {}
            div { class: "pb-20 lg:pb-0", Outlet::<Route> {} }
            BottomNav { alerts: n_alerts }
        }
        MarksSource {}
        ChartDialog {}
        CommandPalette {}
    }
}

/// Zustand der Live-Verbindung als Punkt (Handy-Kopfzeile).
#[component]
fn ConnectionDot() -> Element {
    let connected = use_connected();
    let (dot, text) = if connected() { ("bg-success", "Live verbunden") } else { ("bg-warning animate-pulse", "Verbinde …") };
    rsx! {
        span { class: "grid size-9 place-items-center", title: "{text}", "aria-label": "{text}", role: "status",
            span { class: "size-2 rounded-full {dot}" }
        }
    }
}

/// Punkt mit Text: steht die Live-Verbindung?
#[component]
fn ConnectionState() -> Element {
    let connected = use_connected();
    if connected() {
        return rsx! {};
    }
    rsx! {
        span { class: "inline-flex items-center gap-1.5 text-xs text-muted-foreground", role: "status",
            span { class: "size-2 animate-pulse rounded-full bg-warning" }
            "Verbinde …"
        }
    }
}

const BAR_BTN: &str = "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border bg-card px-2 text-xs outline-none transition-colors hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-40 [&_svg]:size-3.5";

/// Zeitraum für alle Diagramme der Seite – eine Zeile über dem Inhalt (am Handy waagrecht scrollbar).
#[component]
pub fn TimeBar() -> Element {
    let app = use_app();
    let range = (app.range)();
    let live = range.is_live();
    let mut custom = use_signal(|| false);
    let can_back = nav::can_back(app);
    let preset_active = |secs: i64| live && range.span() == secs;
    let span_text = if live {
        PRESETS.iter().find(|(_, s)| *s == range.span()).map_or_else(|| format!("letzte {}", shared::model::duration(range.span() as f64)), |(l, _)| format!("letzte {l}"))
    } else {
        let (a, b) = range.resolve(fmt::now());
        if fmt::day(a) == fmt::day(b) { format!("{} {} – {}", fmt::day(a), fmt::hm(a), fmt::hm(b)) } else { format!("{} {} – {} {}", fmt::day(a), fmt::hm(a), fmt::day(b), fmt::hm(b)) }
    };

    rsx! {
        div { class: "sticky top-0 z-20 border-b bg-background/85 backdrop-blur", "data-name": "TimeBar",
            div { class: "flex items-center gap-2 px-3 py-2 md:px-8",
            div { class: "flex min-w-0 flex-1 items-center gap-2 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
                div { class: "inline-flex shrink-0 rounded-md border bg-card p-0.5", role: "group", "aria-label": "Zeitraum",
                    for (label, secs) in PRESETS.iter().copied() {
                        button {
                            key: "{secs}",
                            r#type: "button",
                            class: "rounded px-2 py-1 text-xs whitespace-nowrap tabular-nums outline-none transition-colors focus-visible:ring-[3px] focus-visible:ring-ring/50",
                            class: if preset_active(secs) { "bg-accent font-medium text-accent-foreground" } else { "text-muted-foreground hover:text-foreground" },
                            "aria-pressed": "{preset_active(secs)}",
                            onclick: move |_| nav::set(app, TimeRange::live(secs)),
                            "{label}"
                        }
                    }
                }
                div { class: "relative shrink-0",
                    button {
                        r#type: "button",
                        class: "{BAR_BTN}",
                        class: if custom() { "bg-accent" },
                        title: "Eigener Zeitraum",
                        "aria-expanded": "{custom()}",
                        onclick: move |_| custom.toggle(),
                        icons::CalendarRange {}
                        span { class: "hidden max-w-56 truncate tabular-nums sm:inline", "{span_text}" }
                    }
                }
                div { class: "inline-flex shrink-0 rounded-md border bg-card",
                    button {
                        r#type: "button",
                        class: "inline-flex size-8 items-center justify-center rounded-l-md text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 [&_svg]:size-4",
                        title: "Früher (halbe Länge zurück)",
                        "aria-label": "Früher",
                        onclick: move |_| nav::pan(app, -0.5),
                        icons::ChevronLeft {}
                    }
                    button {
                        r#type: "button",
                        class: "inline-flex size-8 items-center justify-center rounded-r-md border-l text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-40 [&_svg]:size-4",
                        title: "Später (halbe Länge vor)",
                        "aria-label": "Später",
                        disabled: live,
                        onclick: move |_| nav::pan(app, 0.5),
                        icons::ChevronRight {}
                    }
                }
                if can_back {
                    button {
                        r#type: "button",
                        class: "{BAR_BTN}",
                        title: "Zurück zum vorherigen Zeitraum",
                        onclick: move |_| nav::back(app),
                        icons::Undo2 {}
                        span { class: "hidden sm:inline", "Zurück" }
                    }
                }
                div { class: "flex-1" }
                span { class: "hidden shrink-0 text-xs text-muted-foreground xl:inline", "Ziehen zoomt · Doppelklick zurück" }
            }
                ConnectionState {}
                if live {
                    button {
                        r#type: "button",
                        class: "{BAR_BTN}",
                        title: "Anhalten: den aktuellen Zeitraum einfrieren",
                        onclick: move |_| nav::pause(app),
                        span { class: "relative flex size-2",
                            span { class: "absolute inline-flex size-full animate-ping rounded-full bg-success opacity-60 motion-reduce:hidden" }
                            span { class: "relative inline-flex size-2 rounded-full bg-success" }
                        }
                        span { class: "hidden sm:inline", "Live" }
                        icons::Pause { class: "text-muted-foreground" }
                    }
                } else {
                    button {
                        r#type: "button",
                        class: "{BAR_BTN}",
                        title: "Wieder live mitlaufen",
                        onclick: move |_| nav::resume(app),
                        span { class: "size-2 rounded-full bg-muted-foreground" }
                        span { class: "hidden sm:inline", "Angehalten" }
                        icons::Play { class: "text-muted-foreground" }
                    }
                }
            }
            if custom() {
                CustomRange { onclose: move |_| custom.set(false) }
            }
        }
    }
}

/// `2026-11-15T14:03` (Ortszeit) für `datetime-local`.
fn local_input(ts: i64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts as f64 * 1000.0));
    format!("{}-{:02}-{:02}T{:02}:{:02}", d.get_full_year(), d.get_month() + 1, d.get_date(), d.get_hours(), d.get_minutes())
}

fn parse_local(s: &str) -> Option<i64> {
    let ms = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(s)).get_time();
    ms.is_finite().then(|| (ms / 1000.0) as i64)
}

/// Mitternacht (Ortszeit) des Tages von `ts`.
fn midnight(ts: i64) -> i64 {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts as f64 * 1000.0));
    d.set_hours(0);
    d.set_minutes(0);
    d.set_seconds(0);
    d.set_milliseconds(0);
    (d.get_time() / 1000.0) as i64
}

/// Schnelle Auswahl: liefert (Beginn, Ende); Ende 0 = live bis jetzt.
type Quick = Rc<dyn Fn() -> (i64, i64)>;

/// Eigener Zeitraum: Von/Bis und schnelle Auswahl.
#[component]
fn CustomRange(onclose: EventHandler<()>) -> Element {
    let app = use_app();
    let (a0, b0) = app.range.peek().resolve(fmt::now());
    let mut from = use_signal(|| local_input(a0));
    let mut to = use_signal(|| local_input(b0));
    let mut error = use_signal(|| None::<&'static str>);
    let apply = move |_| {
        match (parse_local(&from()), parse_local(&to())) {
            (Some(a), Some(b)) if b > a => {
                nav::zoom(app, a, b.min(fmt::now()));
                onclose.call(());
            }
            _ => error.set(Some("Bitte gültige Zeiten wählen (Bis nach Von).")),
        }
    };
    let quick: [(&str, Quick); 6] = [
        ("Heute", Rc::new(|| (midnight(fmt::now()), fmt::now()))),
        ("Gestern", Rc::new(|| (midnight(fmt::now()) - 86_400, midnight(fmt::now())))),
        ("Letzte 3 h", Rc::new(|| (fmt::now() - 3 * 3600, 0))),
        ("Letzte 12 h", Rc::new(|| (fmt::now() - 12 * 3600, 0))),
        ("Letzte 3 d", Rc::new(|| (fmt::now() - 3 * 86_400, 0))),
        ("Letzte 90 d", Rc::new(|| (fmt::now() - 90 * 86_400, 0))),
    ];
    rsx! {
        div { class: "border-t bg-card/95 px-3 py-3 animate-nrz-enter md:px-8",
            onkeydown: move |e| {
                if e.key() == Key::Escape {
                    onclose.call(());
                }
            },
            div { class: "flex flex-wrap items-end gap-3",
                div { class: "flex flex-wrap gap-1.5",
                    for (label, f) in quick.into_iter() {
                        button {
                            key: "{label}",
                            r#type: "button",
                            class: "rounded-md border bg-background px-2.5 py-1.5 text-xs outline-none hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50",
                            onclick: move |_| {
                                let (a, b) = f();
                                if b == 0 {
                                    nav::set(app, TimeRange::live(fmt::now() - a));
                                } else {
                                    nav::zoom(app, a, b);
                                }
                                onclose.call(());
                            },
                            "{label}"
                        }
                    }
                }
                div { class: "flex-1" }
                label { class: "grid gap-1 text-xs text-muted-foreground",
                    "Von"
                    input {
                        r#type: "datetime-local",
                        class: "h-8 rounded-md border bg-background px-2 text-xs text-foreground tabular-nums outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                        value: "{from}",
                        oninput: move |e| from.set(e.value()),
                    }
                }
                label { class: "grid gap-1 text-xs text-muted-foreground",
                    "Bis"
                    input {
                        r#type: "datetime-local",
                        class: "h-8 rounded-md border bg-background px-2 text-xs text-foreground tabular-nums outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                        value: "{to}",
                        oninput: move |e| to.set(e.value()),
                    }
                }
                button {
                    r#type: "button",
                    class: "h-8 rounded-md bg-primary px-3 text-xs font-medium text-primary-foreground outline-none hover:bg-primary/90 focus-visible:ring-[3px] focus-visible:ring-ring/50",
                    onclick: apply,
                    "Übernehmen"
                }
                button {
                    r#type: "button",
                    class: "h-8 rounded-md px-2 text-xs text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50",
                    onclick: move |_| onclose.call(()),
                    "Abbrechen"
                }
            }
            if let Some(e) = error() {
                p { class: "mt-2 text-xs text-destructive", "{e}" }
            }
        }
    }
}

/// Leiste unten am Handy: die wichtigsten Bereiche und die Suche.
#[component]
fn BottomNav(alerts: usize) -> Element {
    let route = use_route::<Route>();
    let item = "relative flex flex-1 flex-col items-center justify-center gap-0.5 text-[10px] outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 [&_svg]:size-5";
    let tone = |on: bool| if on { "text-foreground" } else { "text-muted-foreground" };
    let r = &route;
    let t_over = tone(matches!(r, Route::Overview {}));
    let t_cont = tone(matches!(r, Route::Containers {} | Route::Container { .. }));
    let t_metr = tone(matches!(r, Route::Metrics {}));
    let t_alrt = tone(matches!(r, Route::Alerts {}));
    rsx! {
        nav { class: "fixed inset-x-0 bottom-0 z-30 flex h-16 border-t bg-sidenav/95 backdrop-blur safe-bottom lg:hidden", "aria-label": "Hauptbereiche",
            Link { to: "/", class: "{item} {t_over}", icons::LayoutDashboard {} "Übersicht" }
            Link { to: "/containers", class: "{item} {t_cont}", icons::Container {} "Container" }
            Link { to: "/metrics", class: "{item} {t_metr}", icons::ChartLine {} "Metriken" }
            Link { to: "/alerts", class: "{item} {t_alrt}",
                icons::Bell {}
                "Alarme"
                if alerts > 0 {
                    span { class: "absolute top-2 left-1/2 ml-1 rounded-full bg-destructive px-1 text-[10px] leading-4 font-semibold text-destructive-foreground tabular-nums", "{alerts}" }
                }
            }
            button { r#type: "button", class: "{item} text-muted-foreground", onclick: move |_| *PALETTE.write() = true, icons::Search {} "Suchen" }
        }
    }
}

/// Eintrag der Befehlspalette.
#[derive(Clone, PartialEq)]
enum Action {
    Go(String),
    Chart(String),
    Range(i64),
    Pause,
    Resume,
    Back,
}

#[derive(Clone, PartialEq)]
struct Entry {
    group: &'static str,
    title: String,
    detail: String,
    action: Action,
}

/// Passt der Eintrag zur Suche? Jedes Wort muss irgendwo vorkommen.
fn matches(words: &[String], hay: &str) -> bool {
    words.iter().all(|w| hay.contains(w.as_str()))
}

fn entries(app: App, query: &str) -> Vec<Entry> {
    let q = query.trim().to_lowercase();
    let words: Vec<String> = q.split_whitespace().map(str::to_string).collect();
    let mut out = Vec::new();
    for (name, path, area, keys) in PAGES {
        if matches(&words, &format!("{} {} {} {}", name, area, keys, path).to_lowercase()) {
            out.push(Entry { group: "Seiten", title: (*name).into(), detail: (*area).into(), action: Action::Go((*path).into()) });
        }
    }
    let charts = app.charts.read();
    let mut containers: Vec<&ChartMeta> = charts.iter().filter(|c| c.context == DOCKER_CPU.context && !c.retired).collect();
    containers.sort_by(|a, b| a.label("container").cmp(&b.label("container")));
    let mut n = 0;
    for c in containers {
        let Some(name) = c.label("container") else { continue };
        let image = c.label("image").unwrap_or("");
        if !words.is_empty() && matches(&words, &format!("{name} {image} container").to_lowercase()) || words.is_empty() && n < 5 {
            out.push(Entry { group: "Container", title: name.into(), detail: image.into(), action: Action::Go(format!("/containers/{name}")) });
            n += 1;
            if n >= 8 {
                break;
            }
        }
    }
    if !words.is_empty() {
        let mut k = 0;
        for c in charts.iter().filter(|c| !c.retired) {
            let section = catalog::section_title(&c.section);
            let hay = format!("{} {} {} {} {}", c.title, c.family, section, c.id, c.labels.values().cloned().collect::<Vec<_>>().join(" ")).to_lowercase();
            if matches(&words, &hay) {
                let detail = [section, c.family.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                out.push(Entry { group: "Diagramme", title: c.title.clone(), detail, action: Action::Chart(c.id.clone()) });
                k += 1;
                if k >= 40 {
                    break;
                }
            }
        }
    }
    drop(charts);
    let live = app.range.peek().is_live();
    let mut actions: Vec<Entry> = PRESETS
        .iter()
        .map(|(l, s)| Entry { group: "Zeitraum", title: format!("Zeitraum: letzte {l}"), detail: "live".into(), action: Action::Range(*s) })
        .collect();
    actions.push(if live {
        Entry { group: "Zeitraum", title: "Live anhalten".into(), detail: "Zeitraum einfrieren".into(), action: Action::Pause }
    } else {
        Entry { group: "Zeitraum", title: "Wieder live".into(), detail: "mitlaufen".into(), action: Action::Resume }
    });
    if nav::can_back(app) {
        actions.push(Entry { group: "Zeitraum", title: "Zurück".into(), detail: "vorheriger Zeitraum".into(), action: Action::Back });
    }
    out.extend(actions.into_iter().filter(|e| !words.is_empty() && matches(&words, &format!("{} {} zeitraum", e.title, e.detail).to_lowercase())));
    out
}

fn run(app: App, a: &Action) {
    match a {
        Action::Go(p) => {
            navigator().push(p.as_str());
        }
        Action::Chart(id) => expand(id.clone()),
        Action::Range(s) => nav::set(app, TimeRange::live(*s)),
        Action::Pause => nav::pause(app),
        Action::Resume => nav::resume(app),
        Action::Back => nav::back(app),
    }
}

type KeyWatch = Closure<dyn FnMut(web_sys::KeyboardEvent)>;

/// Befehlspalette: Seiten, Container, Diagramme und Zeitraum finden – ⌘K / Strg+K oder „/“.
#[component]
fn CommandPalette() -> Element {
    let app = use_app();
    let mut query = use_signal(String::new);
    let mut selected = use_signal(|| 0usize);
    // Tastenkürzel am Dokument: Meldung über einen Kanal herein (der Rückruf läuft ausserhalb).
    let watch = use_hook(|| Rc::new(RefCell::new(None::<KeyWatch>)));
    use_hook({
        let watch = watch.clone();
        move || {
            let (tx, mut rx) = mpsc::unbounded::<()>();
            let cb = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
                let k = e.key().to_lowercase();
                let typing = e
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok())
                    .is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT") || el.is_content_editable());
                if (k == "k" && (e.meta_key() || e.ctrl_key())) || (k == "/" && !typing) {
                    e.prevent_default();
                    let _ = tx.unbounded_send(());
                }
            });
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                let _ = doc.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
            }
            *watch.borrow_mut() = Some(cb);
            spawn(async move {
                while rx.next().await.is_some() {
                    let open = *PALETTE.peek();
                    *PALETTE.write() = !open;
                }
            });
        }
    });
    use_drop(move || {
        if let (Some(cb), Some(doc)) = (watch.borrow_mut().take(), web_sys::window().and_then(|w| w.document())) {
            let _ = doc.remove_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
        }
    });

    if !PALETTE() {
        return rsx! {};
    }
    let list = entries(app, &query());
    let count = list.len();
    let sel = selected().min(count.saturating_sub(1));
    let mut close = move || {
        *PALETTE.write() = false;
        query.set(String::new());
        selected.set(0);
    };
    let list_key = list.clone();
    let groups: Vec<&'static str> = list.iter().map(|e| e.group).collect();

    rsx! {
        div { class: "fixed inset-0 z-[60] bg-black/60 animate-nrz-fade", onclick: move |_| close() }
        div {
            class: "fixed inset-x-3 top-[10vh] z-[61] mx-auto flex max-h-[70vh] max-w-xl flex-col overflow-hidden rounded-xl border bg-popover text-popover-foreground shadow-2xl animate-nrz-enter",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Befehlspalette",
            div { class: "flex items-center gap-2 border-b px-3",
                icons::Search { class: "size-4 shrink-0 text-muted-foreground" }
                input {
                    class: "h-12 w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground",
                    placeholder: "Seite, Container oder Diagramm suchen …",
                    value: "{query}",
                    "aria-label": "Suchen",
                    autofocus: true,
                    onmounted: move |e| {
                        let _ = e.data().as_web_event().dyn_into::<web_sys::HtmlElement>().map(|el| el.focus());
                    },
                    oninput: move |e| {
                        query.set(e.value());
                        selected.set(0);
                    },
                    onkeydown: move |e| match e.key() {
                        Key::ArrowDown => {
                            e.prevent_default();
                            selected.set((sel + 1).min(count.saturating_sub(1)));
                        }
                        Key::ArrowUp => {
                            e.prevent_default();
                            selected.set(sel.saturating_sub(1));
                        }
                        Key::Enter => {
                            if let Some(en) = list_key.get(sel) {
                                run(app, &en.action);
                                close();
                            }
                        }
                        Key::Escape => close(),
                        _ => {}
                    },
                }
                Kbd { "Esc" }
            }
            div { class: "min-h-0 flex-1 overflow-y-auto p-1.5",
                if list.is_empty() {
                    p { class: "px-3 py-8 text-center text-sm text-muted-foreground", "Nichts gefunden." }
                }
                for (i, e) in list.into_iter().enumerate() {
                    {
                        let first = i == 0 || groups[i - 1] != e.group;
                        let action = e.action.clone();
                        let icon = match &e.action {
                            Action::Go(_) if e.group == "Container" => rsx! { icons::Container {} },
                            Action::Go(_) => rsx! { icons::ArrowUpRight {} },
                            Action::Chart(_) => rsx! { icons::ChartLine {} },
                            _ => rsx! { icons::Clock {} },
                        };
                        rsx! {
                            Fragment { key: "{i}",
                            if first {
                                p { class: "px-2 pt-2 pb-1 text-[11px] font-medium tracking-wider text-muted-foreground uppercase", "{e.group}" }
                            }
                            button {
                                r#type: "button",
                                class: "flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm outline-none [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-muted-foreground",
                                class: if i == sel { "bg-accent text-accent-foreground" } else { "hover:bg-accent/50" },
                                onmouseenter: move |_| selected.set(i),
                                onclick: move |_| {
                                    run(app, &action);
                                    close();
                                },
                                onmounted: move |el| {
                                    if i == sel {
                                        el.data().as_web_event().scroll_into_view_with_bool(false);
                                    }
                                },
                                {icon}
                                span { class: "truncate", "{e.title}" }
                                span { class: "ml-auto truncate pl-3 text-xs text-muted-foreground", "{e.detail}" }
                            }
                            }
                        }
                    }
                }
            }
            div { class: "flex items-center gap-3 border-t bg-muted/40 px-3 py-2 text-[11px] text-muted-foreground",
                span { class: "inline-flex items-center gap-1", Kbd { "↑" } Kbd { "↓" } "auswählen" }
                span { class: "inline-flex items-center gap-1", Kbd { "↵" } "öffnen" }
                span { class: "ml-auto", "{count} Treffer" }
            }
        }
    }
}
