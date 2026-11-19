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

/// Stylesheet, beim Bauen von `build.rs` aus den Klassen im Quelltext erzeugt (siehe `style/`).
const CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/nrz.css"));
const FAVICON: Asset = asset!("/assets/favicon.svg");

fn main() {
    install_css();
    dioxus::launch(App);
}

/// Stylesheet in den `<head>` legen, bevor die Oberfläche das erste Mal zeichnet.
fn install_css() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    let (Ok(style), Ok(Some(head))) = (doc.create_element("style"), doc.query_selector("head")) else { return };
    style.set_text_content(Some(CSS));
    let _ = head.append_child(&style);
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
        Router::<route::Route> {}
    }
}
