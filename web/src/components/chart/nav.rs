//! Zeitraum bedienen: Zoomen mit Verlauf („Zurück“ mehrfach), Verschieben, Anhalten.
//! Alle Diagramme und die Zeitraum-Leiste nutzen diese Funktionen.

use dioxus::prelude::*;

use super::geom;
use crate::{
    fmt,
    state::{App, TimeRange},
};

/// Frühere Zeiträume (neuester zuletzt) – für „Zurück“ nach mehrfachem Zoomen oder Verschieben.
pub static HISTORY: GlobalSignal<Vec<TimeRange>> = Signal::global(Vec::new);
/// Länge des Live-Fensters vor dem Anhalten (für „Weiter“).
pub static PAUSED_SPAN: GlobalSignal<Option<i64>> = Signal::global(|| None);

const KEEP: usize = 30;

fn remember(app: App) {
    let current = *app.range.peek();
    let mut h = HISTORY.write();
    if h.last() != Some(&current) {
        h.push(current);
    }
    let excess = h.len().saturating_sub(KEEP);
    h.drain(..excess);
}

/// Aktuelles Fenster als absolute Zeiten.
pub fn window(app: App) -> (i64, i64) {
    app.range.peek().resolve(fmt::now())
}

/// In ein festes Fenster zoomen (merkt den bisherigen Zeitraum).
pub fn zoom(app: App, after: i64, before: i64) {
    if before - after < 2 {
        return;
    }
    remember(app);
    let now = fmt::now();
    app.zoom(after.min(now - 2), before.min(now));
}

/// Um `factor` hinein- (< 1) oder herauszoomen, Mitte bei `frac` (0..1).
pub fn zoom_by(app: App, frac: f64, factor: f64) {
    let (a, b) = window(app);
    let (a, b) = geom::zoom_window(a, b, frac, factor, 10);
    let now = fmt::now();
    if b >= now - 1 && factor > 1.0 && app.range.peek().is_live() {
        // Herauszoomen im Live-Betrieb bleibt live.
        remember(app);
        app.set_range(TimeRange::live((b - a).min(2_592_000)));
        return;
    }
    zoom(app, a, b);
}

/// Um einen Bruchteil der Länge verschieben (negativ = zurück in die Vergangenheit).
/// Erreicht das Fenster die Gegenwart, läuft es wieder live mit.
pub fn pan(app: App, frac: f64) {
    let (a, b) = window(app);
    let now = fmt::now();
    let (na, nb) = geom::pan_window(a, b, frac, now);
    if app.range.peek().is_live() {
        remember(app);
    }
    if nb >= now - 1 {
        app.set_range(TimeRange::live(nb - na));
    } else {
        app.zoom(na, nb);
    }
}

/// Einen Schritt zurück (oder zum Live-Zeitraum vor dem Zoomen).
pub fn back(app: App) {
    let prev = HISTORY.write().pop();
    match prev {
        Some(r) => app.set_range(r),
        None => app.zoom_out(),
    }
}

/// Gibt es etwas, wohin „Zurück“ führt?
pub fn can_back(app: App) -> bool {
    !HISTORY.read().is_empty() || !(app.range)().is_live()
}

/// Zeitraum neu wählen (Voreinstellung): Verlauf leeren.
pub fn set(app: App, range: TimeRange) {
    HISTORY.write().clear();
    *PAUSED_SPAN.write() = None;
    app.set_range(range);
}

/// Live anhalten: das aktuelle Fenster einfrieren.
pub fn pause(app: App) {
    let r = *app.range.peek();
    if !r.is_live() {
        return;
    }
    let (a, b) = r.resolve(fmt::now());
    *PAUSED_SPAN.write() = Some(r.span());
    app.set_range(TimeRange::fixed(a, b));
}

/// Wieder live mitlaufen (mit der Länge vor dem Anhalten bzw. des aktuellen Fensters).
pub fn resume(app: App) {
    let span = PAUSED_SPAN.write().take().unwrap_or_else(|| app.range.peek().span().clamp(60, 2_592_000));
    HISTORY.write().clear();
    app.set_range(TimeRange::live(span));
}
