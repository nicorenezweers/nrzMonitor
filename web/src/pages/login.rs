//! Anmeldung (nur wenn der Server ein Passwort verlangt).

use dioxus::prelude::*;
use shared::api::LoginReq;

use crate::{api, components::ui::app_shell::AuthFrame};

#[component]
pub fn Login() -> Element {
    let mut password = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    rsx! {
        AuthFrame {
            logo: rsx! {
                svg { view_box: "0 0 64 64",
                    rect { width: "64", height: "64", rx: "14", fill: "var(--muted)" }
                    path { d: "M10 38h10l5-16 8 28 7-20 4 8h10", fill: "none", stroke: "var(--foreground)", stroke_width: "4.5", stroke_linecap: "round", stroke_linejoin: "round" }
                }
            },
            title: "nrzMonitor",
            tagline: "Bitte anmelden",
            form {
                class: "grid gap-4",
                onsubmit: move |e| {
                    e.prevent_default();
                    busy.set(true);
                    spawn(async move {
                        match api::post_empty("/login", &LoginReq { password: password() }).await {
                            Ok(()) => {
                                // Neu laden: Katalog und Live-Kanal starten mit der Sitzung.
                                if let Some(w) = web_sys::window() {
                                    let _ = w.location().reload();
                                }
                            }
                            Err(e) => {
                                error.set(Some(e));
                                busy.set(false);
                            }
                        }
                    });
                },
                label { class: "grid gap-1.5 text-sm",
                    "Passwort"
                    input {
                        r#type: "password",
                        autofocus: true,
                        autocomplete: "current-password",
                        class: "h-9 rounded-md border border-input bg-transparent px-3 text-sm outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                        value: "{password}",
                        oninput: move |e| password.set(e.value()),
                    }
                }
                if let Some(e) = error() {
                    p { class: "rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm", "{e}" }
                }
                button {
                    r#type: "submit",
                    disabled: busy(),
                    class: "h-9 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground outline-none hover:bg-primary/90 focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50",
                    "Anmelden"
                }
            }
        }
    }
}
