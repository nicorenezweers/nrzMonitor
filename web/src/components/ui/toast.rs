use dioxus::prelude::*;
use tw_merge::tw_merge;

#[derive(Clone, Copy, PartialEq, Eq, Default, strum::Display)]
pub enum ToastType {
    #[default]
    Default,
    Success,
    Error,
    Warning,
    Info,
    Loading,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, strum::Display)]
pub enum ToastPosition {
    TopLeft,
    TopCenter,
    TopRight,
    #[default]
    BottomRight,
    BottomCenter,
    BottomLeft,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, strum::Display)]
pub enum ToastDirection {
    TopDown,
    #[default]
    BottomUp,
}

/// Eine sichtbare Meldung im [`ToastToaster`].
#[derive(Clone, PartialEq)]
struct ToastItem {
    id: u64,
    variant: ToastType,
    title: String,
}

static TOASTS: GlobalSignal<Vec<ToastItem>> = Signal::global(Vec::new);
static NEXT_TOAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// So lange bleibt eine Meldung stehen (Laden: bis sie ersetzt oder weggeklickt wird).
const TOAST_MS: u32 = 4000;

/// Zeigt eine Meldung im [`ToastToaster`] (der einmal im Rahmen stehen muss).
pub fn show_toast(variant: ToastType, title: impl Into<String>) {
    let id = NEXT_TOAST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    TOASTS.write().push(ToastItem { id, variant, title: title.into() });
    if variant != ToastType::Loading {
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(TOAST_MS).await;
            dismiss_toast(id);
        });
    }
}

fn dismiss_toast(id: u64) {
    TOASTS.write().retain(|t| t.id != id);
}

#[component]
pub fn ToastTrigger(
    children: Element,
    #[props(into, optional)] class: Option<String>,
    #[props(default = ToastType::Default)] variant: ToastType,
    #[props(into)] title: String,
    #[props(into)] description: String,
    #[props(into, optional)] position: Option<String>,
) -> Element {
    let variant_classes = match variant {
        ToastType::Default => "bg-primary text-primary-foreground shadow-xs hover:bg-primary/90",
        ToastType::Success => "bg-success text-success-foreground hover:bg-success/90",
        ToastType::Error => "bg-destructive text-white shadow-xs hover:bg-destructive/90 dark:bg-destructive/60",
        ToastType::Warning => "bg-warning text-warning-foreground hover:bg-warning/90",
        ToastType::Info => "bg-info text-info-foreground shadow-xs hover:bg-info/90",
        ToastType::Loading => "bg-secondary text-secondary-foreground shadow-xs hover:bg-secondary/80",
    };

    let merged_class = tw_merge!(
        "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-all disabled:pointer-events-none disabled:opacity-50 outline-none focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] w-fit cursor-pointer h-9 px-4 py-2",
        variant_classes,
        class.as_deref().unwrap_or("")
    );

    rsx! {
        button {
            class: "{merged_class}",
            "data-name": "ToastTrigger",
            "data-variant": "{variant}",
            "data-toast-title": "{title}",
            "data-toast-description": "{description}",
            "data-toast-position": position,
            r#type: "button",
            {children}
        }
    }
}

#[component]
pub fn ToastContainer(
    children: Element,
    #[props(into, optional)] class: Option<String>,
    #[props(default = ToastPosition::BottomRight)] position: ToastPosition,
) -> Element {
    let merged_class = tw_merge!("toast__container fixed z-50", class.as_deref().unwrap_or(""));

    rsx! {
        div {
            class: "{merged_class}",
            "data-position": "{position}",
            {children}
        }
    }
}

#[component]
pub fn ToastList(
    children: Element,
    #[props(into, optional)] class: Option<String>,
    #[props(default = ToastPosition::BottomRight)] position: ToastPosition,
    #[props(default = ToastDirection::BottomUp)] direction: ToastDirection,
    #[props(into, optional)] expanded: Option<String>,
    #[props(into, optional)] style: Option<String>,
) -> Element {
    let merged_class = tw_merge!(
        "flex relative flex-col opacity-100 gap-[15px] h-[100px] w-[400px] pointer-events-none [&>*]:pointer-events-auto",
        class.as_deref().unwrap_or("")
    );
    let expanded_val = expanded.as_deref().unwrap_or("false");

    rsx! {
        ol {
            class: "{merged_class}",
            "data-name": "ToastList",
            "data-toast-toaster": "true",
            "data-toast-theme": "light",
            "data-position": "{position}",
            "data-expanded": "{expanded_val}",
            "data-direction": "{direction}",
            style: style,
            {children}
        }
    }
}

#[component]
pub fn ToastToaster(#[props(default = ToastPosition::BottomRight)] position: ToastPosition) -> Element {
    let direction = match position {
        ToastPosition::TopLeft | ToastPosition::TopCenter | ToastPosition::TopRight => ToastDirection::TopDown,
        _ => ToastDirection::BottomUp,
    };

    let container_class = match position {
        ToastPosition::TopLeft => "left-6 top-6",
        ToastPosition::TopRight => "right-6 top-6",
        ToastPosition::TopCenter => "left-1/2 -translate-x-1/2 top-6",
        ToastPosition::BottomCenter => "left-1/2 -translate-x-1/2 bottom-6",
        ToastPosition::BottomLeft => "left-6 bottom-6",
        ToastPosition::BottomRight => "right-6 bottom-6",
    };
    let list_class = match direction {
        ToastDirection::TopDown => "h-auto w-[min(400px,calc(100vw-3rem))]",
        ToastDirection::BottomUp => "h-auto w-[min(400px,calc(100vw-3rem))] flex-col-reverse",
    };

    rsx! {
        ToastContainer { class: container_class, position,
            ToastList { class: list_class, position, direction,
                for t in TOASTS.read().iter().cloned() {
                    ToastView { key: "{t.id}", item: t }
                }
            }
        }
    }
}

#[component]
fn ToastView(item: ToastItem) -> Element {
    let accent = match item.variant {
        ToastType::Success => "border-l-success",
        ToastType::Error => "border-l-destructive",
        ToastType::Warning => "border-l-warning",
        ToastType::Info => "border-l-info",
        ToastType::Default | ToastType::Loading => "border-l-primary",
    };
    let id = item.id;
    rsx! {
        li {
            "data-name": "Toast",
            "data-variant": "{item.variant}",
            role: if item.variant == ToastType::Error { "alert" } else { "status" },
            class: "flex items-start gap-3 rounded-lg border border-l-4 bg-popover p-4 text-sm text-popover-foreground shadow-lg animate-nrz-toast {accent}",
            span { class: "flex-1 break-words", "{item.title}" }
            button {
                r#type: "button",
                class: "shrink-0 rounded-sm p-0.5 text-muted-foreground hover:text-foreground cursor-pointer [&_svg]:size-4",
                "aria-label": "Meldung schliessen",
                onclick: move |_| dismiss_toast(id),
                icons::X {}
            }
        }
    }
}
