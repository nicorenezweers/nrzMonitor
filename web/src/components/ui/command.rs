use dioxus::prelude::*;
use icons::Check;
use tw_merge::tw_merge;

use std::{cell::Cell, collections::HashMap, rc::Rc};

use wasm_bindgen::JsCast;
use web_sys::{Event, HtmlElement, KeyboardEvent};

use super::dom;
use crate::components::ui::button::{Button, ButtonVariant};

const TRIGGER_ID_QUALIFIER: &str = "command__trigger";

/* ========================================================== */
/*                     ✨ CONTEXT ✨                          */
/* ========================================================== */

#[derive(Clone)]
struct CommandDialogContext {
    dialog_id: String,
}

#[derive(Clone, Copy)]
struct CommandContext {
    search_query_signal: Signal<String>,
    should_filter: bool,
}

/* ========================================================== */
/*                     ✨ CLX COMPONENTS ✨                   */
/* ========================================================== */

#[component]
pub fn CommandHeader(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let merged = tw_merge!("flex flex-col gap-2 text-center hidden sm:text-left", class.as_deref().unwrap_or(""));
    rsx! { div { "data-name": "CommandHeader", class: "{merged}", {children} } }
}

#[component]
pub fn CommandTitle(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let merged = tw_merge!("text-lg font-semibold leading-none", class.as_deref().unwrap_or(""));
    rsx! { h2 { "data-name": "CommandTitle", class: "{merged}", {children} } }
}

#[component]
pub fn CommandDescription(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let merged = tw_merge!("text-sm text-muted-foreground", class.as_deref().unwrap_or(""));
    rsx! { p { "data-name": "CommandDescription", class: "{merged}", {children} } }
}

#[component]
pub fn CommandList(
    #[props(into, optional)] class: Option<String>,
    #[props(into, optional)] id: Option<String>,
    #[props(into, optional)] tabindex: Option<String>,
    children: Element,
) -> Element {
    let merged = tw_merge!(
        "overflow-y-auto overflow-x-hidden max-h-[300px] scroll-py-1 no__scrollbar min-h-80 scroll-pt-2 scroll-pb-1.5",
        class.as_deref().unwrap_or("")
    );
    rsx! {
        div {
            "data-name": "CommandList",
            class: "{merged}",
            id: id.as_deref(),
            tabindex: tabindex.as_deref(),
            {children}
        }
    }
}

#[component]
pub fn CommandGroup(
    #[props(into, optional)] class: Option<String>,
    #[props(into, optional)] role: Option<String>,
    #[props(into, optional)] data_category: Option<String>,
    children: Element,
) -> Element {
    let merged = tw_merge!("overflow-hidden p-1 text-foreground", class.as_deref().unwrap_or(""));
    rsx! {
        div {
            "data-name": "CommandGroup",
            class: "{merged}",
            role: role.as_deref(),
            "data-category": data_category.as_deref(),
            {children}
        }
    }
}

#[component]
pub fn CommandItemLink(
    #[props(into, optional)] class: Option<String>,
    #[props(into, optional)] href: Option<String>,
    #[props(into, optional)] target: Option<String>,
    #[props(into, optional)] rel: Option<String>,
    #[props(into, optional)] data_add_cmd: Option<String>,
    children: Element,
) -> Element {
    let merged = tw_merge!(
        "data-[selected=true]:text-accent-foreground [&_svg:not([class*='text-'])]:text-muted-foreground relative flex cursor-default hover:cursor-pointer items-center gap-2 px-2 py-1.5 text-sm outline-hidden select-none data-[disabled=true]:pointer-events-none data-[disabled=true]:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 data-[selected=true]:border-input data-[selected=true]:bg-muted/50 hover:bg-muted h-9 rounded-md border border-transparent font-medium",
        class.as_deref().unwrap_or("")
    );
    rsx! {
        a {
            "data-name": "CommandItemLink",
            class: "{merged}",
            href: href.as_deref(),
            target: target.as_deref(),
            rel: rel.as_deref(),
            "data-add-cmd": data_add_cmd.as_deref(),
            {children}
        }
    }
}

#[component]
pub fn CommandGroupLabel(
    #[props(into, optional)] class: Option<String>,
    #[props(into, optional)] aria_hidden: Option<String>,
    children: Element,
) -> Element {
    let merged = tw_merge!("text-muted-foreground px-2 py-1.5 text-xs font-medium", class.as_deref().unwrap_or(""));
    rsx! {
        div {
            "data-name": "CommandGroupLabel",
            class: "{merged}",
            "aria-hidden": aria_hidden.as_deref(),
            {children}
        }
    }
}

#[component]
pub fn CommandFooter(#[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let merged = tw_merge!(
        "flex gap-4 items-center px-4 h-10 text-xs font-medium rounded-b-xl border-t text-muted-foreground border-t-border bg-muted",
        class.as_deref().unwrap_or("")
    );
    rsx! { footer { "data-name": "CommandFooter", class: "{merged}", {children} } }
}

/* ========================================================== */
/*                     ✨ DIALOG PROVIDER ✨                  */
/* ========================================================== */

#[component]
pub fn CommandDialogProvider(children: Element, #[props(into)] id: String) -> Element {
    let context = CommandDialogContext { dialog_id: id };
    use_context_provider(|| context);
    rsx! { {children} }
}

#[component]
pub fn CommandDialogTrigger(children: Element, #[props(into, optional)] class: Option<String>) -> Element {
    let context = use_context::<CommandDialogContext>();
    let trigger_id = format!("{TRIGGER_ID_QUALIFIER}__{}", context.dialog_id);

    rsx! {
        Button {
            class: class.unwrap_or_default(),
            variant: ButtonVariant::Outline,
            id: "{trigger_id}",
            {children}
        }
    }
}

#[component]
pub fn CommandDialog(children: Element, #[props(into, optional)] class: Option<String>) -> Element {
    let context = use_context::<CommandDialogContext>();
    let merged_class = tw_merge!(
        "grid fixed z-100 gap-4 p-2 w-full bg-clip-padding rounded-xl border border-none ring-4 shadow-2xl sm:max-w-lg bg-background top-[50%] left-[50%] max-w-[calc(100%-2rem)] translate-x-[-50%] translate-y-[-50%] ring-neutral-200/80 transition-all duration-200 data-[state=closed]:opacity-0 data-[state=closed]:scale-95 data-[state=open]:opacity-100 data-[state=open]:scale-100",
        class.as_deref().unwrap_or("")
    );

    let dialog_id = context.dialog_id;
    let backdrop_id = format!("{dialog_id}__{TRIGGER_ID_QUALIFIER}");
    let trigger_id = format!("{TRIGGER_ID_QUALIFIER}__{dialog_id}");

    {
        let (d, b, t) = (dialog_id.clone(), backdrop_id.clone(), trigger_id.clone());
        use_effect(move || setup_command_dialog(d.clone(), b.clone(), t.clone()));
    }

    rsx! {
        // Backdrop
        div {
            "data-name": "CommandDialogBackdrop",
            id: "{backdrop_id}",
            class: "fixed inset-0 transition-opacity duration-200 pointer-events-none z-60 bg-black/50 data-[state=closed]:opacity-0 data-[state=open]:opacity-100",
            "data-state": "closed",
        }
        // Dialog
        div {
            "data-name": "CommandDialog",
            class: "{merged_class}",
            id: "{dialog_id}",
            "data-state": "closed",
            tabindex: "-1",
            style: "pointer-events: none;",
            {children}
        }
    }
}

/* ========================================================== */
/*                     ✨ COMMAND ✨                          */
/* ========================================================== */

#[component]
pub fn Command(
    children: Element,
    #[props(into, optional)] class: Option<String>,
    /// When false, disables client-side filtering (use for server-side search).
    /// Default: true (client-side filtering enabled).
    #[props(default = true)]
    should_filter: bool,
) -> Element {
    let dialog_context = try_use_context::<CommandDialogContext>();
    let search_query_signal = use_signal(String::new);
    let command_context = CommandContext { search_query_signal, should_filter };

    use_context_provider(|| command_context);

    let merged_class = tw_merge!(
        "flex overflow-hidden flex-col w-full h-full bg-transparent rounded-none text-popover-foreground",
        class.as_deref().unwrap_or("")
    );

    let scope = dialog_context.map(|ctx| {
        let backdrop_id = format!("{}__{TRIGGER_ID_QUALIFIER}", ctx.dialog_id);
        (ctx.dialog_id, backdrop_id)
    });
    use_effect(move || setup_command(scope.clone()));

    rsx! {
        div { "data-name": "Command", class: "{merged_class}", tabindex: "-1",
            {children}
        }
    }
}

#[component]
pub fn CommandInput(
    #[props(into, optional)] class: Option<String>,
    #[props(into, optional)] placeholder: Option<String>,
    /// Callback fired when search input changes. Use for server-side search.
    #[props(optional)]
    on_search_change: Option<EventHandler<String>>,
) -> Element {
    let command_context = use_context::<CommandContext>();
    let merged_class = tw_merge!(
        "flex py-3 w-full h-10 text-sm bg-transparent rounded-md disabled:opacity-50 disabled:cursor-not-allowed placeholder:text-muted-foreground outline-hidden",
        class.as_deref().unwrap_or("")
    );

    rsx! {
        input {
            "data-name": "CommandInput",
            class: "{merged_class}",
            placeholder: placeholder.as_deref(),
            autocomplete: "off",
            spellcheck: false,
            "aria-autocomplete": "list",
            role: "combobox",
            "aria-expanded": "true",
            "aria-controls": "command_demo",
            "aria-label": "Search documentation",
            r#type: "text",
            value: "{(command_context.search_query_signal)()}",
            oninput: move |ev| {
                let value = ev.value();
                let mut search_query_signal = command_context.search_query_signal;
                search_query_signal.set(value.clone());
                if let Some(cb) = on_search_change {
                    cb.call(value);
                }
            },
            autofocus: true,
            "data-1p-ignore": "true",
            "data-bwignore": "true",
            "data-lpignore": "true",
        }
    }
}

#[component]
pub fn CommandEmpty(children: Element, #[props(into, optional)] class: Option<String>) -> Element {
    let merged_class = tw_merge!("py-6 text-sm text-center", class.as_deref().unwrap_or(""));
    rsx! { div { "data-name": "CommandEmpty", class: "{merged_class}", {children} } }
}

#[component]
pub fn CommandItem(
    children: Element,
    #[props(into, optional)] class: Option<String>,
    #[props(into, optional)] value: Option<String>,
    #[props(optional)] on_select: Option<EventHandler<()>>,
    #[props(default = false)] selected: bool,
    /// Reserve space for check icon even when not selected (for alignment)
    #[props(default = false)]
    reserve_check_space: bool,
) -> Element {
    let command_context = use_context::<CommandContext>();
    let value_for_filter = value.unwrap_or_default();

    let merged_class = tw_merge!(
        "group relative flex gap-2 items-center px-2 py-1.5 text-sm rounded-sm cursor-default select-none outline-none data-[disabled=true]:pointer-events-none data-[disabled=true]:opacity-50 hover:bg-accent hover:text-accent-foreground",
        class.as_deref().unwrap_or("")
    );

    let is_visible = use_memo(move || {
        if !command_context.should_filter {
            return true;
        }
        let search = (command_context.search_query_signal)().to_lowercase();
        if search.is_empty() {
            return true;
        }
        value_for_filter.to_lowercase().contains(&search)
    });

    let check_class = if reserve_check_space {
        "ml-auto size-4 text-muted-foreground opacity-0 group-aria-selected:opacity-100"
    } else {
        "ml-auto size-4 text-muted-foreground hidden group-aria-selected:block"
    };

    rsx! {
        div {
            "data-name": "CommandItem",
            class: "{merged_class}",
            role: "option",
            tabindex: "0",
            "aria-selected": if selected { "true" } else { "false" },
            style: if is_visible() { "display: flex;" } else { "display: none;" },
            onclick: move |_| {
                if let Some(cb) = on_select {
                    cb.call(());
                }
            },
            {children}
            Check { class: check_class }
        }
    }
}

/* ========================================================== */
/*                     ✨ VERHALTEN ✨                        */
/* ========================================================== */

thread_local! {
    /// Am Dokument gebundene Rückrufe je Palette – beim erneuten Einrichten zuerst lösen.
    static DIALOG_HANDLERS: std::cell::RefCell<HashMap<String, Vec<(&'static str, js_sys::Function)>>> =
        std::cell::RefCell::default();
}

fn set_dialog_open(dialog_id: &str, backdrop_id: &str, open: bool) {
    if let (Some(d), Some(b)) = (dom::by_id(dialog_id), dom::by_id(backdrop_id)) {
        dom::set_open(&[&d, &b], open);
    }
}

// Öffnen über den Auslöser, ⌘K/Strg+K oder „/“; schliessen über Hintergrund und Escape.
fn setup_command_dialog(dialog_id: String, backdrop_id: String, trigger_id: String) {
    dom::when_ready(move || {
        if dom::by_id(&dialog_id).is_none() || dom::by_id(&backdrop_id).is_none() {
            return false;
        }
        let Some(doc) = dom::document() else { return true };
        DIALOG_HANDLERS.with(|h| {
            for (ty, f) in h.borrow_mut().remove(&dialog_id).unwrap_or_default() {
                dom::unlisten(&doc, ty, &f);
            }
        });

        let (d, b, t) = (dialog_id.clone(), backdrop_id.clone(), trigger_id.clone());
        let click = dom::listen(&doc, "click", move |e: Event| {
            if dom::target_element(&e).and_then(|el| el.closest(&format!("#{t}")).ok().flatten()).is_some() {
                set_dialog_open(&d, &b, true);
            }
        });
        let (d, b) = (dialog_id.clone(), backdrop_id.clone());
        let backdrop = dom::listen(&doc, "click", move |e: Event| {
            if dom::target_element(&e).and_then(|el| el.closest(&format!("#{b}")).ok().flatten()).is_some() {
                set_dialog_open(&d, &b, false);
            }
        });
        let (d, b) = (dialog_id.clone(), backdrop_id.clone());
        let key = dom::listen(&doc, "keydown", move |e: KeyboardEvent| {
            let Some(dialog) = dom::by_id(&d) else { return };
            let open = dom::is_open(&dialog);
            let k = e.key();
            if (e.meta_key() || e.ctrl_key()) && k == "k" {
                e.prevent_default();
                if !open {
                    set_dialog_open(&d, &b, true);
                }
            } else if k == "/" && !open {
                let typing = dom::document()
                    .and_then(|doc| doc.active_element())
                    .and_then(|el| el.dyn_into::<HtmlElement>().ok())
                    .is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA") || el.is_content_editable());
                if typing {
                    return;
                }
                e.prevent_default();
                set_dialog_open(&d, &b, true);
            } else if k == "Escape" && open {
                e.prevent_default();
                set_dialog_open(&d, &b, false);
            }
        });
        DIALOG_HANDLERS.with(|h| {
            h.borrow_mut().insert(dialog_id.clone(), vec![("click", click), ("click", backdrop), ("keydown", key)]);
        });
        true
    });
}

/// Liste der Palette: Filtern, Auswahl mit Pfeiltasten, Enter klickt den gewählten Eintrag.
struct PaletteList {
    list: HtmlElement,
    items: Vec<HtmlElement>,
    groups: Vec<HtmlElement>,
    in_dialog: bool,
    index: Cell<usize>,
}

fn hidden(el: &HtmlElement) -> bool {
    el.style().get_property_value("display").ok().as_deref() == Some("none")
}

impl PaletteList {
    fn visible(&self) -> Vec<&HtmlElement> {
        self.items
            .iter()
            .filter(|item| {
                if hidden(item) {
                    return false;
                }
                !self.in_dialog
                    || item
                        .closest("[data-name=\"CommandGroup\"]")
                        .ok()
                        .flatten()
                        .and_then(|g| g.dyn_into::<HtmlElement>().ok())
                        .is_none_or(|g| !hidden(&g))
            })
            .collect()
    }

    fn select(&self, i: usize) {
        let visible = self.visible();
        if visible.is_empty() {
            return;
        }
        for item in &self.items {
            let _ = item.set_attribute("aria-selected", "false");
        }
        if let Some(item) = visible.get(i) {
            let _ = item.set_attribute("aria-selected", "true");
            let opts = web_sys::ScrollIntoViewOptions::new();
            opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
            opts.set_behavior(web_sys::ScrollBehavior::Smooth);
            item.scroll_into_view_with_scroll_into_view_options(&opts);
        }
    }

    fn filter(&self, query: &str) {
        let q = query.trim().to_lowercase();
        for item in &self.items {
            let text = item.text_content().unwrap_or_default().to_lowercase();
            dom::set_style(item, "display", if q.is_empty() || text.contains(&q) { "" } else { "none" });
        }
        for group in &self.groups {
            let any = dom::query_all(group, "[data-name=\"CommandItemLink\"]").iter().any(|i| !hidden(i));
            dom::set_style(group, "display", if any { "" } else { "none" });
        }
        self.index.set(0);
        self.select(0);
    }

    fn key(&self, e: &KeyboardEvent) {
        let visible = self.visible();
        if visible.is_empty() {
            return;
        }
        let i = self.index.get();
        match e.key().as_str() {
            "ArrowDown" => {
                e.prevent_default();
                if i + 1 < visible.len() {
                    self.index.set(i + 1);
                    self.select(i + 1);
                }
            }
            "ArrowUp" => {
                e.prevent_default();
                if i > 0 {
                    self.index.set(i - 1);
                    self.select(i - 1);
                } else {
                    let opts = web_sys::ScrollToOptions::new();
                    opts.set_top(0.0);
                    opts.set_behavior(web_sys::ScrollBehavior::Smooth);
                    self.list.scroll_to_with_scroll_to_options(&opts);
                }
            }
            "Enter" => {
                e.prevent_default();
                if let Some(item) = visible.get(i) {
                    item.click();
                }
            }
            _ => {}
        }
    }
}

/// `scope`: (Dialog-ID, Hintergrund-ID), wenn die Liste in einem `CommandDialog` steht.
fn setup_command(scope: Option<(String, String)>) {
    dom::when_ready(move || {
        let dialog = match &scope {
            Some((d, _)) => match dom::by_id(d) {
                Some(d) => Some(d),
                None => return false,
            },
            None => None,
        };
        let root: web_sys::Element = match &dialog {
            Some(d) => d.clone().into(),
            None => match dom::document().and_then(|d| d.document_element()) {
                Some(r) => r,
                None => return true,
            },
        };
        let list = dom::query_all(&root, "[data-name=\"CommandList\"]").into_iter().next();
        let input = dom::query_all(&root, "[data-name=\"CommandInput\"]").into_iter().next();
        let Some(list) = list else { return false };
        let items = dom::query_all(&list, "[data-name=\"CommandItemLink\"]");
        if items.is_empty() || (dialog.is_some() && input.is_none()) {
            return false;
        }
        let groups = dom::query_all(&list, "[data-name=\"CommandGroup\"]");
        let cl = Rc::new(PaletteList { list, items, groups, in_dialog: dialog.is_some(), index: Cell::new(0) });

        if let Some(input) = &input {
            let cl = cl.clone();
            dom::listen(input, "input", move |e: Event| {
                let value = e
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                    .map(|i| i.value())
                    .unwrap_or_default();
                cl.filter(&value);
            });
        }
        let Some(doc) = dom::document() else { return true };

        match (dialog, &scope, input) {
            (Some(dialog), Some((_, backdrop_id)), Some(input)) => {
                // Eintrag gewählt → Dialog über den Hintergrund schliessen
                for item in &cl.items {
                    let b = backdrop_id.clone();
                    dom::listen(item, "click", move |_: Event| {
                        if let Some(b) = dom::by_id(&b) {
                            b.click();
                        }
                    });
                }
                dom::listen(&doc, "keydown", {
                    let (cl, dialog) = (cl.clone(), dialog.clone());
                    move |e: KeyboardEvent| {
                        if dom::is_open(&dialog) {
                            cl.key(&e);
                        }
                    }
                });
                // beim Öffnen: Suche leeren, erster Eintrag gewählt, Fokus ins Suchfeld
                let input = input.dyn_into::<web_sys::HtmlInputElement>().ok();
                let d = dialog.clone();
                let cb = wasm_bindgen::closure::Closure::<dyn FnMut(js_sys::Array)>::new(move |records: js_sys::Array| {
                    let changed = records.iter().any(|r| {
                        r.dyn_into::<web_sys::MutationRecord>()
                            .ok()
                            .and_then(|r| r.attribute_name())
                            .as_deref()
                            == Some("data-state")
                    });
                    if changed && dom::is_open(&d) {
                        if let Some(input) = &input {
                            input.set_value("");
                            let input = input.clone();
                            dom::later(0, move || {
                                let _ = input.focus();
                            });
                        }
                        cl.filter("");
                    }
                });
                if let Ok(observer) = web_sys::MutationObserver::new(cb.as_ref().unchecked_ref()) {
                    let opts = web_sys::MutationObserverInit::new();
                    opts.set_attributes(true);
                    let _ = observer.observe_with_options(&dialog, &opts);
                }
                cb.forget();
            }
            _ => {
                cl.select(0);
                dom::listen(&doc, "keydown", move |e: KeyboardEvent| cl.key(&e));
            }
        }
        true
    });
}
