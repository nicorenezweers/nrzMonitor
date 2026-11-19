use std::sync::atomic::{AtomicU64, Ordering};

use dioxus::prelude::*;
use tw_merge::tw_merge;

static POPOVER_COUNTER: AtomicU64 = AtomicU64::new(0);

fn use_popover_id() -> String {
    use_hook(|| {
        let id = POPOVER_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{id}")
    })
}

#[derive(Clone)]
struct PopoverContext {
    anchor_name: String,
    trigger_id: String,
    content_id: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum PopoverAlign {
    Start,
    StartOuter,
    End,
    EndOuter,
    #[default]
    Center,
}

#[component]
pub fn Popover(#[props(default = PopoverAlign::Center)] align: PopoverAlign, children: Element) -> Element {
    let id = use_popover_id();
    let anchor_name = format!("--anchor_{id}");
    let trigger_id = format!("popover-trigger-{id}");
    let content_id = format!("popover_{id}");

    provide_context(PopoverContext {
        anchor_name: anchor_name.clone(),
        trigger_id: trigger_id.clone(),
        content_id: content_id.clone(),
    });

    let (position_styles, transform_origin) = match align {
        PopoverAlign::Start => (
            "left: anchor(left); bottom: anchor(top); margin-bottom: 8px; @position-try(flip-block) { top: anchor(bottom); bottom: auto; margin-top: 8px; margin-bottom: 0; }".to_string(),
            "left top".to_string(),
        ),
        PopoverAlign::StartOuter => (
            "right: anchor(left); top: anchor(top); margin-right: 8px; @position-try(flip-block) { top: anchor(bottom); margin-top: 8px; }".to_string(),
            "right top".to_string(),
        ),
        PopoverAlign::End => (
            "right: anchor(right); bottom: anchor(top); margin-bottom: 8px; @position-try(flip-block) { top: anchor(bottom); bottom: auto; margin-top: 8px; margin-bottom: 0; }".to_string(),
            "right top".to_string(),
        ),
        PopoverAlign::EndOuter => (
            "left: anchor(right); top: anchor(top); margin-left: 8px; @position-try(flip-block) { top: anchor(bottom); margin-top: 8px; }".to_string(),
            "left top".to_string(),
        ),
        PopoverAlign::Center => ("position-area: block-start;".to_string(), "center top".to_string()),
    };

    let css = format!(
        "#{content_id} {{ position: fixed; position-anchor: {anchor_name}; {position_styles} transform-origin: {transform_origin}; }}"
    );

    use_effect(move || setup_popover(trigger_id.clone(), content_id.clone()));

    rsx! {
        style { dangerous_inner_html: "{css}" }
        {children}
    }
}

#[component]
pub fn PopoverTrigger(
    #[props(into, optional)] class: Option<String>,
    #[props(default = false)] disabled: bool,
    #[props(into, optional)] aria_label: Option<String>,
    children: Element,
) -> Element {
    let ctx = use_context::<PopoverContext>();
    rsx! {
        button {
            "data-name": "PopoverTrigger",
            id: "{ctx.trigger_id}",
            class: tw_merge!(
                "px-4 py-2 h-9 inline-flex justify-center items-center text-sm font-medium whitespace-nowrap rounded-md transition-colors w-fit focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 [&_svg:not(:last-child)]:mr-2 [&_svg:not(:first-child)]:ml-2  border bg-background border-input hover:bg-accent hover:text-accent-foreground",
                class.as_deref().unwrap_or("")
            ),
            disabled,
            "aria-label": aria_label.as_deref(),
            style: "anchor-name: {ctx.anchor_name};",
            {children}
        }
    }
}

#[component]
pub fn PopoverContent(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let ctx = use_context::<PopoverContext>();
    // Visual base copied verbatim from leptos PopoverContent. The trailing
    // pointer-events / opacity / data-[state=open] classes are dioxus's JS-state
    // equivalent of leptos's `&:popover-open` CSS (dioxus toggles data-state
    // instead of using the native popover attribute).
    let c = tw_merge!(
        "overflow-visible relative z-50 p-4 rounded-md border bg-card shadow-md my-[1ch] w-[250px] min-h-[150px] pointer-events-none opacity-0 transition-all duration-150 data-[state=open]:opacity-100 data-[state=open]:pointer-events-auto",
        class.as_deref().unwrap_or("")
    );
    rsx! {
        div {
            "data-name": "PopoverContent",
            id: "{ctx.content_id}",
            class: "{c}",
            "data-state": "closed",
            role: "dialog",
            {children}
        }
    }
}

#[component]
pub fn PopoverTitle(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let merged = tw_merge!("leading-none font-medium mb-3", class.as_deref().unwrap_or(""));
    rsx! { h3 { "data-name": "PopoverTitle", class: "{merged}", {children} } }
}

#[component]
pub fn PopoverDescription(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let merged = tw_merge!("text-muted-foreground text-sm", class.as_deref().unwrap_or(""));
    rsx! { p { "data-name": "PopoverDescription", class: "{merged}", {children} } }
}

// Verhalten: Auslöser schaltet um, Klick daneben und Escape schliessen.
fn setup_popover(tid: String, cid: String) {
    use super::dom;
    dom::when_ready(move || {
        let (Some(trigger), Some(content)) = (dom::by_id(&tid), dom::by_id(&cid)) else { return false };
        if !dom::init_once(&trigger, "data-popover-init") {
            return true;
        }
        let close = {
            let c = content.clone();
            move || {
                let _ = c.set_attribute("data-state", "closed");
            }
        };
        dom::listen(&trigger, "click", {
            let c = content.clone();
            move |e: web_sys::Event| {
                e.stop_propagation();
                let _ = c.set_attribute("data-state", if dom::is_open(&c) { "closed" } else { "open" });
            }
        });
        if let Some(doc) = dom::document() {
            dom::listen(&doc, "click", {
                let close = close.clone();
                let trigger_node: web_sys::Node = trigger.clone().into();
                move |e: web_sys::Event| {
                    let on_trigger = e.target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Node>(t).ok())
                        .is_some_and(|n| n.is_same_node(Some(&trigger_node)));
                    if !dom::contains_target(&content, &e) && !on_trigger {
                        close();
                    }
                }
            });
            dom::listen(&doc, "keydown", move |e: web_sys::KeyboardEvent| {
                if e.key() == "Escape" {
                    close();
                }
            });
        }
        true
    });
}
