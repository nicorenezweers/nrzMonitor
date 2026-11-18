//! Daten eines Diagramms für den gewählten Zeitraum. Im Live-Betrieb hängt der Live-Kanal neue
//! Punkte an; mehrere Diagramme auf einer Seite werden in **einer** Abfrage geholt.

use std::cell::RefCell;

use dioxus::prelude::*;
use futures::channel::oneshot;
use shared::query::{Agg, ChartData, QueryReq};

use super::geom;
use crate::{
    api,
    live::use_tick,
    state::{TimeRange, use_app},
};

/// Punkte je Reihe, die ein Diagramm anfordert.
pub const POINTS: u32 = 360;

type Waiting = (String, TimeRange, u32, oneshot::Sender<Option<ChartData>>);

thread_local! {
    static QUEUE: RefCell<Vec<Waiting>> = const { RefCell::new(Vec::new()) };
}

/// Ein Diagramm abfragen. Aufrufe im selben Augenblick gehen gebündelt an den Server.
pub async fn fetch(id: &str, range: TimeRange, points: u32) -> Option<ChartData> {
    let (tx, rx) = oneshot::channel();
    let first = QUEUE.with_borrow_mut(|q| {
        q.push((id.to_string(), range, points, tx));
        q.len() == 1
    });
    if first {
        spawn(async {
            gloo_timers::future::TimeoutFuture::new(15).await;
            let mut waiting = QUEUE.with_borrow_mut(std::mem::take);
            while !waiting.is_empty() {
                let (range, points) = (waiting[0].1, waiting[0].2);
                let (batch, rest): (Vec<Waiting>, Vec<Waiting>) = waiting.into_iter().partition(|w| w.1 == range && w.2 == points);
                waiting = rest;
                let req = QueryReq {
                    charts: batch.iter().map(|w| w.0.clone()).collect(),
                    after: range.after,
                    before: range.before,
                    points,
                    agg: Agg::Avg,
                    envelope: false,
                };
                let mut charts = api::query(&req).await.map(|r| r.charts).unwrap_or_default();
                for (id, _, _, tx) in batch {
                    let found = charts.iter().position(|c| c.id == id).map(|i| charts.swap_remove(i));
                    let _ = tx.send(found);
                }
            }
        });
    }
    rx.await.ok().flatten()
}

/// Daten eines Diagramms, passend zum globalen Zeitraum, live fortgeschrieben.
/// Die ID muss über die Lebenszeit der Komponente gleich bleiben (sonst `key` setzen).
pub fn use_chart(id: &str) -> Signal<Option<ChartData>> {
    use_feed(id, None).data
}

/// Daten eines Diagramms samt dem Zeitraum, zu dem sie gehören.
#[derive(Clone, Copy, PartialEq)]
pub struct Feed {
    pub data: Signal<Option<ChartData>>,
    /// Zeitraum, mit dem `data` geladen wurde (bleibt beim Umschalten stehen, bis Neues da ist).
    pub range: Signal<TimeRange>,
}

/// Wie [`use_chart`], die Punktzahl passt sich der Breite an (`width` in Pixeln, 0 = unbekannt).
pub fn use_feed(id: &str, width: Option<Signal<f64>>) -> Feed {
    let app = use_app();
    let mut data = use_signal(|| None::<ChartData>);
    let mut loaded = use_signal(|| *app.range.peek());
    let mut refresh = use_signal(|| 0u32);
    let tick = use_tick(id);
    let id = use_hook(|| id.to_string());

    let id_points = id.clone();
    let points = use_memo(move || {
        let span = (app.range)().span();
        let interval = app.charts.read().iter().find(|c| c.id == id_points).map_or(1, |m| m.interval.max(1));
        let w = width.map_or(0.0, |w| w());
        geom::points_for(span, interval, if w > 0.0 { w } else { 700.0 })
    });

    // Laden, wenn sich Zeitraum oder Punktzahl ändern (oder ein Neuladen verlangt wird).
    let id_load = id.clone();
    use_effect(move || {
        let range = (app.range)();
        let points = points();
        let _ = refresh();
        let id = id_load.clone();
        spawn(async move {
            let got = fetch(&id, range, points).await;
            if range == *app.range.peek() {
                // Ein Fehler oder eine leere Antwort ersetzt keine vorhandenen Daten desselben Zeitraums.
                if got.is_some() || *loaded.peek() != range {
                    data.set(got);
                }
                if *loaded.peek() != range {
                    loaded.set(range);
                }
            }
        });
    });

    // Live: neue Punkte anhängen, solange das Raster dem Takt des Diagramms entspricht.
    let id_live = id.clone();
    use_effect(move || {
        let Some((t, values)) = tick() else { return };
        let range = *app.range.peek();
        if !range.is_live() || *loaded.peek() != range {
            return;
        }
        let own = i64::from(app.chart(&id_live).map_or(1, |m| m.interval.max(1)));
        let mut guard = data.write();
        let Some(d) = guard.as_mut() else { return };
        let step = i64::from(d.step.max(1));
        if step != own {
            return; // verdichtet: wird unten regelmässig neu geladen
        }
        if values.len() != d.values.len() {
            drop(guard);
            refresh += 1; // Dimensionen haben sich geändert
            return;
        }
        let last = d.start + (d.len() as i64 - 1) * step;
        let slot = t.div_euclid(step) * step;
        if slot < last {
            return;
        }
        if slot == last {
            for (col, v) in d.values.iter_mut().zip(values.iter()) {
                if let Some(x) = col.last_mut() {
                    *x = *v;
                }
            }
            return;
        }
        let mut next = last + step;
        while next < slot {
            for col in &mut d.values {
                col.push(None);
            }
            next += step;
        }
        for (col, v) in d.values.iter_mut().zip(values.iter()) {
            col.push(*v);
        }
        let keep = (range.span() / step) as usize + 1;
        let excess = d.len().saturating_sub(keep);
        if excess > 0 {
            for col in &mut d.values {
                col.drain(..excess);
            }
            d.start += excess as i64 * step;
        }
    });

    // Verdichtete Live-Ansicht (z. B. 24 h): regelmässig neu laden statt anhängen.
    use_future(move || {
        let id = id.clone();
        async move {
            let mut waited = 0i64;
            loop {
                gloo_timers::future::TimeoutFuture::new(1000).await;
                waited += 1;
                let range = *app.range.peek();
                let step = data.peek().as_ref().map_or(1, |d| i64::from(d.step.max(1)));
                let own = i64::from(app.chart(&id).map_or(1, |m| m.interval.max(1)));
                let empty = data.peek().is_none();
                if range.is_live() && ((step != own && waited >= step.clamp(5, 60)) || (empty && waited >= 3)) {
                    waited = 0;
                    refresh += 1;
                }
            }
        }
    });

    Feed { data, range: loaded }
}
