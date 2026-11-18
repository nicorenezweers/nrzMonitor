//! Ereignis-Marken für alle Diagramme: einmal je Zeitraum aus `GET /api/v1/events` geladen und mit
//! dem Live-Kanal ergänzt. [`MarksSource`] steht einmal im Rahmen; Karten filtern mit [`marks_for`].

use dioxus::prelude::*;
use shared::{
    events::{Event, EventPage, Severity},
    model::ChartMeta,
};

use super::plot::Mark;
use crate::{api, components::widgets::Tone, fmt, live::use_events, state::use_app};

/// Ereignisse im aktuellen Zeitraum, neueste zuerst.
pub static EVENTS: GlobalSignal<Vec<Event>> = Signal::global(Vec::new);

/// Lädt die Ereignisse des Zeitraums und hält sie live aktuell. Einmal im Rahmen einhängen.
#[component]
pub fn MarksSource() -> Element {
    let app = use_app();
    let live = use_events();
    let mut loaded = use_signal(Vec::<Event>::new);

    use_effect(move || {
        let range = (app.range)();
        spawn(async move {
            // Kurz warten: beim schnellen Umschalten nur die letzte Abfrage schicken.
            gloo_timers::future::TimeoutFuture::new(150).await;
            if range != *app.range.peek() {
                return;
            }
            let (a, b) = range.resolve(fmt::now());
            let path = api::qs("/events", &[("after", (a * 1000).to_string()), ("before", (b * 1000 + 999).to_string()), ("limit", "500".into())]);
            if let Ok(page) = api::get::<EventPage>(&path).await {
                loaded.set(page.events);
            }
        });
    });

    use_effect(move || {
        let range = (app.range)();
        let (a, b) = range.resolve(fmt::now());
        let mut all: Vec<Event> = Vec::new();
        for e in live.read().iter().chain(loaded.read().iter()) {
            let ts = e.ts / 1000;
            if ts >= a && (range.is_live() || ts <= b) && !all.iter().any(|x| (x.id != 0 && x.id == e.id) || (x.ts == e.ts && x.title == e.title)) {
                all.push(e.clone());
            }
        }
        all.sort_by_key(|e| std::cmp::Reverse(e.ts));
        if *EVENTS.peek() != all {
            *EVENTS.write() = all;
        }
    });
    rsx! {}
}

pub fn tone_of(sev: Severity) -> Tone {
    match sev {
        Severity::Info => Tone::Neutral,
        Severity::Ok => Tone::Good,
        Severity::Warn => Tone::Warn,
        Severity::Crit => Tone::Bad,
    }
}

/// Passt ein Ereignis zu einem Diagramm? Wichtige Ereignisse (Warnung, Fehler) zeigen alle
/// Diagramme ohne Instanz; sonst nur die Diagramme des betroffenen Objekts.
pub fn relevant(e: &Event, meta: &ChartMeta) -> bool {
    let own = !e.subject.is_empty() && meta.labels.values().any(|v| *v == e.subject)
        || e.labels.iter().any(|(k, v)| meta.label(k) == Some(v.as_str()));
    let alarm_chart = e.labels.get("chart").is_some_and(|c| *c == meta.id);
    own || alarm_chart || (meta.labels.is_empty() && e.severity >= Severity::Warn && meta.section == "system")
}

/// Marken für ein Diagramm (höchstens 40, die neuesten).
pub fn marks_for(meta: &ChartMeta) -> Vec<Mark> {
    EVENTS
        .read()
        .iter()
        .filter(|e| relevant(e, meta))
        .take(40)
        .map(|e| Mark {
            ts: e.ts / 1000,
            tone: tone_of(e.severity),
            label: if e.subject.is_empty() { e.title.clone() } else { format!("{} – {}", e.subject, e.title) },
        })
        .collect()
}
