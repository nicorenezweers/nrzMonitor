//! nrzMonitor – Oberfläche (Dioxus, WASM) mit nrzUI-Bausteinen.
#![allow(clippy::useless_format)] // rsx! erzeugt format!-Aufrufe für Attribute
#![allow(non_snake_case)]
// Das Fundament stellt Bausteine für alle Seiten bereit; nicht jeder wird schon überall gebraucht.
#![allow(dead_code, unused_imports)]

mod api;
mod components;
mod fmt;
mod live;
mod pages;
mod route;
mod state;

use dioxus::prelude::*;

const TAILWIND: Asset = asset!("/assets/tailwind.css");
const FAVICON: Asset = asset!("/assets/favicon.svg");

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    state::provide();
    rsx! {
        document::Meta { name: "viewport", content: "width=device-width, initial-scale=1, viewport-fit=cover" }
        document::Meta { name: "theme-color", content: "#1f1f1f" }
        document::Meta { name: "apple-mobile-web-app-capable", content: "yes" }
        document::Title { "nrzMonitor" }
        document::Link { rel: "icon", href: FAVICON }
        document::Stylesheet { href: TAILWIND }
        Router::<route::Route> {}
    }
}
