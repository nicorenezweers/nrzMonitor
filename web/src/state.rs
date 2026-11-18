//! Gemeinsamer Zustand der Oberfläche: Server-Info, Katalog der Diagramme, gewählter Zeitraum und
//! die Zeit unter dem Mauszeiger (alle Diagramme zeigen dort ihr Fadenkreuz).

use std::rc::Rc;

use dioxus::prelude::*;
use shared::{api::Info, model::ChartMeta};

use crate::{api, live::Live};

/// Zeitraum aller Diagramme. `before == 0` heisst „bis jetzt, läuft mit“.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    /// Beginn: Unix-Sekunden oder (≤ 0) Sekunden vor dem Ende.
    pub after: i64,
    /// Ende: Unix-Sekunden oder 0 = jetzt.
    pub before: i64,
}

impl TimeRange {
    /// Die letzten `secs` Sekunden, läuft mit.
    pub const fn live(secs: i64) -> Self {
        Self { after: -secs, before: 0 }
    }

    pub const fn fixed(after: i64, before: i64) -> Self {
        Self { after, before }
    }

    pub const fn is_live(&self) -> bool {
        self.before <= 0
    }

    /// Länge in Sekunden.
    pub fn span(&self) -> i64 {
        if self.after <= 0 { -self.after } else { (self.before - self.after).max(1) }
    }

    /// Als absolute Zeiten (Unix-Sekunden).
    pub fn resolve(&self, now: i64) -> (i64, i64) {
        let before = if self.before <= 0 { now + self.before } else { self.before };
        let after = if self.after <= 0 { before + self.after } else { self.after };
        (after, before)
    }
}

/// Auswahl im Zeitraum-Schalter: (Beschriftung, Sekunden).
pub const PRESETS: &[(&str, i64)] = &[
    ("5 min", 300),
    ("15 min", 900),
    ("1 h", 3600),
    ("6 h", 21_600),
    ("24 h", 86_400),
    ("7 d", 604_800),
    ("30 d", 2_592_000),
];

#[derive(Clone, Copy)]
pub struct App {
    pub info: Signal<Info>,
    /// Alle Diagramme, sortiert wie in der Anzeige.
    pub charts: Signal<Rc<Vec<ChartMeta>>>,
    pub range: Signal<TimeRange>,
    /// Vorheriger Zeitraum (für „Zurück“ nach dem Hineinzoomen).
    pub range_back: Signal<Option<TimeRange>>,
    /// Zeit unter dem Mauszeiger (Unix-Sekunden).
    pub hover: Signal<Option<i64>>,
}

impl App {
    /// Diagramm nach ID.
    pub fn chart(&self, id: &str) -> Option<ChartMeta> {
        self.charts.read().iter().find(|c| c.id == id).cloned()
    }

    /// Alle Diagramme eines Kontexts (z. B. `disk.io` → je Gerät eines).
    pub fn by_context(&self, context: &str) -> Vec<ChartMeta> {
        self.charts.read().iter().filter(|c| c.context == context).cloned().collect()
    }

    /// Alle Diagramme eines Bereichs.
    pub fn by_section(&self, section: &str) -> Vec<ChartMeta> {
        self.charts.read().iter().filter(|c| c.section == section).cloned().collect()
    }

    /// Diagramme eines Kontexts mit einem bestimmten Label (z. B. `docker.cpu`, `container` = Name).
    pub fn by_label(&self, context: &str, key: &str, value: &str) -> Option<ChartMeta> {
        self.charts.read().iter().find(|c| c.context == context && c.label(key) == Some(value)).cloned()
    }

    /// In einen festen Zeitraum hineinzoomen; der bisherige bleibt für „Zurück“ gemerkt.
    pub fn zoom(mut self, after: i64, before: i64) {
        if before - after < 2 {
            return;
        }
        let current = *self.range.peek();
        if self.range_back.peek().is_none() || current.is_live() {
            self.range_back.set(Some(current));
        }
        self.range.set(TimeRange::fixed(after, before));
    }

    /// Zurück zum Zeitraum vor dem Zoomen (oder zu den letzten 15 Minuten).
    pub fn zoom_out(mut self) {
        let back = self.range_back.take().unwrap_or(TimeRange::live(900));
        self.range.set(back);
    }

    pub fn set_range(mut self, range: TimeRange) {
        self.range_back.set(None);
        self.range.set(range);
    }

    /// Katalog neu vom Server laden.
    pub async fn reload_charts(mut self) {
        if let Ok(list) = api::get::<Vec<ChartMeta>>("/charts").await {
            self.charts.set(Rc::new(list));
        }
    }
}

/// Einmal ganz oben (in `App`) aufrufen.
pub fn provide() {
    let app = use_context_provider(|| App {
        info: Signal::new(Info::default()),
        charts: Signal::new(Rc::new(Vec::new())),
        range: Signal::new(TimeRange::live(900)),
        range_back: Signal::new(None),
        hover: Signal::new(None),
    });
    let live = use_context_provider(Live::new);

    use_future(move || {
        let live = live.clone();
        async move {
            let mut app = app;
            if let Ok(info) = api::get::<Info>("/info").await {
                if info.auth && !info.authenticated {
                    *api::UNAUTHORIZED.write() = true;
                }
                app.info.set(info);
            }
            app.reload_charts().await;
            live.run(app).await;
        }
    });
}

pub fn use_app() -> App {
    use_context::<App>()
}
