//! Seiten für Container und Apps (Compose-Projekte).
//!
//! Daten: Momentaufnahmen `docker.containers`, `docker.apps`, `docker.images`, `docker.volumes`,
//! `docker.networks`, `docker.info` und die Diagramme aus `shared::catalog::docker`.

use dioxus::prelude::*;
use serde::de::DeserializeOwned;
use shared::{docker::Container as Ctr, model::Unit};

use crate::{components::widgets::Tone, route::Route};

mod apps;
mod containers;
mod detail;

pub use apps::{AppDetail, Apps};
pub use containers::Containers;
pub use detail::Container;

/// Momentaufnahme laufend über den Live-Kanal; bis er steht, einmal per HTTP geladen.
pub fn use_snapshot<T: DeserializeOwned + PartialEq + Clone + 'static>(key: &'static str) -> Memo<Option<T>> {
    let live = crate::live::use_snap::<T>(key);
    let first = use_resource(move || async move { crate::api::snap::<T>(key).await.ok().map(|s| s.data) });
    use_memo(move || live.read().clone().or_else(|| first.read().clone().flatten()))
}

/// Zustand eines Containers als Text und Ton.
pub fn state_of(c: &Ctr) -> (String, Tone) {
    match c.state.as_str() {
        "running" if c.health == "unhealthy" => ("Ungesund".into(), Tone::Warn),
        "running" if c.health == "starting" => ("Startet".into(), Tone::Neutral),
        "running" => ("Läuft".into(), Tone::Good),
        "paused" => ("Pausiert".into(), Tone::Neutral),
        "restarting" => ("Startet neu".into(), Tone::Warn),
        "created" => ("Erstellt".into(), Tone::Neutral),
        "dead" => ("Defekt".into(), Tone::Bad),
        "removing" => ("Wird entfernt".into(), Tone::Neutral),
        _ if c.exit_code != 0 && !c.expected_exit => (format!("Fehler ({})", c.exit_code), Tone::Bad),
        _ => ("Beendet".into(), Tone::Neutral),
    }
}

/// Container mit Problem (ungesund, Fehlerende, Neustart).
pub fn has_problem(c: &Ctr) -> bool {
    matches!(state_of(c).1, Tone::Warn | Tone::Bad)
}

/// „seit 3 h“ für laufende, „vor 2 d“ für beendete Container.
pub fn uptime(c: &Ctr) -> String {
    let now = crate::fmt::now();
    if c.running() && c.started > 0 {
        shared::model::duration((now - c.started).max(0) as f64)
    } else if c.finished > 0 {
        crate::fmt::ago(c.finished)
    } else {
        "–".to_string()
    }
}

pub fn bytes(v: f64) -> String {
    Unit::Bytes.format(v)
}

pub fn bits(v: f64) -> String {
    Unit::BitsPerSec.format(v)
}

pub fn bps(v: f64) -> String {
    Unit::BytesPerSec.format(v)
}

pub fn pct(v: f64) -> String {
    Unit::Percent.format(v)
}

/// Spalten der Container-Tabelle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    App,
    State,
    Cpu,
    Mem,
    Rx,
    Tx,
    Io,
    Up,
    Restarts,
}

fn sort(list: &mut [Ctr], key: SortKey, desc: bool) {
    list.sort_by(|a, b| {
        let o = match key {
            SortKey::Name => a.name.cmp(&b.name),
            SortKey::App => a.app.cmp(&b.app).then_with(|| a.name.cmp(&b.name)),
            SortKey::State => (has_problem(b), b.running()).cmp(&(has_problem(a), a.running())).then_with(|| a.name.cmp(&b.name)),
            SortKey::Cpu => a.cpu.total_cmp(&b.cpu),
            SortKey::Mem => a.mem.total_cmp(&b.mem),
            SortKey::Rx => a.net_rx.total_cmp(&b.net_rx),
            SortKey::Tx => a.net_tx.total_cmp(&b.net_tx),
            SortKey::Io => (a.io_read + a.io_write).total_cmp(&(b.io_read + b.io_write)),
            SortKey::Up => {
                let up = |c: &Ctr| if c.running() { c.started } else { i64::MAX };
                up(b).cmp(&up(a))
            }
            SortKey::Restarts => a.restarts.cmp(&b.restarts),
        };
        if desc { o.reverse() } else { o }
    });
}

/// Kopfzelle zum Sortieren.
#[component]
fn SortHead(label: String, key_: SortKey, sort_key: Signal<SortKey>, desc: Signal<bool>, #[props(default)] class: String, #[props(default)] right: bool) -> Element {
    let active = sort_key() == key_;
    let align = if right { "justify-end text-right" } else { "" };
    let hl = if active { "text-foreground" } else { "" };
    rsx! {
        th { class: "h-9 px-2 text-xs font-medium whitespace-nowrap text-muted-foreground {class}",
            button {
                class: "inline-flex w-full items-center gap-1 hover:text-foreground {align} {hl}",
                onclick: move |_| {
                    if sort_key() == key_ {
                        desc.toggle();
                    } else {
                        sort_key.set(key_);
                        // Zahlen absteigend, Text aufsteigend.
                        desc.set(!matches!(key_, SortKey::Name | SortKey::App));
                    }
                },
                "{label}"
                if active {
                    if desc() {
                        icons::ChevronDown { class: "size-3" }
                    } else {
                        icons::ChevronUp { class: "size-3" }
                    }
                }
            }
        }
    }
}

/// Live-Tabelle von Containern, sortierbar; Klick öffnet die Detailseite.
#[component]
pub fn ContainerTable(list: Vec<Ctr>, #[props(default = true)] show_app: bool) -> Element {
    let sort_key = use_signal(|| SortKey::Cpu);
    let desc = use_signal(|| true);
    let mut rows = list;
    sort(&mut rows, sort_key(), desc());
    let nav = navigator();
    let app_cls = if show_app { "hidden lg:table-cell" } else { "hidden" };
    rsx! {
        div { class: "overflow-x-auto",
            table { class: "w-full text-sm",
                thead { class: "border-b",
                    tr {
                        SortHead { label: "Name", key_: SortKey::Name, sort_key, desc, class: "text-left" }
                        SortHead { label: "App", key_: SortKey::App, sort_key, desc, class: "text-left {app_cls}" }
                        SortHead { label: "Zustand", key_: SortKey::State, sort_key, desc, class: "text-left hidden sm:table-cell" }
                        SortHead { label: "CPU", key_: SortKey::Cpu, sort_key, desc, right: true }
                        SortHead { label: "Speicher", key_: SortKey::Mem, sort_key, desc, right: true }
                        SortHead { label: "Empfangen", key_: SortKey::Rx, sort_key, desc, right: true, class: "hidden md:table-cell" }
                        SortHead { label: "Gesendet", key_: SortKey::Tx, sort_key, desc, right: true, class: "hidden xl:table-cell" }
                        SortHead { label: "Datenträger", key_: SortKey::Io, sort_key, desc, right: true, class: "hidden xl:table-cell" }
                        SortHead { label: "Laufzeit", key_: SortKey::Up, sort_key, desc, right: true, class: "hidden md:table-cell" }
                        SortHead { label: "Neustarts", key_: SortKey::Restarts, sort_key, desc, right: true, class: "hidden 2xl:table-cell" }
                    }
                }
                tbody {
                    for c in rows {
                        {
                            let (label, tone) = state_of(&c);
                            let name = c.name.clone();
                            let running = c.running();
                            let dim = if running { "" } else { "text-muted-foreground" };
                            let mem_hint = c.mem_pct().map(|p| format!(" · {p:.0} %"));
                            rsx! {
                                tr {
                                    key: "{c.id}",
                                    class: "cursor-pointer border-b transition-colors last:border-0 hover:bg-muted/50 {dim}",
                                    onclick: move |_| {
                                        nav.push(Route::Container { name: name.clone() });
                                    },
                                    td { class: "max-w-0 px-2 py-2 sm:max-w-72",
                                        div { class: "flex items-center gap-2",
                                            span { class: "sm:hidden", crate::components::widgets::StatusDot { tone, label: "" } }
                                            div { class: "min-w-0",
                                                p { class: "truncate font-medium text-foreground", "{c.name}" }
                                                p { class: "truncate text-xs text-muted-foreground", "{c.image}" }
                                            }
                                        }
                                    }
                                    td { class: "px-2 py-2 {app_cls}",
                                        span { class: "truncate", "{c.app}" }
                                    }
                                    td { class: "hidden px-2 py-2 whitespace-nowrap sm:table-cell",
                                        crate::components::widgets::StatusDot { tone, label }
                                    }
                                    td { class: "px-2 py-2 text-right tabular-nums whitespace-nowrap",
                                        if running { "{pct(c.cpu)}" } else { "–" }
                                    }
                                    td { class: "px-2 py-2 text-right tabular-nums whitespace-nowrap",
                                        if running {
                                            "{bytes(c.mem)}"
                                            if let Some(h) = mem_hint {
                                                span { class: "hidden text-xs text-muted-foreground 2xl:inline", "{h}" }
                                            }
                                        } else {
                                            "–"
                                        }
                                    }
                                    td { class: "hidden px-2 py-2 text-right tabular-nums whitespace-nowrap md:table-cell",
                                        if c.host_net { span { class: "text-xs text-muted-foreground", title: "Teilt das Netz des Hosts", "Host-Netz" } } else if running { "{bits(c.net_rx)}" } else { "–" }
                                    }
                                    td { class: "hidden px-2 py-2 text-right tabular-nums whitespace-nowrap xl:table-cell",
                                        if c.host_net { "–" } else if running { "{bits(c.net_tx)}" } else { "–" }
                                    }
                                    td { class: "hidden px-2 py-2 text-right tabular-nums whitespace-nowrap xl:table-cell",
                                        if running { "{bps(c.io_read + c.io_write)}" } else { "–" }
                                    }
                                    td { class: "hidden px-2 py-2 text-right tabular-nums whitespace-nowrap md:table-cell", "{uptime(&c)}" }
                                    td { class: "hidden px-2 py-2 text-right tabular-nums 2xl:table-cell", "{c.restarts}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Kleiner Umschalter mit mehreren Möglichkeiten (Filter).
#[component]
pub fn Segmented(options: Vec<(String, String)>, value: Signal<String>) -> Element {
    rsx! {
        div { class: "inline-flex h-9 items-center rounded-md bg-muted p-0.5 text-sm",
            for (v, label) in options {
                {
                    let active = value() == v;
                    let cls = if active { "bg-background text-foreground shadow-sm" } else { "text-muted-foreground hover:text-foreground" };
                    rsx! {
                        button {
                            key: "{v}",
                            class: "h-8 rounded px-3 whitespace-nowrap transition-colors {cls}",
                            onclick: move |_| value.set(v.clone()),
                            "{label}"
                        }
                    }
                }
            }
        }
    }
}
