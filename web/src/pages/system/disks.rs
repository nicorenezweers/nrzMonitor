//! Datenträger: Dateisysteme mit Füllstand, Geräte mit Kennzahlen und Diagrammen je Gerät.

use dioxus::prelude::*;
use shared::{
    catalog::system::*,
    model::Unit,
    system::{DiskInfo, FsInfo, snap},
};

use super::common::{Charts, GroupTitle, KpiRow, dim, pct, use_dims};
use crate::{
    components::{
        chart::ChartCard,
        ui::{
            app_shell::{EmptyState, LoadingState, PageContainer, PageHeader, PageWidth},
            badge::{Badge, BadgeSize, BadgeVariant},
        },
        widgets::{Meter, StatTile, show},
    },
    live::use_snap,
    state::use_app,
};

/// Warnung und kritisch für Füllstände von Dateisystemen.
const WARN: f64 = 80.0;
const CRIT: f64 = 92.0;

#[component]
pub fn Disks() -> Element {
    let disks = use_snap::<Vec<DiskInfo>>(snap::DISKS);
    let fs = use_snap::<Vec<FsInfo>>(snap::FILESYSTEMS);
    let app = use_app();
    let io = use_dims(&SYSTEM_IO.chart_id(None));

    let (disks_loaded, fs_loaded) = (disks.read().is_some(), fs.read().is_some());
    let disks = disks().unwrap_or_default();
    let fs = fs().unwrap_or_default();
    let i = io.read();
    let (read, write) = (dim(&i, "read"), dim(&i, "write"));
    let fullest = fs.iter().max_by(|a, b| a.used_pct().total_cmp(&b.used_pct()));
    let used: u64 = fs.iter().map(|f| f.used).sum();
    let size: u64 = fs.iter().map(|f| f.used + f.avail).sum();
    let busiest = disks.iter().max_by(|a, b| a.util.total_cmp(&b.util));

    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title: "Datenträger", description: "Belegung, Durchsatz und Wartezeiten" }
            div { class: "grid gap-6",
                KpiRow { cols: 5,
                    StatTile { label: "Lesen", value: show(Unit::BytesPerSec, read), hint: "alle Geräte" }
                    StatTile { label: "Schreiben", value: show(Unit::BytesPerSec, write), hint: "alle Geräte" }
                    StatTile {
                        label: "Belegt",
                        value: Unit::Bytes.format(used as f64),
                        hint: format!("von {} in {} Dateisystemen", Unit::Bytes.format(size as f64), fs.len()),
                        Meter { value: pct(used as f64, size as f64), warn: WARN, crit: CRIT }
                    }
                    StatTile {
                        label: "Vollstes Dateisystem",
                        value: fullest.map_or_else(|| "–".to_string(), |f| Unit::Percent.format(f.used_pct())),
                        hint: fullest.map_or_else(String::new, |f| f.mount.clone()),
                    }
                    StatTile {
                        label: "Auslastung",
                        value: busiest.map_or_else(|| "–".to_string(), |d| Unit::Percent.format(d.util)),
                        hint: busiest.map_or_else(String::new, |d| format!("höchste: {}", d.name)),
                    }
                }
                if app.chart(&SYSTEM_IO.chart_id(None)).is_some() {
                    ChartCard { id: SYSTEM_IO.chart_id(None), height: 200 }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Dateisysteme", hint: format!("Warnung ab {WARN:.0} %, kritisch ab {CRIT:.0} %") }
                    if !fs_loaded {
                        LoadingState {}
                    } else if fs.is_empty() {
                        EmptyState { title: "Keine Dateisysteme", description: "Der Sammler hat noch keine eingehängten Dateisysteme gemeldet.", icon: rsx! { icons::HardDrive {} } }
                    } else {
                        div { class: "grid gap-4 md:grid-cols-2 2xl:grid-cols-3",
                            for f in fs.iter() {
                                FsCard { key: "{f.mount}", fs: f.clone() }
                            }
                        }
                    }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Geräte" }
                    if !disks_loaded {
                        LoadingState {}
                    } else if disks.is_empty() {
                        EmptyState { title: "Keine Geräte", description: "Der Sammler hat noch keine Datenträger gemeldet.", icon: rsx! { icons::HardDrive {} } }
                    }
                    for d in disks.iter() {
                        DeviceCard { key: "{d.name}", disk: d.clone() }
                    }
                }
            }
        }
    }
}

#[component]
fn FsCard(fs: FsInfo) -> Element {
    let p = fs.used_pct();
    let state = if p >= CRIT {
        Some(("Kritisch", BadgeVariant::Destructive))
    } else if p >= WARN {
        Some(("Fast voll", BadgeVariant::Warning))
    } else {
        None
    };
    let inodes = (fs.inodes > 0).then(|| pct(fs.inodes_used as f64, fs.inodes as f64));
    rsx! {
        div { class: "rounded-xl border bg-card p-4 shadow-sm",
            div { class: "flex items-start justify-between gap-3",
                div { class: "min-w-0",
                    p { class: "truncate font-mono text-sm font-medium", title: "{fs.mount}", "{fs.mount}" }
                    p { class: "truncate text-xs text-muted-foreground", title: "{fs.device}", "{fs.device} · {fs.fstype}" }
                }
                div { class: "flex shrink-0 items-center gap-1.5",
                    if fs.readonly {
                        Badge { variant: BadgeVariant::Outline, size: BadgeSize::Sm, "nur lesen" }
                    }
                    if let Some((t, v)) = state {
                        Badge { variant: v, size: BadgeSize::Sm, "{t}" }
                    }
                }
            }
            div { class: "mt-4 flex items-baseline justify-between gap-2",
                span { class: "text-2xl font-semibold tracking-tight tabular-nums", "{Unit::Percent.format(p)}" }
                span { class: "text-xs text-muted-foreground tabular-nums", "{Unit::Bytes.format(fs.avail as f64)} frei" }
            }
            Meter { value: p, warn: WARN, crit: CRIT, class: "mt-2 h-2" }
            div { class: "mt-3 grid grid-cols-3 gap-2 text-xs",
                Fact { label: "Belegt", value: Unit::Bytes.format(fs.used as f64) }
                Fact { label: "Frei", value: Unit::Bytes.format(fs.avail as f64) }
                Fact { label: "Gesamt", value: Unit::Bytes.format(fs.size as f64) }
            }
            if let Some(ip) = inodes {
                div { class: "mt-3 flex items-center gap-2 text-xs text-muted-foreground",
                    span { class: "shrink-0", "Inodes" }
                    Meter { value: ip, warn: WARN, crit: CRIT, class: "h-1" }
                    span { class: "shrink-0 tabular-nums", "{Unit::Percent.format(ip)}" }
                }
            }
        }
    }
}

#[component]
fn Fact(#[props(into)] label: String, #[props(into)] value: String) -> Element {
    rsx! {
        div { class: "min-w-0",
            p { class: "text-muted-foreground", "{label}" }
            p { class: "truncate font-medium tabular-nums", "{value}" }
        }
    }
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "nvme" => "NVMe",
        "ssd" => "SSD",
        "hdd" => "Festplatte",
        "virtual" => "Virtuell",
        "usb" => "USB",
        "mmc" => "SD/eMMC",
        k => k,
    }
}

#[component]
fn DeviceCard(disk: DiskInfo) -> Element {
    let n = Some(disk.name.as_str());
    let ids = vec![
        DISK_IO.chart_id(n),
        DISK_UTIL.chart_id(n),
        DISK_OPS.chart_id(n),
        DISK_AWAIT.chart_id(n),
        DISK_QUEUE.chart_id(n),
        DISK_TEMP.chart_id(n),
    ];
    rsx! {
        section { class: "min-w-0 rounded-xl border bg-card shadow-sm",
            div { class: "flex flex-wrap items-center justify-between gap-x-6 gap-y-3 border-b px-4 py-4 sm:px-6",
                div { class: "min-w-0 flex-1 basis-64",
                    div { class: "flex items-center gap-2",
                        h2 { class: "font-mono font-semibold", "{disk.name}" }
                        Badge { variant: BadgeVariant::Secondary, size: BadgeSize::Sm, "{kind_label(&disk.kind)}" }
                    }
                    p { class: "mt-1 truncate text-sm text-muted-foreground",
                        "{disk.model}"
                        if !disk.partitions.is_empty() {
                            " · {disk.partitions.join(\", \")}"
                        }
                    }
                }
                div { class: "grid w-full grid-cols-2 gap-x-6 gap-y-2 text-xs sm:w-auto sm:grid-cols-5",
                    Fact { label: "Grösse", value: Unit::Bytes.format(disk.size as f64) }
                    Fact { label: "Temperatur", value: show(Unit::Celsius, disk.temp) }
                    Fact { label: "Lesen", value: Unit::BytesPerSec.format(disk.read) }
                    Fact { label: "Schreiben", value: Unit::BytesPerSec.format(disk.write) }
                    div { class: "col-span-2 min-w-24 sm:col-span-1",
                        p { class: "text-muted-foreground", "Auslastung" }
                        p { class: "font-medium tabular-nums", "{Unit::Percent.format(disk.util)}" }
                        Meter { value: disk.util, warn: 70.0, crit: 90.0, class: "mt-1 h-1" }
                    }
                }
            }
            div { class: "p-3 sm:p-6",
                Charts { ids, height: 140 }
            }
        }
    }
}
