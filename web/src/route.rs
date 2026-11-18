//! Alle Seiten der Oberfläche. Jeder Bereich ergänzt seine Routen in seinem Abschnitt.

use dioxus::prelude::*;

use crate::{
    components::layout::Shell,
    pages::{
        alerts::Alerts,
        cloudflare::Cloudflare,
        dns::{Dns, DnsQueries},
        docker::{AppDetail, Apps, Container, Containers},
        events::Events,
        explore::Explore,
        metrics::Metrics,
        netbird::Netbird,
        overview::Overview,
        power::Power,
        probes::Probes,
        settings::Settings,
        storage::Storage,
        system::{Cpu, Disks, Memory, Network, Processes, Sensors},
    },
};

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Shell)]
        // --- Übersicht, Metriken, Einstellungen ---------------------------------------------------
        #[route("/")]
        Overview {},
        #[route("/metrics")]
        Metrics {},
        #[route("/settings")]
        Settings {},


        // --- System -------------------------------------------------------------------------------
        #[route("/system/cpu")]
        Cpu {},
        #[route("/system/memory")]
        Memory {},
        #[route("/system/disks")]
        Disks {},
        #[route("/system/network")]
        Network {},
        #[route("/system/sensors")]
        Sensors {},
        #[route("/system/processes")]
        Processes {},
        #[route("/system/power")]
        Power {},


        // --- Container, Apps, Alarme --------------------------------------------------------------
        #[route("/containers")]
        Containers {},
        #[route("/containers/:name")]
        Container { name: String },
        #[route("/apps")]
        Apps {},
        #[route("/apps/:name")]
        AppDetail { name: String },
        #[route("/alerts")]
        Alerts {},


        // --- Netzwerk-Integrationen ---------------------------------------------------------------
        #[route("/dns")]
        Dns {},
        #[route("/dns/queries")]
        DnsQueries {},
        #[route("/cloudflare")]
        Cloudflare {},
        #[route("/netbird")]
        Netbird {},
        #[route("/probes")]
        Probes {},


        // --- Speicher & Analyse -------------------------------------------------------------------
        #[route("/explore")]
        Explore {},
        #[route("/events")]
        Events {},
        #[route("/storage")]
        Storage {},


    #[end_layout]
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

#[component]
fn NotFound(segments: Vec<String>) -> Element {
    let _ = segments;
    navigator().replace(Route::Overview {});
    rsx! {}
}
