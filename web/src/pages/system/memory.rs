//! Arbeitsspeicher: Kennzahlen, grosser Füllstand und Verläufe.

use dioxus::prelude::*;
use shared::{catalog::system::*, model::Unit, system::{HostInfo, snap}};

use super::common::{Charts, GroupTitle, KpiRow, Segment, StackBar, dim, pct, sum, use_dims};
use crate::{
    components::{
        ui::app_shell::{PageContainer, PageHeader, PageSection, PageWidth},
        widgets::{Meter, StatTile, show},
    },
    live::use_snap,
};

#[component]
pub fn Memory() -> Element {
    let info = use_snap::<HostInfo>(snap::INFO);
    let ram = use_dims(&SYSTEM_RAM.chart_id(None));
    let avail = use_dims(&MEM_AVAILABLE.chart_id(None));
    let swap = use_dims(&SYSTEM_SWAP.chart_id(None));
    let committed = use_dims(&MEM_COMMITTED.chart_id(None));

    let r = ram.read();
    let total = info().map(|i| i.mem_total as f64).filter(|t| *t > 0.0).or_else(|| sum(&r));
    let used = dim(&r, "used");
    let buffers = dim(&r, "buffers");
    let cached = dim(&r, "cached");
    let free = dim(&r, "free");
    let available = dim(&avail.read(), "available");
    let s = swap.read();
    let swap_used = dim(&s, "used");
    let swap_total = sum(&s).filter(|t| *t > 0.0);
    let c = committed.read();
    let (commit, limit) = (dim(&c, "committed"), dim(&c, "limit"));
    let used_pct = used.zip(total).map(|(u, t)| pct(u, t));

    let segments: Vec<Segment> = r
        .iter()
        .enumerate()
        .map(|(slot, (id, name, v))| Segment {
            name: name.clone(),
            value: v.unwrap_or(0.0),
            // „Frei“ bleibt neutral; die anderen tragen die Farbe ihrer Reihe im Diagramm.
            slot: (id != "free").then_some(slot),
            text: show(Unit::Bytes, *v),
        })
        .collect();

    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader {
                title: "Arbeitsspeicher",
                description: total.map_or_else(|| "Belegung, Swap und Kernel-Speicher".to_string(), |t| format!("{} eingebaut · Belegung, Swap und Kernel-Speicher", Unit::Bytes.format(t))),
            }
            div { class: "grid gap-6",
                KpiRow { cols: 5,
                    StatTile { label: "Belegt", value: show(Unit::Bytes, used), hint: used_pct.map_or_else(String::new, |p| format!("{} von {}", Unit::Percent.format(p), show(Unit::Bytes, total))),
                        Meter { value: used_pct.unwrap_or(0.0) }
                    }
                    StatTile { label: "Verfügbar", value: show(Unit::Bytes, available), hint: available.zip(total).map_or_else(String::new, |(a, t)| format!("{} frei nutzbar", Unit::Percent.format(pct(a, t)))) }
                    StatTile { label: "Cache und Puffer", value: show(Unit::Bytes, cached.map(|c| c + buffers.unwrap_or(0.0))), hint: "gibt der Kernel bei Bedarf frei" }
                    StatTile {
                        label: "Swap",
                        value: show(Unit::Bytes, swap_used),
                        hint: swap_total.map_or_else(|| "kein Swap eingerichtet".to_string(), |t| format!("von {}", Unit::Bytes.format(t))),
                        if let (Some(u), Some(t)) = (swap_used, swap_total) {
                            Meter { value: pct(u, t), warn: 50.0, crit: 85.0 }
                        }
                    }
                    StatTile {
                        label: "Zugesagt",
                        value: show(Unit::Bytes, commit),
                        hint: limit.map_or_else(String::new, |l| format!("Grenze {}", Unit::Bytes.format(l))),
                        if let (Some(c), Some(l)) = (commit, limit) {
                            // Über der Grenze ist nur bei strengem Overcommit ein Problem: neutral zeigen.
                            Meter { value: pct(c, l), warn: 101.0, crit: 101.0 }
                        }
                    }
                }
                PageSection {
                    title: "Aufteilung",
                    description: format!("Frei im engeren Sinn: {} – der Cache zählt zum verfügbaren Speicher.", show(Unit::Bytes, free)),
                    StackBar { segments, height: 22 }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Belegung" }
                    Charts { ids: vec![SYSTEM_RAM.chart_id(None), MEM_AVAILABLE.chart_id(None), MEM_COMMITTED.chart_id(None)], height: 200 }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Swap, Kernel und Rückschreiben" }
                    Charts { ids: vec![SYSTEM_SWAP.chart_id(None), MEM_KERNEL.chart_id(None), MEM_WRITEBACK.chart_id(None)] }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Aktivität" }
                    Charts { ids: vec![MEM_PAGING.chart_id(None), MEM_SWAPIO.chart_id(None), MEM_FAULTS.chart_id(None)], height: 160 }
                }
                div { class: "grid gap-3",
                    GroupTitle { title: "Seltenes" }
                    Charts { ids: vec![MEM_OOM.chart_id(None), MEM_HUGEPAGES.chart_id(None)], height: 160, pairs: true }
                }
            }
        }
    }
}
