//! Einstellungen: allgemeine Angaben, Vorlieben der Oberfläche und die Abschnitte der Bereiche.

use dioxus::prelude::*;

use super::{alerts::AlertSettings, integrations::IntegrationSettings, storage::StorageSettings};
use crate::{
    components::{
        layout::{default_span, set_default_span},
        ui::{
            app_shell::{PageContainer, PageHeader, PageWidth, SettingsGroup, SettingsRow},
            kbd::Kbd,
        },
    },
    fmt,
    live::use_connected,
    state::{PRESETS, use_app},
};

#[component]
pub fn Settings() -> Element {
    let app = use_app();
    let connected = use_connected();
    let info = app.info.read();
    let mut span = use_signal(default_span);
    let started = if info.started_at > 0 { format!("{} ({})", fmt::full(info.started_at), fmt::ago(info.started_at)) } else { "–".to_string() };
    let live_window = shared::model::duration(f64::from(info.live_window));
    rsx! {
        PageContainer { width: PageWidth::Narrow,
            PageHeader { title: "Einstellungen", description: "Allgemeines, Integrationen, Speicher und Benachrichtigungen" }
            div { class: "grid gap-6",
                SettingsGroup { title: "Allgemein",
                    SettingsRow { label: "Host", span { class: "text-sm text-muted-foreground", "{info.hostname}" } }
                    SettingsRow { label: "Version", span { class: "text-sm text-muted-foreground tabular-nums", "nrzMonitor {info.version}" } }
                    SettingsRow { label: "Betrieb", hint: "Beispieldaten auf Systemen ohne /proc oder mit NRZMON_DEMO=1",
                        span { class: "text-sm text-muted-foreground",
                            if info.demo { "Beispieldaten" } else { "Echte Messwerte" }
                        }
                    }
                    SettingsRow { label: "Läuft seit", span { class: "text-sm text-muted-foreground tabular-nums", "{started}" } }
                    SettingsRow { label: "Messtakt", hint: "Abstand zwischen zwei Messungen der lokalen Sammler (NRZMON_INTERVAL)",
                        span { class: "text-sm text-muted-foreground tabular-nums", "{info.interval} s" }
                    }
                    SettingsRow { label: "Live-Fenster", hint: "So viel liegt in voller Auflösung im Arbeitsspeicher (NRZMON_LIVE_WINDOW)",
                        span { class: "text-sm text-muted-foreground tabular-nums", "{live_window}" }
                    }
                    SettingsRow { label: "Anmeldung", hint: "Passwortschutz über NRZMON_PASSWORD",
                        span { class: "text-sm text-muted-foreground",
                            if info.auth { "verlangt" } else { "aus" }
                        }
                    }
                    SettingsRow { label: "Live-Verbindung",
                        span { class: "inline-flex items-center gap-1.5 text-sm text-muted-foreground",
                            span { class: if connected() { "size-2 rounded-full bg-success" } else { "size-2 animate-pulse rounded-full bg-warning" } }
                            if connected() { "verbunden" } else { "verbinde …" }
                        }
                    }
                }
                SettingsGroup { title: "Oberfläche", footer: "Wird in diesem Browser gespeichert.",
                    SettingsRow { label: "Standard-Zeitraum", hint: "Zeitraum beim Öffnen von nrzMonitor",
                        select {
                            class: "h-9 rounded-md border bg-background px-2 text-sm outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                            "aria-label": "Standard-Zeitraum",
                            value: "{span}",
                            onchange: move |e| {
                                if let Ok(v) = e.value().parse::<i64>() {
                                    span.set(v);
                                    set_default_span(v);
                                }
                            },
                            for (label, secs) in PRESETS.iter().copied() {
                                option { key: "{secs}", value: "{secs}", selected: span() == secs, "letzte {label}" }
                            }
                        }
                    }
                    SettingsRow { label: "Suchen", hint: "Seiten, Container und Diagramme finden",
                        span { class: "inline-flex items-center gap-1", Kbd { "⌘K" } span { class: "text-xs text-muted-foreground", "oder" } Kbd { "/" } }
                    }
                    SettingsRow { label: "Diagramme", hint: "Ziehen zoomt, Doppelklick geht zurück, Umschalt + Mausrad zoomt, Pfeiltasten bewegen das Fadenkreuz",
                        span { class: "text-sm text-muted-foreground", "Maus und Tastatur" }
                    }
                }
                IntegrationSettings {}
                StorageSettings {}
                AlertSettings {}
            }
        }
    }
}
