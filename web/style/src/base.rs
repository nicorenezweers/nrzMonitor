//! Feste Stilregeln: Grundstile (Preflight von Tailwind CSS v4.1, MIT-Lizenz, siehe
//! `LICENSE-tailwindcss`), nrz-Grundstile, Animationen und registrierte Variablen.

use crate::css::S::{self, A, R};

/// Einheitlicher Ausgangspunkt aller Browser (Tailwind „Preflight“).
pub const PREFLIGHT: &[S] = &[
    R("*,::after,::before,::backdrop,::file-selector-button", &[
        ("box-sizing", "border-box"),
        ("margin", "0"),
        ("padding", "0"),
        ("border", "0 solid"),
    ]),
    R("html,:host", &[
        ("line-height", "1.5"),
        ("-webkit-text-size-adjust", "100%"),
        ("tab-size", "4"),
        ("font-family", "var(--default-font-family, ui-sans-serif, system-ui, sans-serif, 'Apple Color Emoji', 'Segoe UI Emoji', 'Segoe UI Symbol', 'Noto Color Emoji')"),
        ("font-feature-settings", "var(--default-font-feature-settings, normal)"),
        ("font-variation-settings", "var(--default-font-variation-settings, normal)"),
        ("-webkit-tap-highlight-color", "transparent"),
    ]),
    R("hr", &[("height", "0"), ("color", "inherit"), ("border-top-width", "1px")]),
    R("abbr:where([title])", &[("-webkit-text-decoration", "underline dotted"), ("text-decoration", "underline dotted")]),
    R("h1,h2,h3,h4,h5,h6", &[("font-size", "inherit"), ("font-weight", "inherit")]),
    R("a", &[("color", "inherit"), ("-webkit-text-decoration", "inherit"), ("text-decoration", "inherit")]),
    R("b,strong", &[("font-weight", "bolder")]),
    R("code,kbd,samp,pre", &[
        ("font-family", "var(--default-mono-font-family, ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace)"),
        ("font-feature-settings", "var(--default-mono-font-feature-settings, normal)"),
        ("font-variation-settings", "var(--default-mono-font-variation-settings, normal)"),
        ("font-size", "1em"),
    ]),
    R("small", &[("font-size", "80%")]),
    R("sub,sup", &[("font-size", "75%"), ("line-height", "0"), ("position", "relative"), ("vertical-align", "baseline")]),
    R("sub", &[("bottom", "-0.25em")]),
    R("sup", &[("top", "-0.5em")]),
    R("table", &[("text-indent", "0"), ("border-color", "inherit"), ("border-collapse", "collapse")]),
    R(":-moz-focusring", &[("outline", "auto")]),
    R("progress", &[("vertical-align", "baseline")]),
    R("summary", &[("display", "list-item")]),
    R("ol,ul,menu", &[("list-style", "none")]),
    R("img,svg,video,canvas,audio,iframe,embed,object", &[("display", "block"), ("vertical-align", "middle")]),
    R("img,video", &[("max-width", "100%"), ("height", "auto")]),
    R("button,input,select,optgroup,textarea,::file-selector-button", &[
        ("font", "inherit"),
        ("font-feature-settings", "inherit"),
        ("font-variation-settings", "inherit"),
        ("letter-spacing", "inherit"),
        ("color", "inherit"),
        ("border-radius", "0"),
        ("background-color", "transparent"),
        ("opacity", "1"),
    ]),
    R(":where(select:is([multiple], [size])) optgroup", &[("font-weight", "bolder")]),
    R(":where(select:is([multiple], [size])) optgroup option", &[("padding-inline-start", "20px")]),
    R("::file-selector-button", &[("margin-inline-end", "4px")]),
    R("::placeholder", &[("opacity", "1")]),
    A("@supports (not (-webkit-appearance: -apple-pay-button)) or (contain-intrinsic-size: 1px)", &[
        R("::placeholder", &[("color", "color-mix(in oklab, currentcolor 50%, transparent)")]),
    ]),
    R("textarea", &[("resize", "vertical")]),
    R("::-webkit-search-decoration", &[("-webkit-appearance", "none")]),
    R("::-webkit-date-and-time-value", &[("min-height", "1lh"), ("text-align", "inherit")]),
    R("::-webkit-datetime-edit", &[("display", "inline-flex")]),
    R("::-webkit-datetime-edit-fields-wrapper", &[("padding", "0")]),
    R("::-webkit-datetime-edit,::-webkit-datetime-edit-year-field,::-webkit-datetime-edit-month-field,::-webkit-datetime-edit-day-field,::-webkit-datetime-edit-hour-field,::-webkit-datetime-edit-minute-field,::-webkit-datetime-edit-second-field,::-webkit-datetime-edit-millisecond-field,::-webkit-datetime-edit-meridiem-field", &[("padding-block", "0")]),
    R(":-moz-ui-invalid", &[("box-shadow", "none")]),
    R("button,input:where([type='button'], [type='reset'], [type='submit']),::file-selector-button", &[("appearance", "button")]),
    R("::-webkit-inner-spin-button,::-webkit-outer-spin-button", &[("height", "auto")]),
    R("[hidden]:where(:not([hidden='until-found']))", &[("display", "none !important")]),
];

/// Grundstile des nrz-Designs (Rahmenfarbe, Schrift, Auswahl, schmale runde Scrollbalken).
pub const NRZ_BASE: &[S] = &[
    R("*", &[("border-color", "var(--border)"), ("outline-color", "color-mix(in oklab, var(--ring) 50%, transparent)")]),
    R("html,body", &[("height", "100%")]),
    R("body", &[
        ("background-color", "var(--background)"),
        ("color", "var(--foreground)"),
        ("-webkit-font-smoothing", "antialiased"),
        ("-moz-osx-font-smoothing", "grayscale"),
        ("font-family", "var(--font-sans)"),
        ("font-synthesis-weight", "none"),
        ("text-rendering", "optimizeLegibility"),
        ("overscroll-behavior", "none"),
        ("-webkit-tap-highlight-color", "transparent"),
    ]),
    R("::selection", &[("background", "var(--selection)"), ("color", "var(--foreground)")]),
    R("button:not(:disabled),[role=\"button\"]:not(:disabled)", &[("cursor", "pointer")]),
    R("dialog", &[("margin", "auto")]),
    R("*", &[("scrollbar-color", "var(--scrollbar) transparent")]),
    R("::-webkit-scrollbar", &[("width", "10px"), ("height", "10px")]),
    R("::-webkit-scrollbar-track", &[("background", "transparent")]),
    R("::-webkit-scrollbar-thumb", &[
        ("background", "var(--scrollbar)"),
        ("border-radius", "9999px"),
        ("border", "2px solid transparent"),
        ("background-clip", "padding-box"),
    ]),
    // Ladekreise auf eigener Ebene: drehen weiter, auch wenn der Hauptthread arbeitet
    R("[class*=\"animate-spin\"]", &[("will-change", "transform")]),
];

/// Bewegungen der nrz-Bausteine (Popover, Abdunkelung, Schublade, Meldungen, Lichtflecken).
pub const NRZ_KEYFRAMES: &[S] = &[
    A("@keyframes nrz-enter", &[
        R("from", &[("opacity", "0"), ("transform", "translateY(4px) scale(0.98)")]),
        R("to", &[("opacity", "1"), ("transform", "none")]),
    ]),
    A("@keyframes nrz-fade-in", &[R("from", &[("opacity", "0")])]),
    A("@keyframes nrz-sheet-in", &[
        R("from", &[("opacity", "0"), ("transform", "translateX(-16px)")]),
        R("to", &[("opacity", "1"), ("transform", "none")]),
    ]),
    A("@keyframes nrz-toast-in", &[
        R("from", &[("opacity", "0"), ("transform", "translateY(8px) scale(0.97)")]),
        R("to", &[("opacity", "1"), ("transform", "none")]),
    ]),
    A("@keyframes nrz-drift-a", &[
        R("0%, 100%", &[("transform", "translate3d(0, 0, 0) scale(1)")]),
        R("50%", &[("transform", "translate3d(40vw, 30vh, 0) scale(1.1)")]),
    ]),
    A("@keyframes nrz-drift-b", &[
        R("0%, 100%", &[("transform", "translate3d(0, 0, 0) scale(1)")]),
        R("50%", &[("transform", "translate3d(-40vw, 10vh, 0) scale(0.95)")]),
    ]),
    A("@media (prefers-reduced-motion: reduce)", &[
        R(".animate-nrz-enter,.animate-nrz-fade,.animate-nrz-sheet,.animate-nrz-toast", &[("animation-duration", "1ms")]),
        R(".nrz-drift>span", &[("animation-duration", "160s")]),
    ]),
];

/// Feste Regeln der nrzUI-Bausteine (Befehlspalette, Untermenüs, Chips).
pub const NRZ_UI: &[S] = &[
    R(
        "[data-name=\"CommandItemLink\"][aria-selected=\"true\"],[data-name=\"CommandItem\"][aria-selected=\"true\"]",
        &[("background-color", "var(--muted)")],
    ),
    R(
        "[data-name=\"CommandList\"]:has([data-name=\"CommandItem\"][style*=\"flex\"]) [data-name=\"CommandEmpty\"]",
        &[("display", "none")],
    ),
    R(".dropdown__menu_sub_content", &[
        ("position", "absolute"),
        ("inset-inline-start", "calc(100% + 8px)"),
        ("inset-block-start", "-4px"),
        ("z-index", "100"),
        ("min-inline-size", "160px"),
        ("opacity", "0"),
        ("visibility", "hidden"),
        ("transform", "translateX(-8px)"),
        ("transition", "all 0.2s ease-out"),
        ("pointer-events", "none"),
    ]),
    R(".dropdown__menu_sub_trigger:hover .dropdown__menu_sub_content", &[
        ("opacity", "1"),
        ("visibility", "visible"),
        ("transform", "translateX(0)"),
        ("pointer-events", "auto"),
    ]),
    R("[data-name=\"ChipsContainer\"] label::after", &[
        ("content", "\"\""),
        (
            "background",
            "linear-gradient(230deg, var(--warning) 0 0.96vmin, #fff0 0 100%), linear-gradient(142deg, var(--warning) 0 1.12vmin, #fff0 0 100%), conic-gradient(from 43deg at 43% calc(64% + 0.24vmin), #fff0 0 0%, var(--warning) 1% 76%, #fff0 77% 100%), conic-gradient(from -45deg at 43% 64%, #fff0 0 0%, var(--warning) 2% 25%, #fff0 26% 100%)",
        ),
    ]),
];

/// Diagramme: Linien behalten ihre Breite, auch wenn die Zeichenfläche in die Breite gezogen wird.
pub const NRZ_PLOT: &[S] = &[R(".nrz-plot path,.nrz-plot line", &[("vector-effect", "non-scaling-stroke")])];

/// Animationen aus dem Tailwind-Thema (nur ausgegeben, wenn `animate-…` benutzt wird).
pub const KEYFRAMES: &[(&str, S)] = &[
    ("--animate-spin", A("@keyframes spin", &[R("to", &[("transform", "rotate(360deg)")])])),
    ("--animate-ping", A("@keyframes ping", &[R("75%, 100%", &[("transform", "scale(2)"), ("opacity", "0")])])),
    ("--animate-pulse", A("@keyframes pulse", &[R("50%", &[("opacity", "0.5")])])),
    ("--animate-bounce", A("@keyframes bounce", &[
        R("0%, 100%", &[("transform", "translateY(-25%)"), ("animation-timing-function", "cubic-bezier(0.8, 0, 1, 1)")]),
        R("50%", &[("transform", "none"), ("animation-timing-function", "cubic-bezier(0, 0, 0.2, 1)")]),
    ])),
];

/// Registrierte Variablen (`@property`): Name, Syntax, Anfangswert.
pub const PROPERTIES: &[(&str, &str, Option<&str>)] = &[
    ("--tw-translate-x", "*", Some("0")),
    ("--tw-translate-y", "*", Some("0")),
    ("--tw-translate-z", "*", Some("0")),
    ("--tw-rotate-x", "*", None),
    ("--tw-rotate-y", "*", None),
    ("--tw-rotate-z", "*", None),
    ("--tw-skew-x", "*", None),
    ("--tw-skew-y", "*", None),
    ("--tw-pan-x", "*", None),
    ("--tw-pan-y", "*", None),
    ("--tw-pinch-zoom", "*", None),
    ("--tw-scroll-snap-strictness", "*", Some("proximity")),
    ("--tw-divide-y-reverse", "*", Some("0")),
    ("--tw-border-style", "*", Some("solid")),
    ("--tw-leading", "*", None),
    ("--tw-font-weight", "*", None),
    ("--tw-tracking", "*", None),
    ("--tw-ordinal", "*", None),
    ("--tw-slashed-zero", "*", None),
    ("--tw-numeric-figure", "*", None),
    ("--tw-numeric-spacing", "*", None),
    ("--tw-numeric-fraction", "*", None),
    ("--tw-shadow", "*", Some("0 0 #0000")),
    ("--tw-shadow-color", "*", None),
    ("--tw-shadow-alpha", "<percentage>", Some("100%")),
    ("--tw-inset-shadow", "*", Some("0 0 #0000")),
    ("--tw-inset-shadow-color", "*", None),
    ("--tw-inset-shadow-alpha", "<percentage>", Some("100%")),
    ("--tw-ring-color", "*", None),
    ("--tw-ring-shadow", "*", Some("0 0 #0000")),
    ("--tw-inset-ring-color", "*", None),
    ("--tw-inset-ring-shadow", "*", Some("0 0 #0000")),
    ("--tw-ring-inset", "*", None),
    ("--tw-ring-offset-width", "<length>", Some("0px")),
    ("--tw-ring-offset-color", "*", Some("#fff")),
    ("--tw-ring-offset-shadow", "*", Some("0 0 #0000")),
    ("--tw-blur", "*", None),
    ("--tw-brightness", "*", None),
    ("--tw-contrast", "*", None),
    ("--tw-grayscale", "*", None),
    ("--tw-hue-rotate", "*", None),
    ("--tw-invert", "*", None),
    ("--tw-opacity", "*", None),
    ("--tw-saturate", "*", None),
    ("--tw-sepia", "*", None),
    ("--tw-drop-shadow", "*", None),
    ("--tw-drop-shadow-color", "*", None),
    ("--tw-drop-shadow-alpha", "<percentage>", Some("100%")),
    ("--tw-drop-shadow-size", "*", None),
    ("--tw-backdrop-blur", "*", None),
    ("--tw-backdrop-brightness", "*", None),
    ("--tw-backdrop-contrast", "*", None),
    ("--tw-backdrop-grayscale", "*", None),
    ("--tw-backdrop-hue-rotate", "*", None),
    ("--tw-backdrop-invert", "*", None),
    ("--tw-backdrop-opacity", "*", None),
    ("--tw-backdrop-saturate", "*", None),
    ("--tw-backdrop-sepia", "*", None),
    ("--tw-duration", "*", None),
    ("--tw-ease", "*", None),
    ("--tw-content", "*", Some("\"\"")),
    ("--tw-outline-style", "*", Some("solid")),
    ("--tw-scale-x", "*", Some("1")),
    ("--tw-scale-y", "*", Some("1")),
    ("--tw-scale-z", "*", Some("1")),
    ("--tw-space-x-reverse", "*", Some("0")),
    ("--tw-space-y-reverse", "*", Some("0")),
    ("--tw-divide-x-reverse", "*", Some("0")),
];
