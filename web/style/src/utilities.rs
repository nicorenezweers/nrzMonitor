//! Klassen (Utilities): `flex`, `gap-4`, `bg-card/50`, `text-sm`, `rounded-xl`, `shadow-sm` …
//! Bedeutung und Schreibweise wie bei Tailwind CSS v4.1 (MIT-Lizenz, siehe `LICENSE-tailwindcss`),
//! damit nrzUI-Bausteine und Seiten unverändert bleiben. Fehlt eine Klasse, hier ergänzen.

use crate::css::{Node, at, d, rule, split_top};
use crate::theme::{self, raw, resolve};
use crate::value::{
    decode_arbitrary, infer, js_number, replace_shadow_colors, substitute_theme, valid_arbitrary, with_alpha,
};

#[derive(Clone, Debug)]
enum Value {
    Named { value: String, fraction: Option<String> },
    Arbitrary { value: String, hint: Option<String> },
}

#[derive(Clone, Debug)]
enum Modifier {
    Named(String),
    Arbitrary(String),
}

struct C<'a> {
    root: &'a str,
    value: Option<Value>,
    modifier: Option<Modifier>,
    negative: bool,
}

impl C<'_> {
    fn named(&self) -> Option<&str> {
        match &self.value {
            Some(Value::Named { value, .. }) => Some(value),
            _ => None,
        }
    }
    fn fraction(&self) -> Option<&str> {
        match &self.value {
            Some(Value::Named { fraction, .. }) => fraction.as_deref(),
            _ => None,
        }
    }
    fn arbitrary(&self) -> Option<(&str, Option<&str>)> {
        match &self.value {
            Some(Value::Arbitrary { value, hint }) => Some((value, hint.as_deref())),
            _ => None,
        }
    }
}

/// CSS-Inhalt einer Klasse ohne Varianten (`None`: keine bekannte Klasse).
pub fn compile(base: &str) -> Option<Vec<Node>> {
    let (base, important) = match base.strip_suffix('!').or_else(|| base.strip_prefix('!')) {
        Some(b) => (b, true),
        None => (base, false),
    };
    let mut nodes = compile_base(base)?;
    if important {
        make_important(&mut nodes);
    }
    Some(nodes)
}

fn make_important(nodes: &mut [Node]) {
    for n in nodes {
        match n {
            Node::Decl(p, v) if p != "--tw-sort" => v.push_str(" !important"),
            Node::Rule(_, kids) | Node::At(_, kids) => make_important(kids),
            Node::Decl(..) => {}
        }
    }
}

fn compile_base(base: &str) -> Option<Vec<Node>> {
    if base.starts_with('[') {
        return arbitrary_property(base);
    }
    let (negative, body) = match base.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, base),
    };
    let segs = split_top(body, '/');
    if segs.len() > 2 {
        return None;
    }
    let main = segs[0];
    let modifier_raw = segs.get(1).copied();
    let modifier = match modifier_raw {
        None => None,
        Some(m) => Some(parse_modifier(m)?),
    };
    if main.is_empty() {
        return None;
    }
    if !negative && modifier.is_none()
        && let Some(n) = static_utility(main) {
            return Some(n);
        }
    if let Some(n) = functional(&C { root: main, value: None, modifier: modifier.clone(), negative }) {
        return Some(n);
    }
    let mut end = main.len();
    while let Some(i) = main[..end].rfind('-') {
        if i == 0 {
            break;
        }
        let rest = &main[i + 1..];
        if rest.is_empty() {
            break;
        }
        match parse_value(rest, modifier_raw, &modifier) {
            Some(value) => {
                let c = C { root: &main[..i], value: Some(value), modifier: modifier.clone(), negative };
                if let Some(n) = functional(&c) {
                    return Some(n);
                }
            }
            None if rest.contains('[') => return None,
            None => {}
        }
        end = i;
    }
    None
}

fn parse_value(rest: &str, modifier_raw: Option<&str>, modifier: &Option<Modifier>) -> Option<Value> {
    if rest.contains('[') {
        if !rest.starts_with('[') || !rest.ends_with(']') {
            return None;
        }
        let mut v = decode_arbitrary(&rest[1..rest.len() - 1]);
        if !valid_arbitrary(&v) {
            return None;
        }
        let mut hint = None;
        if let Some(colon) = v.find(':')
            && colon > 0 && v[..colon].chars().all(|c| c == '-' || c.is_ascii_lowercase()) {
                hint = Some(v[..colon].to_string());
                v = v[colon + 1..].to_string();
            }
        if v.trim().is_empty() {
            return None;
        }
        return Some(Value::Arbitrary { value: v, hint });
    }
    if rest.starts_with('(') && rest.ends_with(')') {
        let inner = &rest[1..rest.len() - 1];
        let (hint, name) = match inner.find(':') {
            Some(i) if !inner.starts_with("--") => (Some(inner[..i].to_string()), &inner[i + 1..]),
            _ => (None, inner),
        };
        if !name.starts_with("--") {
            return None;
        }
        return Some(Value::Arbitrary { value: format!("var({name})"), hint });
    }
    let fraction = match (modifier_raw, modifier) {
        (Some(m), Some(Modifier::Named(_))) => Some(format!("{rest}/{m}")),
        _ => None,
    };
    Some(Value::Named { value: rest.to_string(), fraction })
}

fn parse_modifier(m: &str) -> Option<Modifier> {
    if m.starts_with('[') && m.ends_with(']') {
        let v = decode_arbitrary(&m[1..m.len() - 1]);
        return valid_arbitrary(&v).then_some(Modifier::Arbitrary(v));
    }
    if m.starts_with('(') && m.ends_with(')') {
        let inner = &m[1..m.len() - 1];
        return inner.starts_with("--").then(|| Modifier::Arbitrary(format!("var({inner})")));
    }
    (!m.is_empty()).then(|| Modifier::Named(m.to_string()))
}

/// `[eigenschaft:wert]`
fn arbitrary_property(base: &str) -> Option<Vec<Node>> {
    if !base.ends_with(']') || base.len() < 3 {
        return None;
    }
    let c1 = base.as_bytes()[1];
    if c1 != b'-' && !c1.is_ascii_lowercase() {
        return None;
    }
    let inner = &base[1..base.len() - 1];
    let idx = inner.find(':')?;
    if idx == 0 || idx == inner.len() - 1 {
        return None;
    }
    let prop = &inner[..idx];
    if !prop.chars().all(|c| c == '-' || c.is_ascii_alphanumeric()) {
        return None;
    }
    let value = decode_arbitrary(&inner[idx + 1..]);
    (valid_arbitrary(&value) && !value.starts_with(':')).then(|| vec![d(prop, value)])
}

// --- Zahlen ------------------------------------------------------------------------------------

fn num(v: &str) -> Option<f64> {
    v.parse::<f64>().ok().filter(|n| n.is_finite() && js_number(*n) == v)
}

fn is_positive_integer(v: &str) -> bool {
    num(v).is_some_and(|n| n >= 0.0 && n.fract() == 0.0)
}

fn is_strict_positive_integer(v: &str) -> bool {
    num(v).is_some_and(|n| n > 0.0 && n.fract() == 0.0)
}

fn is_multiple_of_quarter(v: &str) -> bool {
    num(v).is_some_and(|n| n >= 0.0 && n % 0.25 == 0.0)
}

fn is_fraction(f: &str) -> bool {
    let parts: Vec<&str> = f.split('/').collect();
    parts.len() == 2 && is_positive_integer(parts[0]) && is_positive_integer(parts[1])
}

// --- Bausteine für funktionale Klassen ------------------------------------------------------------

/// Blosser Wert (`4` in `gap-4`) → CSS-Wert.
type Bare<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Wie Tailwinds `functionalUtility`: Wert aus Thema, `[…]`, Bruch oder blossem Wert ermitteln.
struct Spec<'a> {
    theme: &'a [&'a str],
    default: Option<&'a str>,
    negative: bool,
    fractions: bool,
    bare: Option<Bare<'a>>,
    negative_bare: Option<Bare<'a>>,
    statics: &'a [(&'a str, &'a str)],
}

const SPEC: Spec<'static> =
    Spec { theme: &[], default: None, negative: false, fractions: false, bare: None, negative_bare: None, statics: &[] };

fn value_of(c: &C, s: &Spec) -> Option<(String, Option<String>)> {
    if c.negative && !s.negative {
        return None;
    }
    let neg = |v: String| if c.negative { format!("calc({v} * -1)") } else { v };
    match &c.value {
        None => {
            if c.modifier.is_some() {
                return None;
            }
            let v = match s.default {
                Some(d) => Some(d.to_string()),
                None => resolve(None, s.theme),
            }?;
            Some((neg(v), None))
        }
        Some(Value::Arbitrary { value, hint }) => {
            if c.modifier.is_some() {
                return None;
            }
            Some((neg(value.clone()), hint.clone()))
        }
        Some(Value::Named { value, fraction }) => {
            let key = fraction.as_deref().unwrap_or(value);
            let mut v = resolve(Some(key), s.theme);
            if v.is_none() && s.fractions
                && let Some(f) = fraction {
                    if !is_fraction(f) {
                        return None;
                    }
                    v = Some(format!("calc({f} * 100%)"));
                }
            if v.is_none() && c.negative
                && let Some(nb) = s.negative_bare
                    && let Some(x) = nb(value) {
                        if !x.contains('/') && c.modifier.is_some() {
                            return None;
                        }
                        return Some((x, None));
                    }
            if v.is_none()
                && let Some(b) = s.bare {
                    v = b(value);
                    if v.as_deref().is_some_and(|x| !x.contains('/')) && c.modifier.is_some() {
                        return None;
                    }
                }
            if v.is_none() && !c.negative && c.modifier.is_none()
                && let Some((_, x)) = s.statics.iter().find(|(k, _)| k == value) {
                    return Some((x.to_string(), None));
                }
            Some((neg(v?), None))
        }
    }
}

fn spacing_bare(v: &str) -> Option<String> {
    is_multiple_of_quarter(v).then(|| format!("calc(var(--spacing) * {v})"))
}

fn spacing_negative_bare(v: &str) -> Option<String> {
    is_multiple_of_quarter(v).then(|| format!("calc(var(--spacing) * -{v})"))
}

fn integer_bare(v: &str) -> Option<String> {
    is_positive_integer(v).then(|| v.to_string())
}

fn px_bare(v: &str) -> Option<String> {
    is_positive_integer(v).then(|| format!("{v}px"))
}

fn percent_bare(v: &str) -> Option<String> {
    is_positive_integer(v).then(|| format!("{v}%"))
}

/// Abstände und Grössen (`p-4`, `-mt-1`, `w-1/2`, `inset-x-0` …) wie Tailwinds `spacingUtility`.
fn spacing(c: &C, theme: &[&str], negative: bool, fractions: bool) -> Option<String> {
    if c.named() == Some("px") && c.modifier.is_none() {
        if c.negative && !negative {
            return None;
        }
        return Some(if c.negative { "-1px".into() } else { "1px".into() });
    }
    let spec = Spec {
        theme,
        negative,
        fractions,
        bare: Some(&spacing_bare),
        negative_bare: Some(&spacing_negative_bare),
        ..SPEC
    };
    c.value.as_ref()?;
    value_of(c, &spec).map(|(v, _)| v)
}

/// Farbe aus Thema oder `[…]`, mit Deckkraft aus dem Modifikator (`bg-card/50`).
fn color(c: &C, namespaces: &[&str]) -> Option<String> {
    if c.negative {
        return None;
    }
    let base = match &c.value {
        Some(Value::Named { value, .. }) => match value.as_str() {
            "transparent" => "transparent".to_string(),
            "current" => "currentcolor".to_string(),
            "inherit" => "inherit".to_string(),
            v => resolve(Some(v), namespaces)?,
        },
        Some(Value::Arbitrary { value, .. }) => substitute_theme(value),
        None => return None,
    };
    as_color(&base, &c.modifier)
}

fn as_color(value: &str, modifier: &Option<Modifier>) -> Option<String> {
    match modifier {
        None => Some(value.to_string()),
        Some(Modifier::Arbitrary(a)) => Some(with_alpha(value, a)),
        Some(Modifier::Named(m)) => {
            if let Some(alpha) = resolve(Some(m), &["--opacity"]) {
                return Some(with_alpha(value, &alpha));
            }
            is_multiple_of_quarter(m).then(|| with_alpha(value, &format!("{m}%")))
        }
    }
}

/// Typ eines `[…]`-Werts: Hinweis (`[length:…]`) oder erkannt.
fn arbitrary_type(c: &C, types: &[&str]) -> Option<String> {
    let (v, hint) = c.arbitrary()?;
    Some(hint.map(str::to_string).or_else(|| infer(v, types).map(str::to_string)).unwrap_or_default())
}

const SHADOW: &str = "var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow)";
const FILTER: &str = "var(--tw-blur,) var(--tw-brightness,) var(--tw-contrast,) var(--tw-grayscale,) var(--tw-hue-rotate,) var(--tw-invert,) var(--tw-saturate,) var(--tw-sepia,) var(--tw-drop-shadow,)";
const BACKDROP: &str = "var(--tw-backdrop-blur,) var(--tw-backdrop-brightness,) var(--tw-backdrop-contrast,) var(--tw-backdrop-grayscale,) var(--tw-backdrop-hue-rotate,) var(--tw-backdrop-invert,) var(--tw-backdrop-opacity,) var(--tw-backdrop-saturate,) var(--tw-backdrop-sepia,)";
const TRANSLATE: &str = "var(--tw-translate-x) var(--tw-translate-y)";
const EASE: &str = "var(--tw-ease, var(--default-transition-timing-function))";
const DURATION: &str = "var(--tw-duration, var(--default-transition-duration))";
const TRANSITION_DEFAULT: &str = "color, background-color, border-color, outline-color, text-decoration-color, fill, stroke, --tw-gradient-from, --tw-gradient-via, --tw-gradient-to, opacity, box-shadow, transform, translate, scale, rotate, filter, -webkit-backdrop-filter, backdrop-filter, display, visibility, content-visibility, overlay, pointer-events";
const TRANSITION_COLORS: &str = "color, background-color, border-color, outline-color, text-decoration-color, fill, stroke, --tw-gradient-from, --tw-gradient-via, --tw-gradient-to";

fn shadow_value(value: &str) -> String {
    replace_shadow_colors(value, &|c| format!("var(--tw-shadow-color, {c})"))
}

fn ring_shadow(width: &str) -> String {
    format!("var(--tw-ring-inset,) 0 0 0 calc({width} + var(--tw-ring-offset-width)) var(--tw-ring-color, currentcolor)")
}

const RING_OFFSET_SHADOW: &str = "var(--tw-ring-inset,) 0 0 0 var(--tw-ring-offset-width) var(--tw-ring-offset-color)";

fn border_width(props: &[&str], width: String) -> Vec<Node> {
    let mut out = Vec::new();
    for p in props {
        out.push(d(&format!("{p}-style"), "var(--tw-border-style)"));
        out.push(d(&format!("{p}-width"), width.clone()));
    }
    // Tailwind gibt erst alle Stile, dann alle Breiten aus
    let (mut styles, mut widths): (Vec<Node>, Vec<Node>) = (Vec::new(), Vec::new());
    for n in out {
        match &n {
            Node::Decl(p, _) if p.ends_with("-style") => styles.push(n),
            _ => widths.push(n),
        }
    }
    styles.extend(widths);
    styles
}

/// Seiten der Rahmen-Klassen (`border-x`, `border-t` …) und Ecken der Radius-Klassen.
fn border_props(root: &str) -> Option<&'static [&'static str]> {
    Some(match root {
        "border" => &["border"],
        "border-x" => &["border-inline"],
        "border-y" => &["border-block"],
        "border-s" => &["border-inline-start"],
        "border-e" => &["border-inline-end"],
        "border-t" => &["border-top"],
        "border-r" => &["border-right"],
        "border-b" => &["border-bottom"],
        "border-l" => &["border-left"],
        _ => return None,
    })
}

fn radius_props(root: &str) -> Option<&'static [&'static str]> {
    Some(match root {
        "rounded" => &["border-radius"],
        "rounded-s" => &["border-start-start-radius", "border-end-start-radius"],
        "rounded-e" => &["border-start-end-radius", "border-end-end-radius"],
        "rounded-t" => &["border-top-left-radius", "border-top-right-radius"],
        "rounded-r" => &["border-top-right-radius", "border-bottom-right-radius"],
        "rounded-b" => &["border-bottom-right-radius", "border-bottom-left-radius"],
        "rounded-l" => &["border-top-left-radius", "border-bottom-left-radius"],
        "rounded-ss" => &["border-start-start-radius"],
        "rounded-se" => &["border-start-end-radius"],
        "rounded-ee" => &["border-end-end-radius"],
        "rounded-es" => &["border-end-start-radius"],
        "rounded-tl" => &["border-top-left-radius"],
        "rounded-tr" => &["border-top-right-radius"],
        "rounded-br" => &["border-bottom-right-radius"],
        "rounded-bl" => &["border-bottom-left-radius"],
        _ => return None,
    })
}

fn all(props: &[&str], v: &str) -> Vec<Node> {
    props.iter().map(|p| d(p, v)).collect()
}

const SIZE_KEYWORDS: &[(&str, &str)] = &[
    ("full", "100%"),
    ("svw", "100svw"),
    ("lvw", "100lvw"),
    ("dvw", "100dvw"),
    ("svh", "100svh"),
    ("lvh", "100lvh"),
    ("dvh", "100dvh"),
    ("min", "min-content"),
    ("max", "max-content"),
    ("fit", "fit-content"),
];

/// Funktionale Klassen: Wurzel + Wert (`gap-4`, `bg-card`, `rounded-xl` …).
fn functional(c: &C) -> Option<Vec<Node>> {
    let root = c.root;

    // Abstände
    let spacing_prop: Option<(&str, &[&str], bool)> = match root {
        "m" => Some(("margin", &["--margin", "--spacing"], true)),
        "mx" => Some(("margin-inline", &["--margin", "--spacing"], true)),
        "my" => Some(("margin-block", &["--margin", "--spacing"], true)),
        "ms" => Some(("margin-inline-start", &["--margin", "--spacing"], true)),
        "me" => Some(("margin-inline-end", &["--margin", "--spacing"], true)),
        "mt" => Some(("margin-top", &["--margin", "--spacing"], true)),
        "mr" => Some(("margin-right", &["--margin", "--spacing"], true)),
        "mb" => Some(("margin-bottom", &["--margin", "--spacing"], true)),
        "ml" => Some(("margin-left", &["--margin", "--spacing"], true)),
        "p" => Some(("padding", &["--padding", "--spacing"], false)),
        "px" => Some(("padding-inline", &["--padding", "--spacing"], false)),
        "py" => Some(("padding-block", &["--padding", "--spacing"], false)),
        "ps" => Some(("padding-inline-start", &["--padding", "--spacing"], false)),
        "pe" => Some(("padding-inline-end", &["--padding", "--spacing"], false)),
        "pt" => Some(("padding-top", &["--padding", "--spacing"], false)),
        "pr" => Some(("padding-right", &["--padding", "--spacing"], false)),
        "pb" => Some(("padding-bottom", &["--padding", "--spacing"], false)),
        "pl" => Some(("padding-left", &["--padding", "--spacing"], false)),
        "gap" => Some(("gap", &["--gap", "--spacing"], false)),
        "gap-x" => Some(("column-gap", &["--gap", "--spacing"], false)),
        "gap-y" => Some(("row-gap", &["--gap", "--spacing"], false)),
        "scroll-m" => Some(("scroll-margin", &["--scroll-margin", "--spacing"], true)),
        "scroll-mx" => Some(("scroll-margin-inline", &["--scroll-margin", "--spacing"], true)),
        "scroll-my" => Some(("scroll-margin-block", &["--scroll-margin", "--spacing"], true)),
        "scroll-mt" => Some(("scroll-margin-top", &["--scroll-margin", "--spacing"], true)),
        "scroll-mr" => Some(("scroll-margin-right", &["--scroll-margin", "--spacing"], true)),
        "scroll-mb" => Some(("scroll-margin-bottom", &["--scroll-margin", "--spacing"], true)),
        "scroll-ml" => Some(("scroll-margin-left", &["--scroll-margin", "--spacing"], true)),
        "scroll-p" => Some(("scroll-padding", &["--scroll-padding", "--spacing"], false)),
        "scroll-px" => Some(("scroll-padding-inline", &["--scroll-padding", "--spacing"], false)),
        "scroll-py" => Some(("scroll-padding-block", &["--scroll-padding", "--spacing"], false)),
        "scroll-pt" => Some(("scroll-padding-top", &["--scroll-padding", "--spacing"], false)),
        "scroll-pr" => Some(("scroll-padding-right", &["--scroll-padding", "--spacing"], false)),
        "scroll-pb" => Some(("scroll-padding-bottom", &["--scroll-padding", "--spacing"], false)),
        "scroll-pl" => Some(("scroll-padding-left", &["--scroll-padding", "--spacing"], false)),
        _ => None,
    };
    if let Some((prop, theme, negative)) = spacing_prop {
        if c.named() == Some("auto") && root.starts_with('m') && !c.negative && c.modifier.is_none() {
            return Some(vec![d(prop, "auto")]);
        }
        return spacing(c, theme, negative, false).map(|v| vec![d(prop, v)]);
    }

    // Position
    let inset_prop = match root {
        "inset" => Some("inset"),
        "inset-x" => Some("inset-inline"),
        "inset-y" => Some("inset-block"),
        "start" => Some("inset-inline-start"),
        "end" => Some("inset-inline-end"),
        "top" => Some("top"),
        "right" => Some("right"),
        "bottom" => Some("bottom"),
        "left" => Some("left"),
        _ => None,
    };
    if let Some(prop) = inset_prop {
        if c.modifier.is_none() {
            match (c.named(), c.negative) {
                (Some("auto"), false) => return Some(vec![d(prop, "auto")]),
                (Some("full"), false) => return Some(vec![d(prop, "100%")]),
                (Some("full"), true) => return Some(vec![d(prop, "-100%")]),
                _ => {}
            }
        }
        return spacing(c, &["--inset", "--spacing"], true, true).map(|v| vec![d(prop, v)]);
    }

    // Grössen
    let size_prop: Option<(&[&str], &[&str])> = match root {
        "size" => Some((&["width", "height"], &["--size", "--spacing"])),
        "w" => Some((&["width"], &["--width", "--spacing", "--container"])),
        "min-w" => Some((&["min-width"], &["--min-width", "--spacing", "--container"])),
        "max-w" => Some((&["max-width"], &["--max-width", "--spacing", "--container"])),
        "h" => Some((&["height"], &["--height", "--spacing"])),
        "min-h" => Some((&["min-height"], &["--min-height", "--height", "--spacing"])),
        "max-h" => Some((&["max-height"], &["--max-height", "--height", "--spacing"])),
        _ => None,
    };
    if let Some((props, theme)) = size_prop {
        let sort = || if root == "size" { vec![d("--tw-sort", "size")] } else { vec![] };
        if !c.negative && c.modifier.is_none()
            && let Some(n) = c.named() {
                let kw = SIZE_KEYWORDS.iter().find(|(k, _)| *k == n).map(|(_, v)| *v).or(match (n, root) {
                    ("auto", "size" | "w" | "h" | "min-w" | "min-h") => Some("auto"),
                    ("screen", "w" | "min-w" | "max-w") => Some("100vw"),
                    ("screen", "h" | "min-h" | "max-h") => Some("100vh"),
                    ("none", "max-w" | "max-h") => Some("none"),
                    _ => None,
                });
                if let Some(v) = kw {
                    let mut out = sort();
                    out.extend(all(props, v));
                    return Some(out);
                }
            }
        let v = spacing(c, theme, false, true)?;
        let mut out = sort();
        out.extend(all(props, &v));
        return Some(out);
    }

    match root {
        "basis" => {
            if !c.negative && c.modifier.is_none() {
                match c.named() {
                    Some("auto") => return Some(vec![d("flex-basis", "auto")]),
                    Some("full") => return Some(vec![d("flex-basis", "100%")]),
                    _ => {}
                }
            }
            spacing(c, &["--flex-basis", "--spacing", "--container"], false, true).map(|v| vec![d("flex-basis", v)])
        }
        "translate-x" | "translate-y" => {
            let var = if root == "translate-x" { "--tw-translate-x" } else { "--tw-translate-y" };
            let v = match (c.named(), c.negative, &c.modifier) {
                (Some("full"), false, None) => "100%".to_string(),
                (Some("full"), true, None) => "-100%".to_string(),
                _ => spacing(c, &["--translate", "--spacing"], true, true)?,
            };
            Some(vec![d(var, v), d("translate", TRANSLATE)])
        }
        "z" => {
            if c.named() == Some("auto") && !c.negative {
                return Some(vec![d("z-index", "auto")]);
            }
            let spec = Spec { theme: &["--z-index"], negative: true, bare: Some(&integer_bare), ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d("z-index", v)])
        }
        "order" => {
            let spec = Spec { theme: &["--order"], negative: true, bare: Some(&integer_bare), ..SPEC };
            match c.named() {
                Some("first") if !c.negative => Some(vec![d("order", "-infinity")]),
                Some("last") if !c.negative => Some(vec![d("order", "infinity")]),
                Some("none") if !c.negative => Some(vec![d("order", "0")]),
                _ => {
                    c.value.as_ref()?;
                    value_of(c, &spec).map(|(v, _)| vec![d("order", v)])
                }
            }
        }
        "col" | "row" => {
            let prop = if root == "col" { "grid-column" } else { "grid-row" };
            if c.named() == Some("auto") && !c.negative {
                return Some(vec![d(prop, "auto")]);
            }
            let spec = Spec { theme: &[if root == "col" { "--grid-column" } else { "--grid-row" }], negative: true, bare: Some(&integer_bare), ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d(prop, v)])
        }
        "col-span" | "row-span" => {
            let prop = if root == "col-span" { "grid-column" } else { "grid-row" };
            if c.named() == Some("full") && !c.negative {
                return Some(vec![d(prop, "1 / -1")]);
            }
            let spec = Spec { bare: Some(&integer_bare), ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d(prop, format!("span {v} / span {v}"))])
        }
        "col-start" | "col-end" | "row-start" | "row-end" => {
            let prop = match root {
                "col-start" => "grid-column-start",
                "col-end" => "grid-column-end",
                "row-start" => "grid-row-start",
                _ => "grid-row-end",
            };
            if c.named() == Some("auto") && !c.negative {
                return Some(vec![d(prop, "auto")]);
            }
            let theme_key = format!("--{prop}");
            let spec = Spec { theme: &[&theme_key], negative: true, bare: Some(&integer_bare), ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d(prop, v)])
        }
        "grid-cols" | "grid-rows" => {
            let prop = if root == "grid-cols" { "grid-template-columns" } else { "grid-template-rows" };
            match c.named() {
                Some("none") if !c.negative && c.modifier.is_none() => return Some(vec![d(prop, "none")]),
                Some("subgrid") if !c.negative && c.modifier.is_none() => return Some(vec![d(prop, "subgrid")]),
                _ => {}
            }
            let bare = |v: &str| is_strict_positive_integer(v).then(|| format!("repeat({v}, minmax(0, 1fr))"));
            let theme_key = format!("--{prop}");
            let spec = Spec { theme: &[&theme_key], bare: Some(&bare), ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d(prop, v)])
        }
        "auto-cols" | "auto-rows" => {
            let prop = if root == "auto-cols" { "grid-auto-columns" } else { "grid-auto-rows" };
            let kw = match c.named() {
                Some("auto") => Some("auto"),
                Some("min") => Some("min-content"),
                Some("max") => Some("max-content"),
                Some("fr") => Some("minmax(0, 1fr)"),
                _ => None,
            };
            if let (Some(v), false, None) = (kw, c.negative, &c.modifier) {
                return Some(vec![d(prop, v)]);
            }
            let (v, _) = c.arbitrary()?;
            (!c.negative && c.modifier.is_none()).then(|| vec![d(prop, v)])
        }
        "flex" => {
            if c.negative {
                return None;
            }
            if let Some((v, _)) = c.arbitrary() {
                return c.modifier.is_none().then(|| vec![d("flex", v)]);
            }
            if let Some(f) = c.fraction() {
                return is_fraction(f).then(|| vec![d("flex", format!("calc({f} * 100%)"))]);
            }
            let n = c.named()?;
            (is_positive_integer(n) && c.modifier.is_none()).then(|| vec![d("flex", n)])
        }
        "shrink" | "grow" => {
            let prop = if root == "shrink" { "flex-shrink" } else { "flex-grow" };
            let spec = Spec { default: Some("1"), bare: Some(&integer_bare), ..SPEC };
            value_of(c, &spec).map(|(v, _)| vec![d(prop, v)])
        }
        "rotate" => {
            let bare = |v: &str| is_positive_integer(v).then(|| format!("{v}deg"));
            let spec = Spec { theme: &["--rotate"], negative: true, bare: Some(&bare), ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d("rotate", v)])
        }
        "scale" => {
            if c.modifier.is_some() {
                return None;
            }
            let neg = |v: String| if c.negative { format!("calc({v} * -1)") } else { v };
            if let Some((v, _)) = c.arbitrary() {
                return Some(vec![d("scale", neg(v.to_string()))]);
            }
            let n = c.named()?;
            let v = resolve(Some(n), &["--scale"]).or_else(|| is_positive_integer(n).then(|| format!("{n}%")))?;
            let v = neg(v);
            Some(vec![
                d("--tw-scale-x", v.clone()),
                d("--tw-scale-y", v.clone()),
                d("--tw-scale-z", v),
                d("scale", "var(--tw-scale-x) var(--tw-scale-y)"),
            ])
        }
        "origin" => {
            let (v, _) = c.arbitrary()?;
            Some(vec![d("transform-origin", v)])
        }
        "opacity" => {
            let bare = |v: &str| is_multiple_of_quarter(v).then(|| format!("{v}%"));
            let spec = Spec { theme: &["--opacity"], bare: Some(&bare), ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d("opacity", v)])
        }
        "duration" => {
            if c.negative || c.modifier.is_some() {
                return None;
            }
            let v = match &c.value {
                Some(Value::Arbitrary { value, .. }) => value.clone(),
                Some(Value::Named { value, fraction }) => resolve(Some(fraction.as_deref().unwrap_or(value)), &["--transition-duration"])
                    .or_else(|| is_positive_integer(value).then(|| format!("{value}ms")))?,
                None => return None,
            };
            Some(vec![d("--tw-duration", v.clone()), d("transition-duration", v)])
        }
        "delay" => {
            let bare = |v: &str| is_positive_integer(v).then(|| format!("{v}ms"));
            let spec = Spec { theme: &["--transition-delay"], bare: Some(&bare), ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d("transition-delay", v)])
        }
        "ease" => {
            let spec = Spec { theme: &["--ease"], ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d("--tw-ease", v.clone()), d("transition-timing-function", v)])
        }
        "transition" => {
            let spec = Spec { theme: &["--transition-property"], default: Some(TRANSITION_DEFAULT), ..SPEC };
            value_of(c, &spec).map(|(v, _)| {
                vec![d("transition-property", v), d("transition-timing-function", EASE), d("transition-duration", DURATION)]
            })
        }
        "animate" => {
            let spec = Spec { theme: &["--animate"], ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d("animation", v)])
        }
        "cursor" => {
            let spec = Spec { theme: &["--cursor"], ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d("cursor", v)])
        }
        "text" => text(c),
        "font" => font(c),
        "leading" => {
            if c.named() == Some("none") && !c.negative && c.modifier.is_none() {
                return Some(vec![d("--tw-leading", "1"), d("line-height", "1")]);
            }
            let spec = Spec { theme: &["--leading"], bare: Some(&spacing_bare), ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d("--tw-leading", v.clone()), d("line-height", v)])
        }
        "tracking" => {
            let spec = Spec { theme: &["--tracking"], negative: true, ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d("--tw-tracking", v.clone()), d("letter-spacing", v)])
        }
        "underline-offset" => {
            if c.named() == Some("auto") && !c.negative {
                return Some(vec![d("text-underline-offset", "auto")]);
            }
            let spec = Spec { theme: &["--text-underline-offset"], negative: true, bare: Some(&px_bare), ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d("text-underline-offset", v)])
        }
        "line-clamp" => {
            let v = match c.named() {
                Some("none") => return Some(vec![d("overflow", "visible"), d("display", "block"), d("-webkit-box-orient", "horizontal"), d("-webkit-line-clamp", "unset")]),
                Some(n) if is_positive_integer(n) => n.to_string(),
                _ => c.arbitrary()?.0.to_string(),
            };
            Some(vec![d("overflow", "hidden"), d("display", "-webkit-box"), d("-webkit-box-orient", "vertical"), d("-webkit-line-clamp", v)])
        }
        "bg" => {
            if let Some(t) = arbitrary_type(c, &["image", "color"])
                && (t == "image" || t == "url") {
                    let (v, _) = c.arbitrary()?;
                    return c.modifier.is_none().then(|| vec![d("background-image", v)]);
                }
            color(c, &["--background-color", "--color"]).map(|v| vec![d("background-color", v)])
        }
        "accent" => color(c, &["--accent-color", "--color"]).map(|v| vec![d("accent-color", v)]),
        "caret" => color(c, &["--caret-color", "--color"]).map(|v| vec![d("caret-color", v)]),
        "fill" => color(c, &["--fill", "--color"]).map(|v| vec![d("fill", v)]),
        "stroke" => {
            if let Some(n) = c.named().filter(|n| is_positive_integer(n)) {
                return Some(vec![d("stroke-width", n)]);
            }
            color(c, &["--stroke", "--color"]).map(|v| vec![d("stroke", v)])
        }
        "decoration" => {
            if let Some(n) = c.named().filter(|n| is_positive_integer(n)) {
                return Some(vec![d("text-decoration-thickness", format!("{n}px"))]);
            }
            color(c, &["--text-decoration-color", "--color"]).map(|v| vec![d("text-decoration-color", v)])
        }
        "border" | "border-x" | "border-y" | "border-s" | "border-e" | "border-t" | "border-r" | "border-b" | "border-l" => {
            border(c)
        }
        "rounded" | "rounded-s" | "rounded-e" | "rounded-t" | "rounded-r" | "rounded-b" | "rounded-l" | "rounded-ss"
        | "rounded-se" | "rounded-ee" | "rounded-es" | "rounded-tl" | "rounded-tr" | "rounded-br" | "rounded-bl" => {
            let props = radius_props(root)?;
            if c.negative {
                return None;
            }
            let v = match (&c.value, &c.modifier) {
                (_, Some(_)) => return None,
                (None, _) => resolve(None, &["--radius"])?,
                (Some(Value::Arbitrary { value, .. }), _) => value.clone(),
                (Some(Value::Named { value, .. }), _) => match value.as_str() {
                    "none" => "0".to_string(),
                    "full" => "calc(infinity * 1px)".to_string(),
                    v => resolve(Some(v), &["--radius"])?,
                },
            };
            Some(all(props, &v))
        }
        "shadow" => shadow(c),
        "ring" => ring(c),
        "ring-offset" => {
            if c.negative {
                return None;
            }
            if let Some(t) = arbitrary_type(c, &["color", "length"]) {
                let (v, _) = c.arbitrary()?;
                if t == "length" {
                    return c.modifier.is_none().then(|| vec![d("--tw-ring-offset-width", v), d("--tw-ring-offset-shadow", RING_OFFSET_SHADOW)]);
                }
                return as_color(v, &c.modifier).map(|v| vec![d("--tw-ring-offset-color", v)]);
            }
            let n = c.named()?;
            if c.modifier.is_none()
                && let Some(v) = resolve(Some(n), &["--ring-offset-width"]).or_else(|| px_bare(n)) {
                    return Some(vec![d("--tw-ring-offset-width", v), d("--tw-ring-offset-shadow", RING_OFFSET_SHADOW)]);
                }
            color(c, &["--ring-offset-color", "--color"]).map(|v| vec![d("--tw-ring-offset-color", v)])
        }
        "outline" => {
            if c.negative {
                return None;
            }
            match &c.value {
                None => c.modifier.is_none().then(|| vec![d("outline-style", "var(--tw-outline-style)"), d("outline-width", "1px")]),
                Some(Value::Arbitrary { value, .. }) => {
                    let t = arbitrary_type(c, &["color", "length", "number", "percentage"])?;
                    if t == "length" || t == "number" || t == "percentage" {
                        return c.modifier.is_none().then(|| vec![d("outline-style", "var(--tw-outline-style)"), d("outline-width", value.clone())]);
                    }
                    as_color(value, &c.modifier).map(|v| vec![d("outline-color", v)])
                }
                Some(Value::Named { value, .. }) => {
                    if let Some(v) = color(c, &["--outline-color", "--color"]) {
                        return Some(vec![d("outline-color", v)]);
                    }
                    if c.modifier.is_some() {
                        return None;
                    }
                    let w = resolve(Some(value), &["--outline-width"]).or_else(|| px_bare(value))?;
                    Some(vec![d("outline-style", "var(--tw-outline-style)"), d("outline-width", w)])
                }
            }
        }
        "outline-offset" => {
            let spec = Spec { theme: &["--outline-offset"], negative: true, bare: Some(&px_bare), ..SPEC };
            c.value.as_ref()?;
            value_of(c, &spec).map(|(v, _)| vec![d("outline-offset", v)])
        }
        "divide-x" | "divide-y" => {
            let x = root == "divide-x";
            let spec = Spec { theme: &["--divide-width", "--border-width"], default: Some("1px"), bare: Some(&px_bare), ..SPEC };
            if let Some(n) = c.named()
                && !is_positive_integer(n) && resolve(Some(n), &["--divide-width", "--border-width"]).is_none() {
                    return None;
                }
            let (v, _) = value_of(c, &spec)?;
            let (rev, start, end, sort) = if x {
                ("--tw-divide-x-reverse", "inline-start", "inline-end", "divide-x-width")
            } else {
                ("--tw-divide-y-reverse", "top", "bottom", "divide-y-width")
            };
            Some(vec![rule(":where(& > :not(:last-child))", vec![
                d("--tw-sort", sort),
                d(rev, "0"),
                d(&format!("border-{end}-style"), "var(--tw-border-style)"),
                d(&format!("border-{start}-style"), "var(--tw-border-style)"),
                d(&format!("border-{start}-width"), format!("calc({v} * var({rev}))")),
                d(&format!("border-{end}-width"), format!("calc({v} * calc(1 - var({rev})))")),
            ])])
        }
        "divide" => color(c, &["--divide-color", "--border-color", "--color"])
            .map(|v| vec![rule(":where(& > :not(:last-child))", vec![d("--tw-sort", "divide-color"), d("border-color", v)])]),
        "space-x" | "space-y" => {
            let v = spacing(c, &["--space", "--spacing"], true, false)?;
            let (rev, start, end) = if root == "space-x" {
                ("--tw-space-x-reverse", "margin-inline-start", "margin-inline-end")
            } else {
                ("--tw-space-y-reverse", "margin-block-start", "margin-block-end")
            };
            Some(vec![rule(":where(& > :not(:last-child))", vec![
                d("--tw-sort", "row-gap"),
                d(rev, "0"),
                d(start, format!("calc({v} * var({rev}))")),
                d(end, format!("calc({v} * calc(1 - var({rev})))")),
            ])])
        }
        "content" => {
            let (v, _) = c.arbitrary()?;
            (!c.negative && c.modifier.is_none()).then(|| vec![d("--tw-content", v), d("content", "var(--tw-content)")])
        }
        "blur" | "backdrop-blur" => {
            if c.negative || c.modifier.is_some() {
                return None;
            }
            let v = match &c.value {
                None => raw("--blur")?.to_string(),
                Some(Value::Arbitrary { value, .. }) => value.clone(),
                Some(Value::Named { value, .. }) => match value.as_str() {
                    "none" => {
                        return Some(if root == "blur" {
                            vec![d("--tw-blur", " "), d("filter", FILTER)]
                        } else {
                            vec![d("--tw-backdrop-blur", " "), d("-webkit-backdrop-filter", BACKDROP), d("backdrop-filter", BACKDROP)]
                        });
                    }
                    v => resolve(Some(v), &["--blur"])?,
                },
            };
            Some(if root == "blur" {
                vec![d("--tw-blur", format!("blur({v})")), d("filter", FILTER)]
            } else {
                vec![
                    d("--tw-backdrop-blur", format!("blur({v})")),
                    d("-webkit-backdrop-filter", BACKDROP),
                    d("backdrop-filter", BACKDROP),
                ]
            })
        }
        "grayscale" | "invert" | "sepia" => {
            let spec = Spec { theme: &[], default: Some("100%"), bare: Some(&percent_bare), ..SPEC };
            let (v, _) = value_of(c, &spec)?;
            Some(vec![d(&format!("--tw-{root}"), format!("{root}({v})")), d("filter", FILTER)])
        }
        "brightness" | "contrast" | "saturate" => {
            let spec = Spec { bare: Some(&percent_bare), ..SPEC };
            c.value.as_ref()?;
            let (v, _) = value_of(c, &spec)?;
            Some(vec![d(&format!("--tw-{root}"), format!("{root}({v})")), d("filter", FILTER)])
        }
        "@container" => {
            if c.negative {
                return None;
            }
            let v = match &c.value {
                None => "inline-size".to_string(),
                Some(Value::Arbitrary { value, .. }) => value.clone(),
                Some(Value::Named { value, .. }) if value == "normal" => "normal".to_string(),
                _ => return None,
            };
            Some(match &c.modifier {
                Some(Modifier::Named(name)) | Some(Modifier::Arbitrary(name)) => {
                    vec![d("container-type", v), d("container-name", name.clone())]
                }
                None => vec![d("container-type", v)],
            })
        }
        "aspect" => {
            if c.negative || c.modifier.is_some() && c.fraction().is_none() {
                return None;
            }
            let v = match (&c.value, c.fraction()) {
                (Some(Value::Arbitrary { value, .. }), _) => value.clone(),
                (_, Some(f)) if is_fraction(f) => f.replace('/', " / "),
                (Some(Value::Named { value, .. }), _) => match value.as_str() {
                    "auto" => "auto".into(),
                    "square" => "1 / 1".into(),
                    v => resolve(Some(v), &["--aspect"])?,
                },
                _ => return None,
            };
            Some(vec![d("aspect-ratio", v)])
        }
        "will-change" => {
            let (v, _) = c.arbitrary()?;
            Some(vec![d("will-change", v)])
        }
        _ => None,
    }
}

/// `text-sm` (Schriftgrösse) oder `text-muted-foreground` (Farbe).
fn text(c: &C) -> Option<Vec<Node>> {
    if c.negative {
        return None;
    }
    if let Some((v, hint)) = c.arbitrary() {
        let t = hint.map(str::to_string).or_else(|| {
            infer(v, &["color", "length", "percentage", "absolute-size", "relative-size"]).map(str::to_string)
        });
        return match t.as_deref() {
            Some("size" | "length" | "percentage" | "absolute-size" | "relative-size") => {
                if let Some(m) = &c.modifier {
                    let lh = line_height_modifier(m)?;
                    return Some(vec![d("font-size", v), d("line-height", lh)]);
                }
                Some(vec![d("font-size", v)])
            }
            _ => as_color(&substitute_theme(v), &c.modifier).map(|v| vec![d("color", v)]),
        };
    }
    let n = c.named()?;
    if let Some(size) = resolve(Some(n), &["--text"]) {
        let lh = match &c.modifier {
            Some(m) => line_height_modifier(m)?,
            None => match theme::var(&format!("--text-{n}--line-height")) {
                Some(lh) => format!("var(--tw-leading, {lh})"),
                None => return Some(vec![d("font-size", size)]),
            },
        };
        return Some(vec![d("font-size", size), d("line-height", lh)]);
    }
    color(c, &["--text-color", "--color"]).map(|v| vec![d("color", v)])
}

fn line_height_modifier(m: &Modifier) -> Option<String> {
    match m {
        Modifier::Arbitrary(v) => Some(v.clone()),
        Modifier::Named(n) => resolve(Some(n), &["--leading"]).or_else(|| spacing_bare(n)),
    }
}

/// `font-mono` (Schriftfamilie) oder `font-medium` (Gewicht).
fn font(c: &C) -> Option<Vec<Node>> {
    if c.negative || c.modifier.is_some() {
        return None;
    }
    if let Some((v, hint)) = c.arbitrary() {
        let t = hint.map(str::to_string).or_else(|| infer(v, &["number", "generic-name", "family-name"]).map(str::to_string));
        return Some(match t.as_deref() {
            Some("generic-name" | "family-name") => vec![d("font-family", v)],
            _ => vec![d("--tw-font-weight", v), d("font-weight", v)],
        });
    }
    let n = c.named()?;
    if let Some(f) = resolve(Some(n), &["--font"]) {
        return Some(vec![d("font-family", f)]);
    }
    let w = resolve(Some(n), &["--font-weight"])?;
    Some(vec![d("--tw-font-weight", w.clone()), d("font-weight", w)])
}

/// Rahmen: Breite (`border`, `border-2`, `border-t-0`) oder Farbe (`border-border`, `border-t-transparent`).
fn border(c: &C) -> Option<Vec<Node>> {
    if c.negative {
        return None;
    }
    let sides = border_props(c.root)?;
    let color_props: Vec<String> = sides.iter().map(|s| format!("{s}-color")).collect();
    let colors = |v: String| color_props.iter().map(|p| d(p, v.clone())).collect::<Vec<_>>();
    match &c.value {
        None => c.modifier.is_none().then(|| border_width(sides, "1px".into())),
        Some(Value::Arbitrary { value, .. }) => {
            let t = arbitrary_type(c, &["color", "line-width", "length"])?;
            if t == "length" || t == "line-width" {
                return c.modifier.is_none().then(|| border_width(sides, value.clone()));
            }
            as_color(&substitute_theme(value), &c.modifier).map(colors)
        }
        Some(Value::Named { value, .. }) => {
            if let Some(v) = color(c, &["--border-color", "--color"]) {
                return Some(colors(v));
            }
            if c.modifier.is_some() {
                return None;
            }
            let w = resolve(Some(value), &["--border-width"]).or_else(|| px_bare(value))?;
            Some(border_width(sides, w))
        }
    }
}

fn shadow(c: &C) -> Option<Vec<Node>> {
    if c.negative {
        return None;
    }
    let with_shadow = |v: &str| vec![d("--tw-shadow", shadow_value(v)), d("box-shadow", SHADOW)];
    match &c.value {
        None => c.modifier.is_none().then(|| with_shadow(raw("--shadow").unwrap_or_default())),
        Some(Value::Arbitrary { value, hint }) => {
            let value = substitute_theme(value);
            let t = hint.as_deref().map(str::to_string).or_else(|| infer(&value, &["color"]).map(str::to_string));
            if t.as_deref() == Some("color") {
                return as_color(&value, &c.modifier)
                    .map(|v| vec![d("--tw-shadow-color", with_alpha(&v, "var(--tw-shadow-alpha)"))]);
            }
            Some(with_shadow(&value))
        }
        Some(Value::Named { value, .. }) => {
            if value == "none" {
                return c.modifier.is_none().then(|| vec![d("--tw-shadow", "0 0 #0000"), d("box-shadow", SHADOW)]);
            }
            if let Some(v) = raw(&format!("--shadow-{value}")) {
                return c.modifier.is_none().then(|| with_shadow(v));
            }
            color(c, &["--box-shadow-color", "--color"])
                .map(|v| vec![d("--tw-shadow-color", with_alpha(&v, "var(--tw-shadow-alpha)"))])
        }
    }
}

fn ring(c: &C) -> Option<Vec<Node>> {
    if c.negative {
        return None;
    }
    let with_width = |w: &str| vec![d("--tw-ring-shadow", ring_shadow(w)), d("box-shadow", SHADOW)];
    match &c.value {
        None => c.modifier.is_none().then(|| with_width("1px")),
        Some(Value::Arbitrary { value, .. }) => {
            let t = arbitrary_type(c, &["color", "length"])?;
            if t == "length" {
                return c.modifier.is_none().then(|| with_width(value));
            }
            as_color(&substitute_theme(value), &c.modifier).map(|v| vec![d("--tw-ring-color", v)])
        }
        Some(Value::Named { value, .. }) => {
            if let Some(v) = color(c, &["--ring-color", "--color"]) {
                return Some(vec![d("--tw-ring-color", v)]);
            }
            if c.modifier.is_some() {
                return None;
            }
            let w = resolve(Some(value), &["--ring-width"]).or_else(|| px_bare(value))?;
            Some(with_width(&w))
        }
    }
}

/// Klassen ohne Wert (`flex`, `hidden`, `truncate` …) und die Hilfsklassen des nrz-Designs.
fn static_utility(name: &str) -> Option<Vec<Node>> {
    let one = |p: &str, v: &str| Some(vec![d(p, v)]);
    let display = |v: &str| Some(vec![d("display", v)]);
    match name {
        // nrz-Hilfsklassen (Bewegung, Lichtflecken, Safe-Area, Diagramme)
        "animate-nrz-enter" => one("animation", "nrz-enter 150ms ease-out"),
        "animate-nrz-fade" => one("animation", "nrz-fade-in 150ms ease-out"),
        "animate-nrz-sheet" => one("animation", "nrz-sheet-in 200ms cubic-bezier(0.16, 1, 0.3, 1)"),
        "animate-nrz-toast" => one("animation", "nrz-toast-in 260ms cubic-bezier(0.16, 1, 0.3, 1)"),
        // Hintergrund der Anmeldeseite: zwei weiche, graue Lichtflecken. Markup: div { class: "nrz-drift", span {} span {} }
        "nrz-drift" => Some(vec![
            d("position", "absolute"),
            d("inset", "0"),
            d("overflow", "hidden"),
            d("pointer-events", "none"),
            rule("& > span", vec![
                d("position", "absolute"),
                d("display", "block"),
                d("border-radius", "50%"),
                d("filter", "blur(70px)"),
                d("will-change", "transform"),
            ]),
            rule("& > span:nth-child(1)", vec![
                d("width", "44vmax"),
                d("height", "44vmax"),
                d("left", "-10vmax"),
                d("top", "-12vmax"),
                d("background", "radial-gradient(circle, oklch(from var(--foreground) l c h / 6%), transparent 65%)"),
                d("animation", "nrz-drift-a 32s ease-in-out infinite"),
            ]),
            rule("& > span:nth-child(2)", vec![
                d("width", "38vmax"),
                d("height", "38vmax"),
                d("right", "-12vmax"),
                d("top", "18vh"),
                d("background", "radial-gradient(circle, oklch(from var(--foreground) l c h / 5%), transparent 65%)"),
                d("animation", "nrz-drift-b 40s ease-in-out infinite"),
            ]),
        ]),
        // iPhone: Notch und Home-Indikator
        "safe-top" => one("padding-top", "env(safe-area-inset-top)"),
        "safe-bottom" => one("padding-bottom", "env(safe-area-inset-bottom)"),
        // Diagramme ausserhalb des Bildschirms nicht zeichnen
        "chart-lazy" => Some(vec![d("content-visibility", "auto"), d("contain-intrinsic-size", "auto 260px")]),

        "container" => {
            let mut out = vec![d("--tw-sort", "--tw-container-component"), d("width", "100%")];
            for bp in ["sm", "md", "lg", "xl", "2xl"] {
                let v = raw(&format!("--breakpoint-{bp}"))?;
                out.push(at(format!("@media (width >= {v})"), vec![d("max-width", v)]));
            }
            Some(out)
        }
        "sr-only" => Some(vec![
            d("position", "absolute"),
            d("width", "1px"),
            d("height", "1px"),
            d("padding", "0"),
            d("margin", "-1px"),
            d("overflow", "hidden"),
            d("clip", "rect(0, 0, 0, 0)"),
            d("white-space", "nowrap"),
            d("border-width", "0"),
        ]),
        "pointer-events-none" => one("pointer-events", "none"),
        "pointer-events-auto" => one("pointer-events", "auto"),
        "visible" => one("visibility", "visible"),
        "invisible" => one("visibility", "hidden"),
        "collapse" => one("visibility", "collapse"),
        "static" | "fixed" | "absolute" | "relative" | "sticky" => one("position", name),
        "isolate" => one("isolation", "isolate"),
        "isolation-auto" => one("isolation", "auto"),
        "box-border" => one("box-sizing", "border-box"),
        "box-content" => one("box-sizing", "content-box"),
        "block" | "inline-block" | "inline" | "flex" | "inline-flex" | "table" | "inline-table" | "table-caption"
        | "table-cell" | "table-column" | "table-column-group" | "table-footer-group" | "table-header-group"
        | "table-row-group" | "table-row" | "flow-root" | "grid" | "inline-grid" | "contents" | "list-item" => display(name),
        "hidden" => display("none"),
        "field-sizing-content" => one("field-sizing", "content"),
        "field-sizing-fixed" => one("field-sizing", "fixed"),
        "flex-auto" => one("flex", "auto"),
        "flex-initial" => one("flex", "0 auto"),
        "flex-none" => one("flex", "none"),
        "flex-row" => one("flex-direction", "row"),
        "flex-row-reverse" => one("flex-direction", "row-reverse"),
        "flex-col" => one("flex-direction", "column"),
        "flex-col-reverse" => one("flex-direction", "column-reverse"),
        "flex-wrap" => one("flex-wrap", "wrap"),
        "flex-nowrap" => one("flex-wrap", "nowrap"),
        "flex-wrap-reverse" => one("flex-wrap", "wrap-reverse"),
        "grid-flow-row" => one("grid-auto-flow", "row"),
        "grid-flow-col" => one("grid-auto-flow", "column"),
        "grid-flow-dense" => one("grid-auto-flow", "dense"),
        "table-auto" => one("table-layout", "auto"),
        "table-fixed" => one("table-layout", "fixed"),
        "caption-top" => one("caption-side", "top"),
        "caption-bottom" => one("caption-side", "bottom"),
        "border-collapse" => one("border-collapse", "collapse"),
        "border-separate" => one("border-collapse", "separate"),
        "transform" => one("transform", "var(--tw-rotate-x,) var(--tw-rotate-y,) var(--tw-rotate-z,) var(--tw-skew-x,) var(--tw-skew-y,)"),
        "transform-none" => one("transform", "none"),
        "transform-gpu" => one("transform", "translateZ(0) var(--tw-rotate-x,) var(--tw-rotate-y,) var(--tw-rotate-z,) var(--tw-skew-x,) var(--tw-skew-y,)"),
        "origin-center" => one("transform-origin", "center"),
        "origin-top" => one("transform-origin", "top"),
        "origin-top-right" => one("transform-origin", "top right"),
        "origin-right" => one("transform-origin", "right"),
        "origin-bottom-right" => one("transform-origin", "bottom right"),
        "origin-bottom" => one("transform-origin", "bottom"),
        "origin-bottom-left" => one("transform-origin", "bottom left"),
        "origin-left" => one("transform-origin", "left"),
        "origin-top-left" => one("transform-origin", "top left"),
        "animate-none" => one("animation", "none"),
        "touch-auto" => one("touch-action", "auto"),
        "touch-none" => one("touch-action", "none"),
        "touch-manipulation" => one("touch-action", "manipulation"),
        "touch-pan-x" | "touch-pan-left" | "touch-pan-right" => {
            Some(vec![d("--tw-pan-x", &name[6..]), d("touch-action", "var(--tw-pan-x,) var(--tw-pan-y,) var(--tw-pinch-zoom,)")])
        }
        "touch-pan-y" | "touch-pan-up" | "touch-pan-down" => {
            Some(vec![d("--tw-pan-y", &name[6..]), d("touch-action", "var(--tw-pan-x,) var(--tw-pan-y,) var(--tw-pinch-zoom,)")])
        }
        "touch-pinch-zoom" => {
            Some(vec![d("--tw-pinch-zoom", "pinch-zoom"), d("touch-action", "var(--tw-pan-x,) var(--tw-pan-y,) var(--tw-pinch-zoom,)")])
        }
        "resize-none" => one("resize", "none"),
        "resize-x" => one("resize", "horizontal"),
        "resize-y" => one("resize", "vertical"),
        "resize" => one("resize", "both"),
        "snap-none" => one("scroll-snap-type", "none"),
        "snap-x" | "snap-y" | "snap-both" => {
            one("scroll-snap-type", &format!("{} var(--tw-scroll-snap-strictness)", &name[5..]))
        }
        "snap-mandatory" => one("--tw-scroll-snap-strictness", "mandatory"),
        "snap-proximity" => one("--tw-scroll-snap-strictness", "proximity"),
        "snap-align-none" => one("scroll-snap-align", "none"),
        "snap-start" => one("scroll-snap-align", "start"),
        "snap-end" => one("scroll-snap-align", "end"),
        "snap-center" => one("scroll-snap-align", "center"),
        "snap-normal" => one("scroll-snap-stop", "normal"),
        "snap-always" => one("scroll-snap-stop", "always"),
        "list-inside" => one("list-style-position", "inside"),
        "list-outside" => one("list-style-position", "outside"),
        "list-none" => one("list-style-type", "none"),
        "list-disc" => one("list-style-type", "disc"),
        "list-decimal" => one("list-style-type", "decimal"),
        "appearance-none" => one("appearance", "none"),
        "appearance-auto" => one("appearance", "auto"),
        "place-content-center" => one("place-content", "center"),
        "place-content-start" => one("place-content", "start"),
        "place-content-end" => one("place-content", "end"),
        "place-content-between" => one("place-content", "space-between"),
        "place-content-around" => one("place-content", "space-around"),
        "place-content-evenly" => one("place-content", "space-evenly"),
        "place-content-baseline" => one("place-content", "baseline"),
        "place-content-stretch" => one("place-content", "stretch"),
        "place-items-center" => one("place-items", "center"),
        "place-items-start" => one("place-items", "start"),
        "place-items-end" => one("place-items", "end"),
        "place-items-baseline" => one("place-items", "baseline"),
        "place-items-stretch" => one("place-items", "stretch"),
        "content-normal" => one("align-content", "normal"),
        "content-center" => one("align-content", "center"),
        "content-start" => one("align-content", "flex-start"),
        "content-end" => one("align-content", "flex-end"),
        "content-between" => one("align-content", "space-between"),
        "content-around" => one("align-content", "space-around"),
        "content-evenly" => one("align-content", "space-evenly"),
        "content-baseline" => one("align-content", "baseline"),
        "content-stretch" => one("align-content", "stretch"),
        "content-none" => Some(vec![d("--tw-content", "none"), d("content", "none")]),
        "items-center" => one("align-items", "center"),
        "items-stretch" => one("align-items", "stretch"),
        "items-start" => one("align-items", "flex-start"),
        "items-end" => one("align-items", "flex-end"),
        "items-baseline" => one("align-items", "baseline"),
        "justify-normal" => one("justify-content", "normal"),
        "justify-center" => one("justify-content", "center"),
        "justify-start" => one("justify-content", "flex-start"),
        "justify-end" => one("justify-content", "flex-end"),
        "justify-between" => one("justify-content", "space-between"),
        "justify-around" => one("justify-content", "space-around"),
        "justify-evenly" => one("justify-content", "space-evenly"),
        "justify-baseline" => one("justify-content", "baseline"),
        "justify-stretch" => one("justify-content", "stretch"),
        "justify-items-normal" => one("justify-items", "normal"),
        "justify-items-center" => one("justify-items", "center"),
        "justify-items-start" => one("justify-items", "start"),
        "justify-items-end" => one("justify-items", "end"),
        "justify-items-stretch" => one("justify-items", "stretch"),
        "place-self-auto" => one("place-self", "auto"),
        "place-self-start" => one("place-self", "start"),
        "place-self-end" => one("place-self", "end"),
        "place-self-center" => one("place-self", "center"),
        "place-self-stretch" => one("place-self", "stretch"),
        "self-auto" => one("align-self", "auto"),
        "self-start" => one("align-self", "flex-start"),
        "self-end" => one("align-self", "flex-end"),
        "self-center" => one("align-self", "center"),
        "self-stretch" => one("align-self", "stretch"),
        "self-baseline" => one("align-self", "baseline"),
        "justify-self-auto" => one("justify-self", "auto"),
        "justify-self-start" => one("justify-self", "flex-start"),
        "justify-self-end" => one("justify-self", "flex-end"),
        "justify-self-center" => one("justify-self", "center"),
        "justify-self-stretch" => one("justify-self", "stretch"),
        "overflow-auto" | "overflow-hidden" | "overflow-clip" | "overflow-visible" | "overflow-scroll" => {
            one("overflow", &name[9..])
        }
        "overflow-x-auto" | "overflow-x-hidden" | "overflow-x-clip" | "overflow-x-visible" | "overflow-x-scroll" => {
            one("overflow-x", &name[11..])
        }
        "overflow-y-auto" | "overflow-y-hidden" | "overflow-y-clip" | "overflow-y-visible" | "overflow-y-scroll" => {
            one("overflow-y", &name[11..])
        }
        "overscroll-auto" | "overscroll-contain" | "overscroll-none" => one("overscroll-behavior", &name[11..]),
        "overscroll-x-auto" | "overscroll-x-contain" | "overscroll-x-none" => one("overscroll-behavior-x", &name[13..]),
        "overscroll-y-auto" | "overscroll-y-contain" | "overscroll-y-none" => one("overscroll-behavior-y", &name[13..]),
        "scroll-auto" => one("scroll-behavior", "auto"),
        "scroll-smooth" => one("scroll-behavior", "smooth"),
        "truncate" => Some(vec![d("overflow", "hidden"), d("text-overflow", "ellipsis"), d("white-space", "nowrap")]),
        "text-ellipsis" => one("text-overflow", "ellipsis"),
        "text-clip" => one("text-overflow", "clip"),
        "whitespace-normal" | "whitespace-nowrap" | "whitespace-pre" | "whitespace-pre-line" | "whitespace-pre-wrap"
        | "whitespace-break-spaces" => one("white-space", &name[11..]),
        "text-wrap" => one("text-wrap", "wrap"),
        "text-nowrap" => one("text-wrap", "nowrap"),
        "text-balance" => one("text-wrap", "balance"),
        "text-pretty" => one("text-wrap", "pretty"),
        "break-normal" => Some(vec![d("overflow-wrap", "normal"), d("word-break", "normal")]),
        "break-words" => one("overflow-wrap", "break-word"),
        "break-all" => one("word-break", "break-all"),
        "break-keep" => one("word-break", "keep-all"),
        "wrap-anywhere" => one("overflow-wrap", "anywhere"),
        "wrap-break-word" => one("overflow-wrap", "break-word"),
        "wrap-normal" => one("overflow-wrap", "normal"),
        "border-solid" | "border-dashed" | "border-dotted" | "border-double" | "border-hidden" | "border-none" => {
            Some(vec![d("--tw-border-style", &name[7..]), d("border-style", &name[7..])])
        }
        "bg-fixed" => one("background-attachment", "fixed"),
        "bg-local" => one("background-attachment", "local"),
        "bg-scroll" => one("background-attachment", "scroll"),
        "bg-clip-border" => one("background-clip", "border-box"),
        "bg-clip-padding" => one("background-clip", "padding-box"),
        "bg-clip-content" => one("background-clip", "content-box"),
        "bg-clip-text" => one("background-clip", "text"),
        "bg-cover" => one("background-size", "cover"),
        "bg-contain" => one("background-size", "contain"),
        "bg-center" => one("background-position", "center"),
        "bg-no-repeat" => one("background-repeat", "no-repeat"),
        "bg-none" => one("background-image", "none"),
        "object-contain" | "object-cover" | "object-fill" | "object-none" | "object-scale-down" => one("object-fit", &name[7..]),
        "object-center" => one("object-position", "center"),
        "object-top" => one("object-position", "top"),
        "object-bottom" => one("object-position", "bottom"),
        "text-left" | "text-center" | "text-right" | "text-justify" | "text-start" | "text-end" => one("text-align", &name[5..]),
        "align-baseline" | "align-top" | "align-middle" | "align-bottom" | "align-text-top" | "align-text-bottom"
        | "align-sub" | "align-super" => one("vertical-align", &name[6..]),
        "uppercase" | "lowercase" | "capitalize" => one("text-transform", name),
        "normal-case" => one("text-transform", "none"),
        "italic" => one("font-style", "italic"),
        "not-italic" => one("font-style", "normal"),
        "normal-nums" => one("font-variant-numeric", "normal"),
        "ordinal" | "slashed-zero" | "lining-nums" | "oldstyle-nums" | "proportional-nums" | "tabular-nums"
        | "diagonal-fractions" | "stacked-fractions" => {
            let var = match name {
                "ordinal" => "--tw-ordinal",
                "slashed-zero" => "--tw-slashed-zero",
                "lining-nums" | "oldstyle-nums" => "--tw-numeric-figure",
                "proportional-nums" | "tabular-nums" => "--tw-numeric-spacing",
                _ => "--tw-numeric-fraction",
            };
            Some(vec![
                d(var, name),
                d("font-variant-numeric", "var(--tw-ordinal,) var(--tw-slashed-zero,) var(--tw-numeric-figure,) var(--tw-numeric-spacing,) var(--tw-numeric-fraction,)"),
            ])
        }
        "underline" => one("text-decoration-line", "underline"),
        "overline" => one("text-decoration-line", "overline"),
        "line-through" => one("text-decoration-line", "line-through"),
        "no-underline" => one("text-decoration-line", "none"),
        "antialiased" => Some(vec![d("-webkit-font-smoothing", "antialiased"), d("-moz-osx-font-smoothing", "grayscale")]),
        "subpixel-antialiased" => Some(vec![d("-webkit-font-smoothing", "auto"), d("-moz-osx-font-smoothing", "auto")]),
        "select-none" | "select-text" | "select-all" | "select-auto" => {
            Some(vec![d("-webkit-user-select", &name[7..]), d("user-select", &name[7..])])
        }
        "outline-hidden" => Some(vec![
            d("--tw-outline-style", "none"),
            d("outline-style", "none"),
            at("@media (forced-colors: active)", vec![d("outline", "2px solid transparent"), d("outline-offset", "2px")]),
        ]),
        "outline-none" => Some(vec![d("--tw-outline-style", "none"), d("outline-style", "none")]),
        "outline-solid" | "outline-dashed" | "outline-dotted" | "outline-double" => {
            Some(vec![d("--tw-outline-style", &name[8..]), d("outline-style", &name[8..])])
        }
        "filter" => one("filter", FILTER),
        "filter-none" => one("filter", "none"),
        "backdrop-filter" => Some(vec![d("-webkit-backdrop-filter", BACKDROP), d("backdrop-filter", BACKDROP)]),
        "backdrop-filter-none" => Some(vec![d("-webkit-backdrop-filter", "none"), d("backdrop-filter", "none")]),
        "transition-none" => one("transition-property", "none"),
        "transition-all" | "transition-colors" | "transition-opacity" | "transition-shadow" | "transition-transform" => {
            let props = match name {
                "transition-all" => "all",
                "transition-colors" => TRANSITION_COLORS,
                "transition-opacity" => "opacity",
                "transition-shadow" => "box-shadow",
                _ => "transform, translate, scale, rotate",
            };
            Some(vec![d("transition-property", props), d("transition-timing-function", EASE), d("transition-duration", DURATION)])
        }
        "ease-linear" => Some(vec![d("--tw-ease", "linear"), d("transition-timing-function", "linear")]),
        "will-change-auto" => one("will-change", "auto"),
        "will-change-scroll" => one("will-change", "scroll-position"),
        "will-change-contents" => one("will-change", "contents"),
        "will-change-transform" => one("will-change", "transform"),
        "cursor-auto" | "cursor-default" | "cursor-pointer" | "cursor-wait" | "cursor-text" | "cursor-move"
        | "cursor-help" | "cursor-not-allowed" | "cursor-none" | "cursor-context-menu" | "cursor-progress"
        | "cursor-cell" | "cursor-crosshair" | "cursor-vertical-text" | "cursor-alias" | "cursor-copy"
        | "cursor-no-drop" | "cursor-grab" | "cursor-grabbing" | "cursor-all-scroll" | "cursor-col-resize"
        | "cursor-row-resize" | "cursor-n-resize" | "cursor-e-resize" | "cursor-s-resize" | "cursor-w-resize"
        | "cursor-ne-resize" | "cursor-nw-resize" | "cursor-se-resize" | "cursor-sw-resize" | "cursor-ew-resize"
        | "cursor-ns-resize" | "cursor-nesw-resize" | "cursor-nwse-resize" | "cursor-zoom-in" | "cursor-zoom-out" => {
            one("cursor", &name[7..])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decls(class: &str) -> Vec<(String, String)> {
        compile(class)
            .unwrap_or_else(|| panic!("{class} unbekannt"))
            .into_iter()
            .filter_map(|n| match n {
                Node::Decl(p, v) => Some((p, v)),
                _ => None,
            })
            .collect()
    }

    fn one(class: &str) -> String {
        decls(class).into_iter().filter(|(p, _)| p != "--tw-sort").map(|(p, v)| format!("{p}: {v}")).collect::<Vec<_>>().join("; ")
    }

    #[test]
    fn wie_tailwind() {
        assert_eq!(one("gap-1.5"), "gap: calc(var(--spacing) * 1.5)");
        assert_eq!(one("-mt-1"), "margin-top: calc(var(--spacing) * -1)");
        assert_eq!(one("-translate-x-1/2"), "--tw-translate-x: calc(calc(1/2 * 100%) * -1); translate: var(--tw-translate-x) var(--tw-translate-y)");
        assert_eq!(one("top-1/2"), "top: calc(1/2 * 100%)");
        assert_eq!(one("max-w-3xl"), "max-width: var(--container-3xl)");
        assert_eq!(one("size-full"), "width: 100%; height: 100%");
        assert_eq!(one("bg-card/50"), "background-color: color-mix(in oklab, var(--card) 50%, transparent)");
        assert_eq!(one("bg-black/50"), "background-color: color-mix(in oklab, var(--color-black) 50%, transparent)");
        assert_eq!(one("text-sm"), "font-size: var(--text-sm); line-height: var(--tw-leading, var(--text-sm--line-height))");
        assert_eq!(one("text-[2vmin]"), "font-size: 2vmin");
        assert_eq!(one("text-[var(--warning)]"), "color: var(--warning)");
        assert_eq!(one("rounded"), "border-radius: 0.25rem");
        assert_eq!(one("rounded-t-xl"), "border-top-left-radius: calc(var(--radius) + 4px); border-top-right-radius: calc(var(--radius) + 4px)");
        assert_eq!(one("border-t"), "border-top-style: var(--tw-border-style); border-top-width: 1px");
        assert_eq!(one("ring-2"), "--tw-ring-shadow: var(--tw-ring-inset,) 0 0 0 calc(2px + var(--tw-ring-offset-width)) var(--tw-ring-color, currentcolor); box-shadow: var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow)");
        assert_eq!(one("grid-cols-2"), "grid-template-columns: repeat(2, minmax(0, 1fr))");
        assert_eq!(one("w-[calc(50%-0.25rem)]"), "width: calc(50% - 0.25rem)");
        assert_eq!(one("[-webkit-tap-highlight-color:transparent]"), "-webkit-tap-highlight-color: transparent");
        assert_eq!(one("opacity-35"), "opacity: 35%");
        assert_eq!(one("z-[1000000]"), "z-index: 1000000");
        assert!(compile("px-{x}").is_none());
        assert!(compile("foo-bar").is_none());
    }
}
