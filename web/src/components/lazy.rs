//! Inhalt erst aufbauen, wenn er (fast) im Bild ist, und wieder abbauen, wenn er weit weg ist.
//! Ein abgebautes Diagramm fragt nichts ab und hört im Live-Kanal nicht mehr zu – so bleiben auch
//! Seiten mit hunderten Diagrammen leicht.

use std::{cell::RefCell, rc::Rc};

use dioxus::{prelude::*, web::WebEventExt};
use futures::{StreamExt, channel::mpsc};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{IntersectionObserver, IntersectionObserverEntry, IntersectionObserverInit};

type Watch = (IntersectionObserver, Closure<dyn FnMut(js_sys::Array)>);

#[component]
pub fn Lazy(
    /// Platzhalterhöhe in Pixeln, solange der Inhalt nicht aufgebaut ist.
    #[props(default = 260)]
    min_height: u32,
    #[props(into, optional)] class: Option<String>,
    children: Element,
) -> Element {
    let mut visible = use_signal(|| false);
    let watch = use_hook(|| Rc::new(RefCell::new(None::<Watch>)));
    // Der Beobachter ruft von ausserhalb – die Meldung kommt über einen Kanal herein.
    let (tx, rx) = use_hook(|| {
        let (tx, rx) = mpsc::unbounded::<bool>();
        (tx, Rc::new(RefCell::new(Some(rx))))
    });
    use_future(move || {
        let rx = rx.borrow_mut().take();
        async move {
            let Some(mut rx) = rx else { return };
            while let Some(v) = rx.next().await {
                if *visible.peek() != v {
                    visible.set(v);
                }
            }
        }
    });
    let watch_drop = watch.clone();
    use_drop(move || {
        if let Some((observer, _)) = watch_drop.borrow_mut().take() {
            observer.disconnect();
        }
    });

    let style = if visible() { String::new() } else { format!("min-height:{min_height}px") };
    rsx! {
        div {
            class,
            style: "{style}",
            onmounted: move |e| {
                if watch.borrow().is_some() {
                    return;
                }
                let element = e.data().as_web_event();
                let tx = tx.clone();
                let callback = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
                    for entry in entries.iter() {
                        if let Ok(entry) = entry.dyn_into::<IntersectionObserverEntry>() {
                            let _ = tx.unbounded_send(entry.is_intersecting());
                        }
                    }
                });
                let options = IntersectionObserverInit::new();
                options.set_root_margin("500px 0px");
                match IntersectionObserver::new_with_options(callback.as_ref().unchecked_ref(), &options) {
                    Ok(observer) => {
                        observer.observe(&element);
                        *watch.borrow_mut() = Some((observer, callback));
                    }
                    // Ohne Beobachter einfach immer zeigen.
                    Err(_) => visible.set(true),
                }
            },
            if visible() {
                {children}
            }
        }
    }
}
