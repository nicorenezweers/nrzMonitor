//! Varianten wie `hover:`, `sm:`, `dark:`, `data-[state=open]:`, `group-hover/card:` oder
//! `[&_svg]:`. Selektoren und Reihenfolge wie bei Tailwind CSS v4.1 (MIT-Lizenz, siehe
//! `LICENSE-tailwindcss`); `dark` ist wie im nrz-Thema die Klasse `.dark` am `<html>`.

use std::cmp::Ordering;

use crate::css::{Node, at, d, rule, split_top};
use crate::theme;
use crate::value::{decode_arbitrary, quote_attribute_value, valid_arbitrary};

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Named(String),
    Arbitrary(String),
}

impl Val {
    fn text(&self) -> &str {
        match self {
            Val::Named(s) | Val::Arbitrary(s) => s,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Variant {
    Static(&'static str),
    Functional { root: &'static str, value: Option<Val>, modifier: Option<String> },
    Compound { root: &'static str, modifier: Option<String>, inner: Box<Variant> },
    Arbitrary { selector: String, relative: bool },
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Static,
    Functional,
    Compound,
}

/// Bekannte Varianten in Tailwinds Reihenfolge. Gleiche Zahl = gleiche Gruppe (Breakpoints).
const REGISTRY: &[(&str, Kind, u32)] = &[
    ("*", Kind::Static, 0),
    ("**", Kind::Static, 1),
    ("not", Kind::Compound, 2),
    ("group", Kind::Compound, 3),
    ("peer", Kind::Compound, 4),
    ("first-letter", Kind::Static, 5),
    ("first-line", Kind::Static, 6),
    ("marker", Kind::Static, 7),
    ("selection", Kind::Static, 8),
    ("file", Kind::Static, 9),
    ("placeholder", Kind::Static, 10),
    ("backdrop", Kind::Static, 11),
    ("details-content", Kind::Static, 12),
    ("before", Kind::Static, 13),
    ("after", Kind::Static, 14),
    ("first", Kind::Static, 15),
    ("last", Kind::Static, 16),
    ("only", Kind::Static, 17),
    ("odd", Kind::Static, 18),
    ("even", Kind::Static, 19),
    ("first-of-type", Kind::Static, 20),
    ("last-of-type", Kind::Static, 21),
    ("only-of-type", Kind::Static, 22),
    ("visited", Kind::Static, 23),
    ("target", Kind::Static, 24),
    ("open", Kind::Static, 25),
    ("default", Kind::Static, 26),
    ("checked", Kind::Static, 27),
    ("indeterminate", Kind::Static, 28),
    ("placeholder-shown", Kind::Static, 29),
    ("autofill", Kind::Static, 30),
    ("optional", Kind::Static, 31),
    ("required", Kind::Static, 32),
    ("valid", Kind::Static, 33),
    ("invalid", Kind::Static, 34),
    ("user-valid", Kind::Static, 35),
    ("user-invalid", Kind::Static, 36),
    ("in-range", Kind::Static, 37),
    ("out-of-range", Kind::Static, 38),
    ("read-only", Kind::Static, 39),
    ("empty", Kind::Static, 40),
    ("focus-within", Kind::Static, 41),
    ("hover", Kind::Static, 42),
    ("focus", Kind::Static, 43),
    ("focus-visible", Kind::Static, 44),
    ("active", Kind::Static, 45),
    ("enabled", Kind::Static, 46),
    ("disabled", Kind::Static, 47),
    ("inert", Kind::Static, 48),
    ("in", Kind::Compound, 49),
    ("has", Kind::Compound, 50),
    ("aria", Kind::Functional, 51),
    ("data", Kind::Functional, 52),
    ("nth", Kind::Functional, 53),
    ("nth-last", Kind::Functional, 54),
    ("nth-of-type", Kind::Functional, 55),
    ("nth-last-of-type", Kind::Functional, 56),
    ("supports", Kind::Functional, 57),
    ("motion-safe", Kind::Static, 58),
    ("motion-reduce", Kind::Static, 59),
    ("contrast-more", Kind::Static, 60),
    ("contrast-less", Kind::Static, 61),
    ("max", Kind::Functional, 62),
    ("sm", Kind::Static, 63),
    ("md", Kind::Static, 63),
    ("lg", Kind::Static, 63),
    ("xl", Kind::Static, 63),
    ("2xl", Kind::Static, 63),
    ("min", Kind::Functional, 63),
    ("portrait", Kind::Static, 64),
    ("landscape", Kind::Static, 65),
    ("ltr", Kind::Static, 66),
    ("rtl", Kind::Static, 67),
    ("dark", Kind::Static, 68),
    ("starting", Kind::Static, 69),
    ("print", Kind::Static, 70),
    ("forced-colors", Kind::Static, 71),
    ("noscript", Kind::Static, 72),
];

const MAX_GROUP: u32 = 62;
const MIN_GROUP: u32 = 63;

fn registered(name: &str) -> Option<(&'static str, Kind, u32)> {
    REGISTRY.iter().find(|(n, _, _)| *n == name).copied()
}

/// Variante aus ihrer Schreibweise lesen (`hover`, `data-[state=open]`, `group-hover/card` …).
pub fn parse(s: &str) -> Option<Variant> {
    if s.starts_with('[') && s.ends_with(']') {
        if s[1..].starts_with('@') && s.contains('&') {
            return None;
        }
        let sel = decode_arbitrary(&s[1..s.len() - 1]);
        if !valid_arbitrary(&sel) {
            return None;
        }
        let relative = sel.starts_with(['>', '+', '~']);
        let selector = if !relative && !sel.starts_with('@') && !sel.contains('&') { format!("&:is({sel})") } else { sel };
        return Some(Variant::Arbitrary { selector, relative });
    }
    let parts = split_top(s, '/');
    if parts.len() > 2 {
        return None;
    }
    let body = parts[0];
    let mut modifier = parts.get(1).map(|m| m.to_string());
    if modifier.as_deref() == Some("") {
        return None;
    }
    for (root, value) in roots(body) {
        let Some((name, kind, _)) = registered(root) else { continue };
        match kind {
            Kind::Static => {
                if value.is_some() || modifier.is_some() {
                    return None;
                }
                return Some(Variant::Static(name));
            }
            Kind::Functional => {
                let value = match value {
                    None => None,
                    Some(v) if v.ends_with(']') => {
                        if !v.starts_with('[') {
                            continue;
                        }
                        let a = decode_arbitrary(&v[1..v.len() - 1]);
                        if !valid_arbitrary(&a) {
                            return None;
                        }
                        Some(Val::Arbitrary(a))
                    }
                    Some(v) => Some(Val::Named(v.to_string())),
                };
                return Some(Variant::Functional { root: name, value, modifier });
            }
            Kind::Compound => {
                let mut v = value?.to_string();
                if matches!(name, "not" | "has" | "in")
                    && let Some(m) = modifier.take() {
                        v = format!("{v}/{m}");
                    }
                let inner = parse(&v)?;
                if !compounds(name, &inner) {
                    return None;
                }
                return Some(Variant::Compound { root: name, modifier, inner: Box::new(inner) });
            }
        }
    }
    None
}

/// Mögliche Aufteilungen in Wurzel und Wert, längste Wurzel zuerst (wie Tailwinds `findRoots`).
fn roots(input: &str) -> Vec<(&str, Option<&str>)> {
    let mut out = vec![(input, None)];
    let mut end = input.len();
    while let Some(i) = input[..end].rfind('-') {
        if i == 0 {
            break;
        }
        let value = &input[i + 1..];
        if value.is_empty() {
            break;
        }
        out.push((&input[..i], Some(value)));
        end = i;
    }
    out
}

/// Lässt sich `inner` mit `group-`, `peer-`, `has-` … verbinden? Nur Varianten, die reine
/// Selektoren erzeugen (keine At-Regeln, keine Pseudo-Elemente).
fn compounds(root: &str, inner: &Variant) -> bool {
    match inner {
        Variant::Arbitrary { selector, relative } => !selector.starts_with('@') && (!relative || root == "has" || root == "not"),
        Variant::Compound { .. } => true,
        other => match apply(other, vec![]) {
            Some(nodes) => nodes.iter().all(|n| matches!(n, Node::Rule(sel, _) if !sel.contains("::"))),
            None => false,
        },
    }
}

/// Variante auf den Inhalt einer Regel anwenden (`None`: passt nicht).
pub fn apply(v: &Variant, nodes: Vec<Node>) -> Option<Vec<Node>> {
    match v {
        Variant::Static(name) => apply_static(name, nodes),
        Variant::Functional { root, value, modifier } => apply_functional(root, value.as_ref(), modifier.as_deref(), nodes),
        Variant::Arbitrary { selector, relative } => {
            if *relative {
                return None;
            }
            if let Some(rest) = selector.strip_prefix('@') {
                let name_len = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(rest.len());
                let (name, params) = rest.split_at(name_len);
                Some(vec![at(format!("@{name} {}", params.trim()).trim_end().to_string(), nodes)])
            } else {
                Some(vec![rule(selector.clone(), nodes)])
            }
        }
        Variant::Compound { root, modifier, inner } => {
            let mut wrapper = match &**inner {
                Variant::Arbitrary { selector, relative: true } => vec![rule(selector.clone(), vec![])],
                other => apply(other, vec![])?,
            };
            for n in wrapper.iter_mut() {
                let Node::Rule(sel, kids) = n else { return None };
                if kids.iter().any(|k| matches!(k, Node::Rule(..))) {
                    return None;
                }
                *sel = compound_selector(root, modifier.as_deref(), sel);
            }
            fill(&mut wrapper, &nodes);
            Some(wrapper)
        }
    }
}

fn compound_selector(root: &str, modifier: Option<&str>, sel: &str) -> String {
    let marker = |name: &str| match modifier {
        Some(m) => format!(":where(.{name}\\/{m})"),
        None => format!(":where(.{name})"),
    };
    match root {
        "group" | "peer" => {
            let mut s = sel.replacen('&', &marker(root), 1);
            if split_top(&s, ',').len() > 1 {
                s = format!(":is({s})");
            }
            if root == "group" { format!("&:is({s} *)") } else { format!("&:is({s} ~ *)") }
        }
        "has" => format!("&:has({})", sel.replacen('&', "*", 1)),
        "in" => format!(":where({}) &", sel.replacen('&', "*", 1)),
        _ => format!("&:not({})", sel.replace('&', "*")),
    }
}

/// Leere Platzhalter (innerste Regel/At-Regel ohne Inhalt) mit dem eigentlichen Inhalt füllen.
fn fill(nodes: &mut [Node], content: &[Node]) {
    for n in nodes.iter_mut() {
        match n {
            Node::Rule(_, kids) | Node::At(_, kids) => {
                if kids.is_empty() {
                    *kids = content.to_vec();
                } else {
                    fill(kids, content);
                }
            }
            Node::Decl(..) => {}
        }
    }
}

fn wrap(selectors: &[&str], nodes: Vec<Node>) -> Option<Vec<Node>> {
    Some(selectors.iter().map(|s| rule(*s, nodes.clone())).collect())
}

fn media(query: &str, nodes: Vec<Node>) -> Option<Vec<Node>> {
    Some(vec![at(format!("@media {query}"), nodes)])
}

fn apply_static(name: &str, nodes: Vec<Node>) -> Option<Vec<Node>> {
    let pseudo = |p: &str, nodes: Vec<Node>| Some(vec![rule(format!("&{p}"), nodes)]);
    match name {
        "*" => wrap(&[":is(& > *)"], nodes),
        "**" => wrap(&[":is(& *)"], nodes),
        "first-letter" => pseudo("::first-letter", nodes),
        "first-line" => pseudo("::first-line", nodes),
        "marker" => wrap(&["& *::marker", "&::marker"], nodes),
        "selection" => wrap(&["& *::selection", "&::selection"], nodes),
        "file" => pseudo("::file-selector-button", nodes),
        "placeholder" => pseudo("::placeholder", nodes),
        "backdrop" => pseudo("::backdrop", nodes),
        "details-content" => pseudo("::details-content", nodes),
        "before" | "after" => {
            let mut inner = vec![d("content", "var(--tw-content)")];
            inner.extend(nodes);
            Some(vec![rule(format!("&::{name}"), inner)])
        }
        "first" => pseudo(":first-child", nodes),
        "last" => pseudo(":last-child", nodes),
        "only" => pseudo(":only-child", nodes),
        "odd" => pseudo(":nth-child(odd)", nodes),
        "even" => pseudo(":nth-child(even)", nodes),
        "open" => pseudo(":is([open], :popover-open, :open)", nodes),
        "inert" => pseudo(":is([inert], [inert] *)", nodes),
        "hover" => Some(vec![rule("&:hover", vec![at("@media (hover: hover)", nodes)])]),
        "first-of-type" | "last-of-type" | "only-of-type" | "visited" | "target" | "default" | "checked"
        | "indeterminate" | "placeholder-shown" | "autofill" | "optional" | "required" | "valid" | "invalid"
        | "user-valid" | "user-invalid" | "in-range" | "out-of-range" | "read-only" | "empty" | "focus-within"
        | "focus" | "focus-visible" | "active" | "enabled" | "disabled" => pseudo(&format!(":{name}"), nodes),
        "motion-safe" => media("(prefers-reduced-motion: no-preference)", nodes),
        "motion-reduce" => media("(prefers-reduced-motion: reduce)", nodes),
        "contrast-more" => media("(prefers-contrast: more)", nodes),
        "contrast-less" => media("(prefers-contrast: less)", nodes),
        "sm" | "md" | "lg" | "xl" | "2xl" => media(&format!("(width >= {})", breakpoint(name)?), nodes),
        "portrait" => media("(orientation: portrait)", nodes),
        "landscape" => media("(orientation: landscape)", nodes),
        "ltr" => wrap(&["&:where(:dir(ltr), [dir=\"ltr\"], [dir=\"ltr\"] *)"], nodes),
        "rtl" => wrap(&["&:where(:dir(rtl), [dir=\"rtl\"], [dir=\"rtl\"] *)"], nodes),
        // nrz-Thema: dunkel über die Klasse `.dark` am <html>
        "dark" => wrap(&["&:where(.dark, .dark *)"], nodes),
        "starting" => Some(vec![at("@starting-style", nodes)]),
        "print" => media("print", nodes),
        "forced-colors" => media("(forced-colors: active)", nodes),
        "noscript" => media("(scripting: none)", nodes),
        _ => None,
    }
}

fn breakpoint(name: &str) -> Option<&'static str> {
    theme::raw(&format!("--breakpoint-{name}"))
}

fn apply_functional(root: &str, value: Option<&Val>, modifier: Option<&str>, nodes: Vec<Node>) -> Option<Vec<Node>> {
    let value = value?;
    if modifier.is_some() {
        return None;
    }
    let v = value.text();
    match (root, value) {
        ("aria", Val::Arbitrary(_)) => Some(vec![rule(format!("&[aria-{}]", quote_attribute_value(v)), nodes)]),
        ("aria", Val::Named(_)) => Some(vec![rule(format!("&[aria-{v}=\"true\"]"), nodes)]),
        ("data", Val::Arbitrary(_)) => Some(vec![rule(format!("&[data-{}]", quote_attribute_value(v)), nodes)]),
        ("data", Val::Named(_)) => Some(vec![rule(format!("&[data-{v}]"), nodes)]),
        ("nth" | "nth-last" | "nth-of-type" | "nth-last-of-type", _) => {
            if matches!(value, Val::Named(_)) && !v.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let pseudo = match root {
                "nth" => "nth-child",
                "nth-last" => "nth-last-child",
                other => other,
            };
            Some(vec![rule(format!("&:{pseudo}({v})"), nodes)])
        }
        ("supports", Val::Arbitrary(_)) => {
            let cond = if v.contains(':') && !v.starts_with('(') { format!("({v})") } else { v.to_string() };
            Some(vec![at(format!("@supports {cond}"), nodes)])
        }
        ("max", _) => media(&format!("(width < {})", breakpoint_value(value)?), nodes),
        ("min", _) => media(&format!("(width >= {})", breakpoint_value(value)?), nodes),
        _ => None,
    }
}

fn breakpoint_value(value: &Val) -> Option<String> {
    match value {
        Val::Arbitrary(v) => Some(v.clone()),
        Val::Named(n) => breakpoint(n).map(str::to_string),
    }
}

fn order_of(v: &Variant) -> u32 {
    let root = match v {
        Variant::Static(r) => r,
        Variant::Functional { root, .. } | Variant::Compound { root, .. } => root,
        Variant::Arbitrary { .. } => return u32::MAX,
    };
    registered(root).map(|(_, _, o)| o).unwrap_or(u32::MAX - 1)
}

fn root_of(v: &Variant) -> &str {
    match v {
        Variant::Static(r) => r,
        Variant::Functional { root, .. } | Variant::Compound { root, .. } => root,
        Variant::Arbitrary { selector, .. } => selector,
    }
}

/// Breakpoint-Wert einer Variante für den Vergleich innerhalb der Gruppe.
fn breakpoint_of(v: &Variant) -> Option<String> {
    match v {
        Variant::Static(name) => breakpoint(name).map(str::to_string),
        Variant::Functional { value: Some(val), modifier: None, .. } => breakpoint_value(val).filter(|b| !b.contains("var(")),
        _ => None,
    }
}

fn compare_breakpoints(a: &str, z: &str, asc: bool) -> Ordering {
    if a == z {
        return Ordering::Equal;
    }
    let bucket = |s: &str| match s.find('(') {
        Some(i) => s[..i].to_string(),
        None => s.chars().filter(|c| !(c.is_ascii_digit() || *c == '.')).collect(),
    };
    let int = |s: &str| s.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<i64>().ok();
    let (ab, zb) = (bucket(a), bucket(z));
    if ab != zb {
        return ab.cmp(&zb);
    }
    match (int(a), int(z)) {
        (Some(x), Some(y)) if x != y => {
            if asc { x.cmp(&y) } else { y.cmp(&x) }
        }
        (Some(_), Some(_)) => Ordering::Equal,
        _ => a.cmp(z),
    }
}

/// Reihenfolge zweier Varianten (wie Tailwinds `variants.compare`).
pub fn compare(a: &Variant, z: &Variant) -> Ordering {
    if a == z {
        return Ordering::Equal;
    }
    match (a, z) {
        (Variant::Arbitrary { selector: x, .. }, Variant::Arbitrary { selector: y, .. }) => return x.cmp(y),
        (Variant::Arbitrary { .. }, _) => return Ordering::Greater,
        (_, Variant::Arbitrary { .. }) => return Ordering::Less,
        _ => {}
    }
    let (oa, oz) = (order_of(a), order_of(z));
    if oa != oz {
        return oa.cmp(&oz);
    }
    if let (Variant::Compound { inner: ia, modifier: ma, .. }, Variant::Compound { inner: iz, modifier: mz, .. }) = (a, z) {
        let o = compare(ia, iz);
        if o != Ordering::Equal {
            return o;
        }
        return match (ma, mz) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => Ordering::Greater,
            (None, Some(_)) => Ordering::Less,
            (None, None) => Ordering::Equal,
        };
    }
    if oa == MAX_GROUP || oa == MIN_GROUP {
        let asc = oa == MIN_GROUP;
        return match (breakpoint_of(a), breakpoint_of(z)) {
            (None, _) => if asc { Ordering::Less } else { Ordering::Greater },
            (_, None) => if asc { Ordering::Greater } else { Ordering::Less },
            (Some(x), Some(y)) => compare_breakpoints(&x, &y, asc),
        };
    }
    if root_of(a) != root_of(z) {
        return root_of(a).cmp(root_of(z));
    }
    let value = |v: &Variant| match v {
        Variant::Functional { value, .. } => value.clone(),
        _ => None,
    };
    match (value(a), value(z)) {
        (None, _) => Ordering::Less,
        (_, None) => Ordering::Greater,
        (Some(Val::Arbitrary(_)), Some(Val::Named(_))) => Ordering::Greater,
        (Some(Val::Named(_)), Some(Val::Arbitrary(_))) => Ordering::Less,
        (Some(x), Some(y)) => x.text().cmp(y.text()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::{flatten, print};

    fn css(variants: &[&str]) -> String {
        let mut nodes = vec![d("opacity", "1")];
        for v in variants.iter().rev() {
            nodes = apply(&parse(v).expect(v), nodes).expect(v);
        }
        let mut flats = Vec::new();
        flatten(".x", &nodes, &mut Vec::new(), &mut flats);
        let mut out = String::new();
        print(&flats, &mut out);
        out
    }

    #[test]
    fn selektoren() {
        assert_eq!(css(&["hover"]), "@media (hover: hover){.x:hover{opacity:1}}");
        assert_eq!(css(&["group-hover/card"]), "@media (hover: hover){.x:is(:where(.group\\/card):hover *){opacity:1}}");
        assert_eq!(css(&["has-[:checked]", "after"]), ".x:has(*:is(:checked))::after{content:var(--tw-content);opacity:1}");
        assert_eq!(css(&["has-[>svg]"]), ".x:has(>svg){opacity:1}");
        assert_eq!(css(&["data-[state=open]"]), ".x[data-state=\"open\"]{opacity:1}");
        assert_eq!(css(&["[[data-size=sm]_&]"]), "[data-size=sm] .x{opacity:1}");
        assert_eq!(css(&["[.border-b]"]), ".x:is(.border-b){opacity:1}");
        assert_eq!(css(&["[@media(hover:none)]"]), "@media (hover:none){.x{opacity:1}}");
        assert_eq!(css(&["max-lg"]), "@media (width < 64rem){.x{opacity:1}}");
        assert_eq!(css(&["peer-disabled"]), ".x:is(:where(.peer):disabled ~ *){opacity:1}");
        assert_eq!(css(&["*"]), ":is(.x > *){opacity:1}");
    }

    #[test]
    fn reihenfolge() {
        let p = |s: &str| parse(s).unwrap();
        assert_eq!(compare(&p("hover"), &p("focus")), Ordering::Less);
        assert_eq!(compare(&p("max-lg"), &p("max-sm")), Ordering::Less);
        assert_eq!(compare(&p("sm"), &p("md")), Ordering::Less);
        assert_eq!(compare(&p("data-inset"), &p("data-[a=b]")), Ordering::Less);
        assert_eq!(compare(&p("dark"), &p("[&_svg]")), Ordering::Less);
        assert_eq!(compare(&p("group-hover"), &p("group-hover/card")), Ordering::Less);
    }
}
