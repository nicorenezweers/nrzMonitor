use dioxus::prelude::*;
use icons::{Check, ChevronRight};
use tw_merge::tw_merge;

use crate::components::hooks::use_random::use_random_id_for;
pub use crate::components::ui::separator::Separator as DropdownMenuSeparator;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum DropdownMenuAlign {
    #[default]
    Start,
    StartOuter,
    End,
    EndOuter,
    Center,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum DropdownMenuPosition {
    #[default]
    Auto,
    Top,
    Bottom,
}

#[derive(Clone)]
struct DropdownMenuContext {
    target_id: String,
    align: DropdownMenuAlign,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum DropdownMenuActionVariant {
    #[default]
    Default,
    Destructive,
}

#[component]
pub fn DropdownMenu(
    #[props(default = DropdownMenuAlign::default())] align: DropdownMenuAlign,
    children: Element,
) -> Element {
    let dropdown_target_id = use_random_id_for("dropdown");
    provide_context(DropdownMenuContext { target_id: dropdown_target_id, align });

    rsx! {
        div { "data-name": "DropdownMenu",
            {children}
        }
    }
}

#[component]
pub fn DropdownMenuTrigger(
    #[props(into, optional)] class: Option<String>,
    #[props(optional)] as_child: bool,
    children: Element,
) -> Element {
    let ctx = use_context::<DropdownMenuContext>();
    let merged = tw_merge!(
        "px-4 py-2 h-9 inline-flex justify-center items-center text-sm font-medium whitespace-nowrap rounded-md transition-colors w-fit focus:outline-none focus:ring-1 focus:ring-ring focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 [&_svg:not([class*='size-'])]:size-4 border bg-background border-input hover:bg-accent hover:text-accent-foreground",
        class.as_deref().unwrap_or("")
    );

    if as_child {
        rsx! { span { "data-name": "DropdownMenuTrigger", "data-dropdown-trigger": "{ctx.target_id}", class: "contents", {children} } }
    } else {
        rsx! {
            button {
                type: "button",
                class: "{merged}",
                "data-name": "DropdownMenuTrigger",
                "data-dropdown-trigger": "{ctx.target_id}",
                tabindex: "0",
                {children}
            }
        }
    }
}

#[component]
pub fn DropdownMenuContent(
    #[props(into, optional)] class: Option<String>,
    #[props(default = DropdownMenuPosition::default())] position: DropdownMenuPosition,
    children: Element,
) -> Element {
    let ctx = use_context::<DropdownMenuContext>();
    let width_class = match ctx.align {
        DropdownMenuAlign::Center => "min-w-full",
        _ => "w-[180px]",
    };
    let merged = tw_merge!(
        width_class,
        "z-50 p-1 rounded-md border bg-card shadow-md h-fit fixed transition-all duration-200 data-[state=closed]:opacity-0 data-[state=closed]:scale-95 data-[state=open]:opacity-100 data-[state=open]:scale-100",
        class.as_deref().unwrap_or("")
    );

    let align_attr = match ctx.align {
        DropdownMenuAlign::Start => "start",
        DropdownMenuAlign::StartOuter => "start-outer",
        DropdownMenuAlign::End => "end",
        DropdownMenuAlign::EndOuter => "end-outer",
        DropdownMenuAlign::Center => "center",
    };
    let position_attr = match position {
        DropdownMenuPosition::Auto => "auto",
        DropdownMenuPosition::Top => "top",
        DropdownMenuPosition::Bottom => "bottom",
    };
    let target_id = ctx.target_id;
    let tid = target_id.clone();
    use_effect(move || setup_dropdown(tid.clone()));

    rsx! {
        div {
            "data-name": "DropdownMenuContent",
            id: "{target_id}",
            class: "{merged}",
            "data-target": "target__dropdown",
            "data-state": "closed",
            "data-align": align_attr,
            "data-position": position_attr,
            style: "pointer-events: none;",
            {children}
        }
    }
}

#[component]
pub fn DropdownMenuLabel(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let merged = tw_merge!("px-2 py-1.5 text-sm font-medium data-inset:pl-8 mb-1", class.as_deref().unwrap_or(""));
    rsx! { span { "data-name": "DropdownMenuLabel", class: "{merged}", {children} } }
}

#[component]
pub fn DropdownMenuGroup(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let merged = tw_merge!("group", class.as_deref().unwrap_or(""));
    rsx! { ul { "data-name": "DropdownMenuGroup", class: "{merged}", {children} } }
}

#[component]
pub fn DropdownMenuItem(
    #[props(into, optional)] class: Option<String>,
    #[props(optional)] onclick: Option<EventHandler<MouseEvent>>,
    children: Element,
) -> Element {
    let merged = tw_merge!(
        "inline-flex gap-2 items-center w-full rounded-sm px-2 py-1.5 text-sm no-underline transition-colors duration-200 text-popover-foreground hover:bg-accent hover:text-accent-foreground [&_svg:not([class*='size-'])]:size-4",
        class.as_deref().unwrap_or("")
    );
    rsx! {
        li {
            "data-name": "DropdownMenuItem",
            class: "{merged}",
            onclick: move |e| {
                if let Some(handler) = &onclick {
                    handler.call(e);
                }
            },
            {children}
        }
    }
}

#[component]
pub fn DropdownMenuLink(
    #[props(into, optional)] href: Option<String>,
    #[props(into, optional)] target: Option<String>,
    #[props(into, optional)] rel: Option<String>,
    children: Element,
) -> Element {
    rsx! {
        a {
            "data-name": "DropdownMenuLink",
            class: "w-full inline-flex gap-2 items-center",
            href: "{href.as_deref().unwrap_or(\"#\")}",
            target: target,
            rel: rel,
            {children}
        }
    }
}

#[component]
pub fn DropdownMenuAction(
    #[props(into, optional)] class: Option<String>,
    #[props(into, optional)] href: Option<String>,
    #[props(default = DropdownMenuActionVariant::default())] variant: DropdownMenuActionVariant,
    children: Element,
) -> Element {
    let variant_class = match variant {
        DropdownMenuActionVariant::Default => "text-popover-foreground hover:bg-accent hover:text-accent-foreground",
        DropdownMenuActionVariant::Destructive => "text-destructive hover:bg-destructive/10 hover:text-destructive",
    };
    let merged = tw_merge!(
        "inline-flex gap-2 items-center w-full text-sm text-left transition-colors duration-200 focus:outline-none focus-visible:outline-none [&_svg:not([class*='size-'])]:size-4",
        variant_class,
        class.as_deref().unwrap_or("")
    );
    if let Some(href) = href {
        rsx! { a { "data-name": "DropdownMenuAction", class: "{merged}", href: "{href}", "data-dropdown-close": "true", {children} } }
    } else {
        rsx! { button { "data-name": "DropdownMenuAction", class: "{merged}", type: "button", "data-dropdown-close": "true", {children} } }
    }
}

#[component]
pub fn DropdownMenuSub(children: Element) -> Element {
    rsx! { li { "data-name": "DropdownMenuSub", class: "dropdown__menu_sub_trigger relative inline-flex relative gap-2 items-center py-1.5 px-2 w-full text-sm no-underline rounded-sm transition-colors duration-200 cursor-pointer text-popover-foreground [&_svg:not([class*='size-'])]:size-4 hover:bg-accent hover:text-accent-foreground", {children} } }
}

#[component]
pub fn DropdownMenuSubTrigger(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let merged = tw_merge!("flex items-center justify-between w-full", class.as_deref().unwrap_or(""));
    rsx! {
        span { "data-name": "DropdownMenuSubTrigger", class: "{merged}",
            span { class: "flex gap-2 items-center", {children} }
            ChevronRight { class: "opacity-70 size-4" }
        }
    }
}

#[component]
pub fn DropdownMenuSubContent(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let merged = tw_merge!(
        "dropdown__menu_sub_content rounded-md border bg-card shadow-lg p-1 absolute z-[100] min-w-[160px] opacity-0 invisible translate-x-[-8px] transition-all duration-200 ease-out pointer-events-none",
        class.as_deref().unwrap_or("")
    );
    rsx! { ul { "data-name": "DropdownMenuSubContent", class: "{merged}", {children} } }
}

#[component]
pub fn DropdownMenuSubItem(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let merged = tw_merge!(
        "inline-flex gap-2 items-center w-full rounded-sm px-3 py-2 text-sm transition-all duration-150 ease text-popover-foreground hover:bg-accent hover:text-accent-foreground cursor-pointer hover:translate-x-[2px]",
        class.as_deref().unwrap_or("")
    );
    rsx! { li { "data-name": "DropdownMenuSubItem", class: "{merged}", "data-dropdown-close": "true", {children} } }
}

#[component]
pub fn DropdownMenuRadioGroup<T: Clone + PartialEq + Send + Sync + 'static>(
    children: Element,
    value: Signal<T>,
) -> Element {
    provide_context(value);
    rsx! { ul { "data-name": "DropdownMenuRadioGroup", role: "group", class: "group", {children} } }
}

#[component]
pub fn DropdownMenuRadioItem<T: Clone + PartialEq + Send + Sync + 'static>(
    children: Element,
    value: T,
    #[props(into, optional)] class: Option<String>,
) -> Element {
    let mut selected = use_context::<Signal<T>>();
    let value_for_check = value.clone();
    let value_for_click = value;
    let class = tw_merge!(
        "group inline-flex gap-2 items-center w-full rounded-sm pl-2 pr-2 py-1.5 text-sm cursor-pointer no-underline transition-colors duration-200 text-popover-foreground hover:bg-accent hover:text-accent-foreground [&_svg:not([class*='size-'])]:size-4",
        class.as_deref().unwrap_or("")
    );
    let is_selected = move || selected() == value_for_check;
    rsx! {
        li {
            "data-name": "DropdownMenuRadioItem",
            class: "{class}",
            role: "menuitemradio",
            "aria-checked": if is_selected() { "true" } else { "false" },
            "data-dropdown-close": "true",
            onclick: move |_| selected.set(value_for_click.clone()),
            {children}
            Check { class: "ml-auto opacity-0 size-4 text-muted-foreground group-aria-checked:opacity-100" }
        }
    }
}

// ---------------------------------------------------------------------------
// Verhalten: Auslöser schaltet um (andere offene Menüs schliessen zuerst), Klick daneben,
// Aktionen und Escape schliessen; Lage nach `data-align`/`data-position` am Auslöser.
// ---------------------------------------------------------------------------

fn place_dropdown(dropdown: &web_sys::HtmlElement, trigger: &web_sys::HtmlElement) {
    use super::dom;
    let window = web_sys::window();
    let contents = window
        .as_ref()
        .and_then(|w| w.get_computed_style(trigger).ok().flatten())
        .and_then(|s| s.get_property_value("display").ok())
        .is_some_and(|d| d == "contents");
    let anchor: web_sys::Element =
        if contents { trigger.first_element_child().unwrap_or_else(|| trigger.clone().into()) } else { trigger.clone().into() };
    let t = anchor.get_bounding_client_rect();
    let d = dropdown.get_bounding_client_rect();
    let vh = window.and_then(|w| w.inner_height().ok()).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let below = vh - t.bottom();
    let above = t.top();
    let align = dropdown.get_attribute("data-align").unwrap_or_else(|| "start".into());
    let up = match dropdown.get_attribute("data-position").as_deref() {
        Some("top") => true,
        Some("bottom") => false,
        _ => above >= d.height() && below < d.height(),
    };
    let top_above = t.top() - d.height() - 6.0;
    let (top, left, origin) = match align.as_str() {
        "end" => (if up { top_above } else { t.bottom() + 6.0 }, t.right() - d.width(), if up { "right bottom" } else { "right top" }),
        "start-outer" => (if up { top_above } else { t.top() }, t.left() - d.width() - 16.0, if up { "right bottom" } else { "right top" }),
        "end-outer" => (if up { top_above } else { t.top() }, t.right() + 8.0, if up { "left bottom" } else { "left top" }),
        "center" => {
            dom::set_style(dropdown, "min-width", &format!("{}px", t.width()));
            (if up { top_above } else { t.bottom() + 6.0 }, t.left(), if up { "center bottom" } else { "center top" })
        }
        _ => (if up { top_above } else { t.bottom() + 6.0 }, t.left(), if up { "left bottom" } else { "left top" }),
    };
    dom::set_style(dropdown, "top", &format!("{top}px"));
    dom::set_style(dropdown, "left", &format!("{left}px"));
    dom::set_style(dropdown, "transform-origin", origin);
}

fn setup_dropdown(tid: String) {
    use super::dom;
    use std::{cell::RefCell, rc::Rc};

    dom::when_ready(move || {
        let trigger_sel = format!("[data-dropdown-trigger=\"{tid}\"]");
        let (Some(dropdown), Some(trigger)) = (dom::by_id(&tid), dom::query(&trigger_sel)) else { return false };
        if !dom::init_once(&dropdown, "data-initialized") {
            return true;
        }
        let Some(doc) = dom::document() else { return true };
        let outside: Rc<RefCell<Option<js_sys::Function>>> = Rc::default();

        let close: Rc<dyn Fn()> = {
            let (dd, outside, doc) = (dropdown.clone(), outside.clone(), doc.clone());
            Rc::new(move || {
                dom::set_open(&[&dd], false);
                if let Some(f) = outside.borrow_mut().take() {
                    dom::unlisten(&doc, "click", &f);
                }
            })
        };
        let open: Rc<dyn Fn()> = {
            let (dd, t, outside, doc, close) = (dropdown.clone(), trigger.clone(), outside.clone(), doc.clone(), close.clone());
            Rc::new(move || {
                dom::set_open(&[&dd], true);
                dom::set_style(&dd, "visibility", "hidden");
                let _ = dd.offset_height();
                place_dropdown(&dd, &t);
                dom::set_style(&dd, "visibility", "visible");
                let (dd2, t2, outside2, doc2, close2) = (dd.clone(), t.clone(), outside.clone(), doc.clone(), close.clone());
                dom::later(0, move || {
                    if !dom::is_open(&dd2) || outside2.borrow().is_some() {
                        return;
                    }
                    let f = dom::listen(&doc2, "click", move |e: web_sys::Event| {
                        if !dom::contains_target(&dd2, &e) && !dom::contains_target(&t2, &e) {
                            close2();
                        }
                    });
                    *outside2.borrow_mut() = Some(f);
                });
            })
        };
        dom::listen(&trigger, "click", {
            let (dd, open, close) = (dropdown.clone(), open.clone(), close.clone());
            move |e: web_sys::Event| {
                e.stop_propagation();
                let mut other_open = false;
                for other in dom::query_doc("[data-target=\"target__dropdown\"]") {
                    if !other.is_same_node(Some(&dd)) && dom::is_open(&other) {
                        other_open = true;
                        dom::set_open(&[&other], false);
                    }
                }
                if other_open {
                    return;
                }
                if dom::is_open(&dd) { close() } else { open() }
            }
        });
        for action in dom::query_all(&dropdown, "[data-dropdown-close]") {
            let close = close.clone();
            dom::listen(&action, "click", move |_: web_sys::Event| close());
        }
        dom::listen(&doc, "keydown", move |e: web_sys::KeyboardEvent| {
            if e.key() == "Escape" && dom::is_open(&dropdown) {
                e.prevent_default();
                close();
            }
        });
        true
    });
}
