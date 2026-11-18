//! Seite „Container“: Kennzahlen, Summen-Diagramme, Live-Tabelle aller Container, Docker-Bestand.

use dioxus::prelude::*;
use shared::{
    catalog::docker::*,
    docker::{Container as Ctr, DockerInfo, Image, Network, Volume},
};

use super::{use_snapshot, ContainerTable, Segmented, bits, bps, bytes, has_problem, pct};
use crate::{
    components::{
        chart::{ChartCard, section::ChartGrid},
        ui::{
            app_shell::{EmptyState, LoadingState, PageContainer, PageHeader, PageSection, PageWidth},
            input::Input,
        },
        widgets::StatTile,
    },
};

#[component]
pub fn Containers() -> Element {
    let list = use_snapshot::<Vec<Ctr>>("docker.containers");
    let info = use_snapshot::<DockerInfo>("docker.info");
    let mut query = use_signal(String::new);
    let state = use_signal(|| "all".to_string());
    let mut app = use_signal(String::new);

    let info_r = info.read();
    let description = match info_r.as_ref() {
        Some(i) if !i.available => format!("Docker nicht erreichbar: {}", i.error.clone().unwrap_or_default()),
        Some(i) if !i.version.is_empty() => format!("Docker {} · cgroup v{} ({}) · Messtakt {} ms", i.version, i.cgroup_version, i.cgroup_driver, shared::model::trim(i.collect_ms, 1)),
        _ => "Alle Docker-Container des Hosts".to_string(),
    };
    drop(info_r);

    let Some(all) = list.read().clone() else {
        return rsx! {
            PageContainer { width: PageWidth::Full,
                PageHeader { title: "Container", description }
                LoadingState {}
            }
        };
    };
    let running: Vec<&Ctr> = all.iter().filter(|c| c.running()).collect();
    let problems = all.iter().filter(|c| has_problem(c)).count();
    let cpu: f64 = running.iter().map(|c| c.cpu).sum();
    let mem: f64 = running.iter().map(|c| c.mem).sum();
    let rx: f64 = running.iter().map(|c| c.net_rx).sum();
    let tx: f64 = running.iter().map(|c| c.net_tx).sum();
    let rd: f64 = running.iter().map(|c| c.io_read).sum();
    let wr: f64 = running.iter().map(|c| c.io_write).sum();
    let mut apps: Vec<String> = all.iter().map(|c| c.app.clone()).collect();
    apps.sort();
    apps.dedup();
    let napps = apps.len();
    let mem_total = info.read().as_ref().map_or(0, |i| i.mem_total);
    let mem_hint = if mem_total > 0 { format!("{} des Hosts", pct((mem / mem_total as f64 * 1000.0).round() / 10.0)) } else { String::new() };
    let ncpu = info.read().as_ref().map_or(0, |i| i.ncpu);
    let cpu_hint = if ncpu > 0 { format!("von {} % ({ncpu} Kerne)", ncpu * 100) } else { "100 % = ein Kern".to_string() };

    let q = query().to_lowercase();
    let st = state();
    let ap = app();
    let filtered: Vec<Ctr> = all
        .iter()
        .filter(|c| match st.as_str() {
            "running" => c.running(),
            "stopped" => !c.running(),
            "problem" => has_problem(c),
            _ => true,
        })
        .filter(|c| ap.is_empty() || c.app == ap)
        .filter(|c| q.is_empty() || c.name.to_lowercase().contains(&q) || c.image.to_lowercase().contains(&q) || c.app.to_lowercase().contains(&q))
        .cloned()
        .collect();
    let shown = filtered.len();
    let total = all.len();

    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "Container", description }
            div { class: "grid grid-cols-[minmax(0,1fr)] gap-6",
                div { class: "grid grid-cols-2 gap-4 md:grid-cols-3 2xl:grid-cols-6",
                    StatTile { label: "Laufend", value: "{running.len()} / {total}", hint: "{napps} Apps", icon: rsx! { icons::Container {} } }
                    StatTile {
                        label: "Probleme",
                        value: "{problems}",
                        hint: if problems == 0 { "alles in Ordnung" } else { "ungesund, Fehler, Neustarts" },
                        icon: rsx! { icons::TriangleAlert {} },
                        class: if problems > 0 { "border-warning/60" } else { "" },
                    }
                    StatTile { label: "CPU gesamt", value: pct((cpu * 10.0).round() / 10.0), hint: cpu_hint, icon: rsx! { icons::Cpu {} } }
                    StatTile { label: "Speicher gesamt", value: bytes(mem), hint: mem_hint, icon: rsx! { icons::MemoryStick {} } }
                    StatTile { label: "Netzwerk ↓", value: bits(rx), hint: "↑ {bits(tx)} gesendet", icon: rsx! { icons::ArrowDownUp {} } }
                    StatTile { label: "Datenträger", value: bps(rd + wr), hint: "{bps(rd)} lesen · {bps(wr)} schreiben", icon: rsx! { icons::HardDrive {} } }
                }
                ChartGrid {
                    ChartCard { id: DOCKER_CPU_ALL.context, height: 200 }
                    ChartCard { id: DOCKER_MEM_ALL.context, height: 200 }
                    ChartCard { id: DOCKER_NET_RX_ALL.context, height: 200 }
                    ChartCard { id: DOCKER_NET_TX_ALL.context, height: 200 }
                    ChartCard { id: DOCKER_CONTAINERS.context, height: 200 }
                    ChartCard { id: DOCKER_IMAGES.context, height: 200 }
                }
                PageSection {
                    title: "Alle Container",
                    description: if shown == total { format!("{total} Container – Werte jede Sekunde") } else { format!("{shown} von {total} Container") },
                    div { class: "mb-4 flex flex-wrap items-center gap-2",
                        div { class: "w-full sm:w-72",
                            Input {
                                placeholder: "Suchen: Name, Image, App",
                                value: query(),
                                oninput: move |e: FormEvent| query.set(e.value()),
                            }
                        }
                        Segmented {
                            options: vec![
                                ("all".into(), "Alle".into()),
                                ("running".into(), "Laufend".into()),
                                ("stopped".into(), "Gestoppt".into()),
                                ("problem".into(), format!("Probleme{}", if problems > 0 { format!(" ({problems})") } else { String::new() })),
                            ],
                            value: state,
                        }
                        select {
                            class: "h-9 max-w-full rounded-md border border-input bg-background px-2 text-sm",
                            "aria-label": "App",
                            value: "{ap}",
                            onchange: move |e| app.set(e.value()),
                            option { value: "", "Alle Apps" }
                            for a in apps {
                                option { key: "{a}", value: "{a}", "{a}" }
                            }
                        }
                    }
                    if filtered.is_empty() {
                        EmptyState { title: "Keine Container", description: "Kein Container passt zum Filter.", icon: rsx! { icons::SearchX {} } }
                    } else {
                        ContainerTable { list: filtered }
                    }
                }
                DockerInventory {}
            }
        }
    }
}

/// Images, Volumes und Netzwerke.
#[component]
fn DockerInventory() -> Element {
    let images = use_snapshot::<Vec<Image>>("docker.images");
    let volumes = use_snapshot::<Vec<Volume>>("docker.volumes");
    let networks = use_snapshot::<Vec<Network>>("docker.networks");
    let tab = use_signal(|| "images".to_string());
    let imgs = images.read().clone().unwrap_or_default();
    let vols = volumes.read().clone().unwrap_or_default();
    let nets = networks.read().clone().unwrap_or_default();
    let img_size: u64 = imgs.iter().map(|i| i.size).sum();
    let unused = imgs.iter().filter(|i| i.containers == 0).count();
    let th = "h-9 px-2 text-left text-xs font-medium whitespace-nowrap text-muted-foreground";
    let thr = "h-9 px-2 text-right text-xs font-medium whitespace-nowrap text-muted-foreground";
    let td = "px-2 py-1.5";
    let tdr = "px-2 py-1.5 text-right tabular-nums whitespace-nowrap";
    rsx! {
        PageSection {
            title: "Docker-Bestand",
            description: "{imgs.len()} Images ({bytes(img_size as f64)}, {unused} unbenutzt) · {vols.len()} Volumes · {nets.len()} Netzwerke",
            actions: rsx! {
                Segmented {
                    options: vec![("images".into(), "Images".into()), ("volumes".into(), "Volumes".into()), ("networks".into(), "Netzwerke".into())],
                    value: tab,
                }
            },
            div { class: "max-h-[28rem] overflow-auto",
                match tab().as_str() {
                    "volumes" => rsx! {
                        table { class: "w-full text-sm",
                            thead { class: "sticky top-0 border-b bg-card",
                                tr {
                                    th { class: th, "Name" }
                                    th { class: "{th} hidden md:table-cell", "App" }
                                    th { class: thr, "Grösse" }
                                    th { class: thr, "Container" }
                                }
                            }
                            tbody {
                                for v in vols {
                                    tr { key: "{v.name}", class: "border-b last:border-0",
                                        td { class: "{td} max-w-0 sm:max-w-md", p { class: "truncate font-mono text-xs", title: "{v.mountpoint}", "{v.name}" } }
                                        td { class: "{td} hidden md:table-cell text-muted-foreground", "{v.app}" }
                                        td { class: tdr, if v.size >= 0 { "{bytes(v.size as f64)}" } else { "–" } }
                                        td { class: tdr,
                                            if v.containers == 0 { span { class: "text-xs text-muted-foreground", "unbenutzt" } } else { "{v.containers}" }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    "networks" => rsx! {
                        table { class: "w-full text-sm",
                            thead { class: "sticky top-0 border-b bg-card",
                                tr {
                                    th { class: th, "Name" }
                                    th { class: "{th} hidden sm:table-cell", "Treiber" }
                                    th { class: "{th} hidden md:table-cell", "Subnetz" }
                                    th { class: thr, "Container" }
                                }
                            }
                            tbody {
                                for n in nets {
                                    tr { key: "{n.id}", class: "border-b last:border-0",
                                        td { class: "{td} max-w-0 sm:max-w-md", p { class: "truncate", "{n.name}" } }
                                        td { class: "{td} hidden sm:table-cell text-muted-foreground", "{n.driver}" }
                                        td { class: "{td} hidden md:table-cell font-mono text-xs text-muted-foreground", "{n.subnet}" }
                                        td { class: tdr, "{n.containers}" }
                                    }
                                }
                            }
                        }
                    },
                    _ => rsx! {
                        table { class: "w-full text-sm",
                            thead { class: "sticky top-0 border-b bg-card",
                                tr {
                                    th { class: th, "Image" }
                                    th { class: "{th} hidden md:table-cell", "ID" }
                                    th { class: thr, "Grösse" }
                                    th { class: "{thr} hidden sm:table-cell", "Erstellt" }
                                    th { class: thr, "Container" }
                                }
                            }
                            tbody {
                                for i in imgs {
                                    tr { key: "{i.id}", class: "border-b last:border-0",
                                        td { class: "{td} max-w-0 sm:max-w-md",
                                            p { class: "truncate", "{i.tags.first().cloned().unwrap_or_default()}" }
                                        }
                                        td { class: "{td} hidden md:table-cell font-mono text-xs text-muted-foreground", "{i.id}" }
                                        td { class: tdr, "{bytes(i.size as f64)}" }
                                        td { class: "{tdr} hidden sm:table-cell text-muted-foreground", "{crate::fmt::ago(i.created)}" }
                                        td { class: tdr,
                                            if i.containers == 0 { span { class: "text-xs text-muted-foreground", "unbenutzt" } } else { "{i.containers}" }
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
