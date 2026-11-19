//! Seiten. Je Bereich eine Datei oder ein Ordner – wem was gehört, steht in `ARCHITECTURE.md`.

pub mod alerts;
pub mod cloudflare;
pub mod dns;
pub mod docker;
pub mod events;
pub mod explore;
pub mod integrations;
pub mod login;
pub mod mcp;
pub mod metrics;
pub mod netbird;
pub mod overview;
pub mod power;
pub mod probes;
pub mod settings;
pub mod storage;
pub mod system;

use dioxus::prelude::*;

use crate::components::{
    chart::SectionCharts,
    ui::app_shell::{PageContainer, PageHeader, PageWidth},
};

/// Einfache Seite: Kopf und alle Diagramme eines Bereichs. Ausgangspunkt für die Bereichsseiten.
#[component]
pub fn SectionPage(#[props(into)] title: String, #[props(into)] description: String, #[props(into)] section: String) -> Element {
    rsx! {
        PageContainer { width: PageWidth::Full,
            PageHeader { title, description }
            SectionCharts { section }
        }
    }
}
