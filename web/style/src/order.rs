//! Reihenfolge der Regeln wie bei Tailwind CSS v4.1 (MIT-Lizenz, siehe `LICENSE-tailwindcss`):
//! zuerst nach Varianten, dann nach der ersten Eigenschaft in dieser Liste, dann nach Anzahl der
//! Deklarationen, zuletzt nach dem Klassennamen. So gewinnt bei gleichen Eigenschaften dieselbe Klasse
//! wie bisher (z. B. `px-4 pl-2` → `pl-2`).

pub const PROPERTY_ORDER: &[&str] = &[
    "container-type", "pointer-events", "visibility", "position", "inset", "inset-inline", "inset-block",
    "inset-inline-start", "inset-inline-end", "top", "right", "bottom", "left", "isolation", "z-index",
    "order", "grid-column", "grid-column-start", "grid-column-end", "grid-row", "grid-row-start",
    "grid-row-end", "float", "clear", "--tw-container-component", "margin", "margin-inline", "margin-block",
    "margin-inline-start", "margin-inline-end", "margin-top", "margin-right", "margin-bottom", "margin-left",
    "box-sizing", "display", "field-sizing", "aspect-ratio", "height", "max-height", "min-height", "width",
    "max-width", "min-width", "flex", "flex-shrink", "flex-grow", "flex-basis", "table-layout", "caption-side",
    "border-collapse", "border-spacing", "transform-origin", "translate", "--tw-translate-x",
    "--tw-translate-y", "--tw-translate-z", "scale", "--tw-scale-x", "--tw-scale-y", "--tw-scale-z", "rotate",
    "--tw-rotate-x", "--tw-rotate-y", "--tw-rotate-z", "--tw-skew-x", "--tw-skew-y", "transform", "animation",
    "cursor", "touch-action", "--tw-pan-x", "--tw-pan-y", "--tw-pinch-zoom", "resize", "scroll-snap-type",
    "--tw-scroll-snap-strictness", "scroll-snap-align", "scroll-snap-stop", "scroll-margin",
    "scroll-margin-inline", "scroll-margin-block", "scroll-margin-inline-start", "scroll-margin-inline-end",
    "scroll-margin-top", "scroll-margin-right", "scroll-margin-bottom", "scroll-margin-left", "scroll-padding",
    "scroll-padding-inline", "scroll-padding-block", "scroll-padding-inline-start",
    "scroll-padding-inline-end", "scroll-padding-top", "scroll-padding-right", "scroll-padding-bottom",
    "scroll-padding-left", "list-style-position", "list-style-type", "list-style-image", "appearance",
    "columns", "break-before", "break-inside", "break-after", "grid-auto-columns", "grid-auto-flow",
    "grid-auto-rows", "grid-template-columns", "grid-template-rows", "flex-direction", "flex-wrap",
    "place-content", "place-items", "align-content", "align-items", "justify-content", "justify-items", "gap",
    "column-gap", "row-gap", "--tw-space-x-reverse", "--tw-space-y-reverse", "divide-x-width",
    "divide-y-width", "--tw-divide-y-reverse", "divide-style", "divide-color", "place-self", "align-self",
    "justify-self", "overflow", "overflow-x", "overflow-y", "overscroll-behavior", "overscroll-behavior-x",
    "overscroll-behavior-y", "scroll-behavior", "border-radius", "border-start-radius", "border-end-radius",
    "border-top-radius", "border-right-radius", "border-bottom-radius", "border-left-radius",
    "border-start-start-radius", "border-start-end-radius", "border-end-end-radius", "border-end-start-radius",
    "border-top-left-radius", "border-top-right-radius", "border-bottom-right-radius",
    "border-bottom-left-radius", "border-width", "border-inline-width", "border-block-width",
    "border-inline-start-width", "border-inline-end-width", "border-top-width", "border-right-width",
    "border-bottom-width", "border-left-width", "border-style", "border-inline-style", "border-block-style",
    "border-inline-start-style", "border-inline-end-style", "border-top-style", "border-right-style",
    "border-bottom-style", "border-left-style", "border-color", "border-inline-color", "border-block-color",
    "border-inline-start-color", "border-inline-end-color", "border-top-color", "border-right-color",
    "border-bottom-color", "border-left-color", "background-color", "background-image",
    "--tw-gradient-position", "--tw-gradient-stops", "--tw-gradient-via-stops", "--tw-gradient-from",
    "--tw-gradient-from-position", "--tw-gradient-via", "--tw-gradient-via-position", "--tw-gradient-to",
    "--tw-gradient-to-position", "mask-image", "--tw-mask-top", "--tw-mask-top-from-color",
    "--tw-mask-top-from-position", "--tw-mask-top-to-color", "--tw-mask-top-to-position", "--tw-mask-right",
    "--tw-mask-right-from-color", "--tw-mask-right-from-position", "--tw-mask-right-to-color",
    "--tw-mask-right-to-position", "--tw-mask-bottom", "--tw-mask-bottom-from-color",
    "--tw-mask-bottom-from-position", "--tw-mask-bottom-to-color", "--tw-mask-bottom-to-position",
    "--tw-mask-left", "--tw-mask-left-from-color", "--tw-mask-left-from-position", "--tw-mask-left-to-color",
    "--tw-mask-left-to-position", "--tw-mask-linear", "--tw-mask-linear-position",
    "--tw-mask-linear-from-color", "--tw-mask-linear-from-position", "--tw-mask-linear-to-color",
    "--tw-mask-linear-to-position", "--tw-mask-radial", "--tw-mask-radial-shape", "--tw-mask-radial-size",
    "--tw-mask-radial-position", "--tw-mask-radial-from-color", "--tw-mask-radial-from-position",
    "--tw-mask-radial-to-color", "--tw-mask-radial-to-position", "--tw-mask-conic", "--tw-mask-conic-position",
    "--tw-mask-conic-from-color", "--tw-mask-conic-from-position", "--tw-mask-conic-to-color",
    "--tw-mask-conic-to-position", "box-decoration-break", "background-size", "background-attachment",
    "background-clip", "background-position", "background-repeat", "background-origin", "mask-composite",
    "mask-mode", "mask-type", "mask-size", "mask-clip", "mask-position", "mask-repeat", "mask-origin", "fill",
    "stroke", "stroke-width", "object-fit", "object-position", "padding", "padding-inline", "padding-block",
    "padding-inline-start", "padding-inline-end", "padding-top", "padding-right", "padding-bottom",
    "padding-left", "text-align", "text-indent", "vertical-align", "font-family", "font-size", "line-height",
    "font-weight", "letter-spacing", "text-wrap", "overflow-wrap", "word-break", "text-overflow", "hyphens",
    "white-space", "color", "text-transform", "font-style", "font-stretch", "font-variant-numeric",
    "text-decoration-line", "text-decoration-color", "text-decoration-style", "text-decoration-thickness",
    "text-underline-offset", "-webkit-font-smoothing", "placeholder-color", "caret-color", "accent-color",
    "color-scheme", "opacity", "background-blend-mode", "mix-blend-mode", "box-shadow", "--tw-shadow",
    "--tw-shadow-color", "--tw-ring-shadow", "--tw-ring-color", "--tw-inset-shadow", "--tw-inset-shadow-color",
    "--tw-inset-ring-shadow", "--tw-inset-ring-color", "--tw-ring-offset-width", "--tw-ring-offset-color",
    "outline", "outline-width", "outline-offset", "outline-color", "--tw-blur", "--tw-brightness",
    "--tw-contrast", "--tw-drop-shadow", "--tw-grayscale", "--tw-hue-rotate", "--tw-invert", "--tw-saturate",
    "--tw-sepia", "filter", "--tw-backdrop-blur", "--tw-backdrop-brightness", "--tw-backdrop-contrast",
    "--tw-backdrop-grayscale", "--tw-backdrop-hue-rotate", "--tw-backdrop-invert", "--tw-backdrop-opacity",
    "--tw-backdrop-saturate", "--tw-backdrop-sepia", "backdrop-filter", "transition-property",
    "transition-behavior", "transition-delay", "transition-duration", "transition-timing-function",
    "will-change", "contain", "content", "forced-color-adjust",
];

/// Stelle einer Eigenschaft in `PROPERTY_ORDER`.
pub fn index(prop: &str) -> Option<usize> {
    PROPERTY_ORDER.iter().position(|p| *p == prop)
}

/// Klassennamen vergleichen, Zahlen als Zahlen (`gap-2` vor `gap-10`) – wie Tailwinds `compare`.
pub fn compare_names(a: &str, z: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (a, z) = (a.as_bytes(), z.as_bytes());
    for i in 0..a.len().min(z.len()) {
        let (ac, zc) = (a[i], z[i]);
        if ac.is_ascii_digit() && zc.is_ascii_digit() {
            let ae = i + a[i..].iter().take_while(|c| c.is_ascii_digit()).count();
            let ze = i + z[i..].iter().take_while(|c| c.is_ascii_digit()).count();
            let (an, zn) = (&a[i..ae], &z[i..ze]);
            let num = |s: &[u8]| std::str::from_utf8(s).ok().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
            match num(an).partial_cmp(&num(zn)) {
                Some(Ordering::Equal) | None => {}
                Some(o) => return o,
            }
            match an.cmp(zn) {
                Ordering::Equal => continue,
                o => return o,
            }
        }
        if ac != zc {
            return ac.cmp(&zc);
        }
    }
    a.len().cmp(&z.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    #[test]
    fn zahlen_als_zahlen() {
        assert_eq!(compare_names("gap-2", "gap-10"), Less);
        assert_eq!(compare_names("bg-red-500", "bg-red-500/50"), Less);
        assert_eq!(compare_names("a", "a"), Equal);
        assert!(index("padding-left").unwrap() > index("padding").unwrap());
    }
}
