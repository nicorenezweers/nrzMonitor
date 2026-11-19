//! DOM-Helfer der nrzUI-Bausteine: Elemente suchen, Ereignisse binden, Offen/Zu-Zustand setzen.
//! Die Bausteine richten ihr Verhalten nach dem Einhängen damit ein (`use_effect` + [`when_ready`]).

use gloo_timers::callback::Timeout;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{Document, Element, Event, EventTarget, HtmlElement, Node};

pub fn document() -> Option<Document> {
    web_sys::window()?.document()
}

pub fn by_id(id: &str) -> Option<HtmlElement> {
    document()?.get_element_by_id(id)?.dyn_into().ok()
}

pub fn query(sel: &str) -> Option<HtmlElement> {
    document()?.query_selector(sel).ok()??.dyn_into().ok()
}

/// Alle passenden HTML-Elemente unterhalb von `root`.
pub fn query_all(root: &Element, sel: &str) -> Vec<HtmlElement> {
    let Ok(list) = root.query_selector_all(sel) else { return Vec::new() };
    (0..list.length()).filter_map(|i| list.item(i)?.dyn_into().ok()).collect()
}

/// Alle passenden HTML-Elemente im Dokument.
pub fn query_doc(sel: &str) -> Vec<HtmlElement> {
    document().and_then(|d| d.document_element()).map(|r| query_all(&r, sel)).unwrap_or_default()
}

/// Bindet `f` an `target` und gibt die JS-Funktion zurück (zum späteren Lösen mit [`unlisten`]).
pub fn listen<E: JsCast + 'static>(target: &EventTarget, ty: &str, mut f: impl FnMut(E) + 'static) -> js_sys::Function {
    let cb = Closure::<dyn FnMut(Event)>::new(move |e: Event| f(e.unchecked_into()));
    let func: js_sys::Function = cb.as_ref().unchecked_ref::<js_sys::Function>().clone();
    let _ = target.add_event_listener_with_callback(ty, &func);
    cb.forget();
    func
}

pub fn unlisten(target: &EventTarget, ty: &str, func: &js_sys::Function) {
    let _ = target.remove_event_listener_with_callback(ty, func);
}

/// Führt `f` aus, bis es `true` meldet (alle 50 ms neu, solange die Elemente noch fehlen).
pub fn when_ready(mut f: impl FnMut() -> bool + 'static) {
    if !f() {
        Timeout::new(50, move || when_ready(f)).forget();
    }
}

/// Nach `ms` Millisekunden einmal ausführen.
pub fn later(ms: u32, f: impl FnOnce() + 'static) {
    Timeout::new(ms, f).forget();
}

/// Markiert `el` als eingerichtet; `false`, wenn das schon geschehen ist.
pub fn init_once(el: &Element, attr: &str) -> bool {
    if el.has_attribute(attr) {
        return false;
    }
    let _ = el.set_attribute(attr, "true");
    true
}

pub fn is_open(el: &Element) -> bool {
    el.get_attribute("data-state").as_deref() == Some("open")
}

/// Setzt `data-state` und `pointer-events` für alle Elemente.
pub fn set_open(els: &[&HtmlElement], open: bool) {
    for el in els {
        let _ = el.set_attribute("data-state", if open { "open" } else { "closed" });
        let _ = el.style().set_property("pointer-events", if open { "auto" } else { "none" });
    }
}

pub fn set_style(el: &HtmlElement, prop: &str, value: &str) {
    let _ = el.style().set_property(prop, value);
}

/// Liegt das Ziel des Ereignisses in `el`?
pub fn contains_target(el: &Node, e: &Event) -> bool {
    let target = e.target().and_then(|t| t.dyn_into::<Node>().ok());
    el.contains(target.as_ref())
}

pub fn target_element(e: &Event) -> Option<Element> {
    e.target()?.dyn_into().ok()
}
