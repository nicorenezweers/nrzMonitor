//! Stylesheet der nrzMonitor-Oberfläche, komplett in Rust erzeugt.
//!
//! Ersetzt Tailwind CSS: `build.rs` der Oberfläche liest alle Quelltexte unter `web/src`, findet darin
//! Klassen wie `flex`, `gap-4`, `hover:bg-accent` oder `sm:grid-cols-2` und erzeugt genau das CSS dafür,
//! dazu Grundstile und das nrz-Thema. Schreibweise, Bedeutung und Reihenfolge der Regeln folgen
//! Tailwind CSS v4.1 (die Teilmenge, die nrzUI und die Seiten benutzen) – die Bausteine bleiben so,
//! wie sie sind. Unbekannte Klassen in `class: "…"` meldet `build.rs` als Warnung und der Test
//! `alle_klassen_der_oberflaeche_bekannt` als Fehler.
//!
//! Aufbau: `scan` (Kandidaten finden) → `variants` + `utilities` (Regeln bauen) → `order`
//! (sortieren) → `css` (ausgeben). Thema in `theme`, feste Regeln in `base`.

mod base;
mod css;
mod order;
mod scan;
mod theme;
mod utilities;
mod value;
mod variants;

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use css::Node;
use variants::Variant;

/// Ergebnis: fertiges CSS und Klassen aus `class: "…"`, die kein CSS ergeben.
pub struct Output {
    pub css: String,
    pub unknown: Vec<String>,
}

/// Alle `.rs`-Dateien unter `dir` lesen und das Stylesheet erzeugen.
pub fn build(dir: &Path) -> std::io::Result<Output> {
    let mut sources = Vec::new();
    collect(dir, &mut sources)?;
    Ok(generate(&sources))
}

fn collect(dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(std::fs::read_to_string(&path)?);
        }
    }
    Ok(())
}

/// Klassen ohne eigenes CSS: Marker für Varianten oder feste Regeln. `bg-warning-light` und
/// `text-warning-dark` (nrzUI-Badge/Callout, Alarme) kennt das nrz-Thema nicht – sie waren auch mit
/// Tailwind schon ohne Wirkung.
const MARKERS: &[&str] = &["group", "peer", "nrz-plot", "dark", "light", "bg-warning-light", "text-warning-dark"];

struct Compiled {
    raw: String,
    nodes: Vec<Node>,
    variants: Vec<Variant>,
    props: Vec<usize>,
    count: usize,
}

/// Stylesheet für die gegebenen Quelltexte erzeugen.
pub fn generate(sources: &[String]) -> Output {
    let (mut candidates, mut vars, mut attrs) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for src in sources {
        scan::scan(src, &mut candidates, &mut vars);
        scan::class_attributes(src, &mut attrs);
    }
    let compiled: Vec<Compiled> = candidates.iter().filter_map(|raw| compile_candidate(raw)).collect();

    let unknown = attrs
        .into_iter()
        .filter(|t| {
            // `chips__main__container` u. Ä.: Namen für eingebettete Stile der nrzUI-Bausteine
            let marker = MARKERS.contains(&t.as_str()) || t.starts_with("group/") || t.starts_with("peer/") || t.contains("__");
            !marker && compile_candidate(t).is_none()
        })
        .collect();

    // Reihenfolge der Varianten: jede bekommt eine Stelle, gleichwertige teilen sie sich
    let mut unique: Vec<&Variant> = Vec::new();
    for v in compiled.iter().flat_map(|c| &c.variants) {
        if !unique.contains(&v) {
            unique.push(v);
        }
    }
    unique.sort_by(|a, b| variants::compare(a, b));
    let mut ranks = Vec::with_capacity(unique.len());
    for (i, v) in unique.iter().enumerate() {
        let rank = match i {
            0 => 0,
            _ if variants::compare(unique[i - 1], v) == Ordering::Equal => ranks[i - 1],
            _ => ranks[i - 1] + 1,
        };
        ranks.push(rank);
    }
    let rank_of = |v: &Variant| unique.iter().position(|u| *u == v).map(|i| ranks[i]).unwrap_or(0);
    let masks: Vec<Vec<usize>> = compiled
        .iter()
        .map(|c| {
            let mut m: Vec<usize> = c.variants.iter().map(rank_of).collect();
            m.sort_unstable_by(|a, b| b.cmp(a));
            m.dedup();
            m
        })
        .collect();

    let mut sorted: Vec<usize> = (0..compiled.len()).collect();
    sorted.sort_by(|&a, &b| {
        let (ca, cb) = (&compiled[a], &compiled[b]);
        compare_masks(&masks[a], &masks[b])
            .then_with(|| compare_props(&ca.props, &cb.props))
            .then_with(|| cb.count.cmp(&ca.count))
            .then_with(|| order::compare_names(&ca.raw, &cb.raw))
    });

    let mut flats = Vec::new();
    for &i in &sorted {
        let c = &compiled[i];
        css::flatten(&format!(".{}", css::escape(&c.raw)), &strip_sort(&c.nodes), &mut Vec::new(), &mut flats);
    }
    let mut utilities = String::new();
    css::print(&flats, &mut utilities);

    let mut base_css = String::new();
    css::print_static(base::PREFLIGHT, &mut base_css);
    css::print_static(base::NRZ_BASE, &mut base_css);

    // Thema-Variablen, die etwas benutzt (auch über andere Thema-Variablen)
    let mut used: HashSet<String> = names(&utilities).chain(names(&base_css)).collect();
    used.extend(vars);
    let theme_vars = loop {
        let emitted = theme::emitted(&|n| used.contains(n));
        let before = used.len();
        for (_, v) in &emitted {
            used.extend(names(v));
        }
        if used.len() == before {
            break emitted;
        }
    };

    let mut out = String::from("@layer properties;@layer theme,base,components,utilities;");
    out.push_str("@layer theme{:root,:host{");
    out.push_str(&decls(&theme_vars));
    out.push_str("}}@layer base{");
    out.push_str(&base_css);
    out.push_str("}@layer utilities{");
    out.push_str(&utilities);
    out.push('}');

    // nrz-Thema (ausserhalb der Ebenen: gewinnt gegen die Standardwerte)
    out.push_str(":root{");
    out.push_str(&decls(theme::DARK));
    out.push_str("}html.light{");
    out.push_str(&decls(theme::LIGHT));
    out.push('}');
    css::print_static(base::NRZ_KEYFRAMES, &mut out);
    out.push_str(":root{");
    out.push_str(&decls(theme::SERIES_DARK));
    out.push_str("}.light{");
    out.push_str(&decls(theme::SERIES_LIGHT));
    out.push('}');
    css::print_static(base::NRZ_PLOT, &mut out);
    css::print_static(base::NRZ_UI, &mut out);

    // Registrierte Variablen der benutzten Klassen, Animationen des Themas
    let tw: HashSet<String> = names(&utilities).collect();
    let props: Vec<&(&str, &str, Option<&str>)> = base::PROPERTIES.iter().filter(|(n, _, _)| tw.contains(*n)).collect();
    for (name, syntax, initial) in &props {
        out.push_str(&format!("@property {name}{{syntax:\"{syntax}\";inherits:false"));
        if let Some(i) = initial {
            out.push_str(&format!(";initial-value:{i}"));
        }
        out.push('}');
    }
    for (var, keyframes) in base::KEYFRAMES {
        if used.contains(*var) {
            css::print_static(std::slice::from_ref(keyframes), &mut out);
        }
    }
    // Ältere Browser ohne @property: Anfangswerte direkt setzen
    if !props.is_empty() {
        out.push_str("@layer properties{@supports ((-webkit-hyphens: none) and (not (margin-trim: inline))) or ((-moz-orient: inline) and (not (color:rgb(from red r g b)))){*,::before,::after,::backdrop{");
        let initial: Vec<(&str, &str)> = props.iter().map(|(n, _, i)| (*n, i.unwrap_or("initial"))).collect();
        out.push_str(&decls(&initial));
        out.push_str("}}}");
    }

    Output { css: out, unknown }
}

fn compile_candidate(raw: &str) -> Option<Compiled> {
    let segs = css::split_top(raw, ':');
    let (base, rest) = segs.split_last()?;
    let variants: Vec<Variant> = rest.iter().map(|v| variants::parse(v)).collect::<Option<_>>()?;
    let nodes = utilities::compile(base)?;
    let (props, count) = property_sort(&nodes);
    let mut applied = nodes;
    for v in variants.iter().rev() {
        applied = variants::apply(v, applied)?;
    }
    Some(Compiled { raw: raw.to_string(), nodes: applied, variants, props, count })
}

/// Eigenschaften einer Klasse für die Sortierung (wie Tailwinds `getPropertySort`).
fn property_sort(nodes: &[Node]) -> (Vec<usize>, usize) {
    let mut order = BTreeSet::new();
    let (mut count, mut seen_sort) = (0, false);
    let mut queue: std::collections::VecDeque<&Node> = nodes.iter().collect();
    while let Some(n) = queue.pop_front() {
        match n {
            Node::Decl(p, v) => {
                count += 1;
                if seen_sort {
                    continue;
                }
                if p == "--tw-sort"
                    && let Some(i) = order::index(v) {
                        order.insert(i);
                        seen_sort = true;
                        continue;
                    }
                if let Some(i) = order::index(p) {
                    order.insert(i);
                }
            }
            Node::Rule(_, kids) | Node::At(_, kids) => queue.extend(kids),
        }
    }
    (order.into_iter().collect(), count)
}

fn compare_masks(a: &[usize], b: &[usize]) -> Ordering {
    for i in 0..a.len().max(b.len()) {
        match (a.get(i), b.get(i)) {
            (Some(x), Some(y)) if x != y => return x.cmp(y),
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            _ => {}
        }
    }
    Ordering::Equal
}

fn compare_props(a: &[usize], b: &[usize]) -> Ordering {
    let i = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    a.get(i).copied().unwrap_or(usize::MAX).cmp(&b.get(i).copied().unwrap_or(usize::MAX))
}

fn strip_sort(nodes: &[Node]) -> Vec<Node> {
    nodes
        .iter()
        .filter(|n| !matches!(n, Node::Decl(p, _) if p == "--tw-sort"))
        .map(|n| match n {
            Node::Rule(s, kids) => Node::Rule(s.clone(), strip_sort(kids)),
            Node::At(s, kids) => Node::At(s.clone(), strip_sort(kids)),
            other => other.clone(),
        })
        .collect()
}

fn decls(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(p, v)| format!("{p}:{v}")).collect::<Vec<_>>().join(";")
}

/// Alle `--namen` in einem Text.
fn names(text: &str) -> impl Iterator<Item = String> + '_ {
    let b = text.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i + 1 < b.len() {
            if b[i] == b'-' && b[i + 1] == b'-' && (i == 0 || !is_name(b[i - 1])) {
                let start = i;
                i += 2;
                while i < b.len() && is_name(b[i]) {
                    i += 1;
                }
                if i > start + 2 {
                    return Some(text[start..i].to_string());
                }
            } else {
                i += 1;
            }
        }
        None
    })
}

fn is_name(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reihenfolge_wie_tailwind() {
        let out = generate(&["px-4 pl-2 hover:bg-accent sm:p-2 p-1 focus:bg-muted".to_string()]).css;
        let pos = |s: &str| out.find(s).unwrap_or_else(|| panic!("{s} fehlt"));
        assert!(pos(".p-1{") < pos(".px-4{"));
        assert!(pos(".px-4{") < pos(".pl-2{"));
        assert!(pos(".hover\\:bg-accent:hover") < pos(".focus\\:bg-muted:focus"));
        assert!(pos(".focus\\:bg-muted:focus") < pos(".sm\\:p-2{"));
    }

    #[test]
    fn thema_und_variablen() {
        let out = generate(&["text-sm bg-black/50 animate-spin var(--color-muted)".to_string()]).css;
        assert!(out.contains("--text-sm:0.875rem"));
        assert!(out.contains("--text-sm--line-height:calc(1.25 / 0.875)"));
        assert!(out.contains("--color-black:#000"));
        assert!(out.contains("--color-muted:var(--muted)"));
        assert!(out.contains("@keyframes spin"));
        assert!(!out.contains("--font-sans:var(--font-sans)"));
    }

    /// Jede Klasse in `class: "…"` der Oberfläche muss CSS ergeben (dx zeigt Build-Warnungen nicht an).
    #[test]
    fn alle_klassen_der_oberflaeche_bekannt() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src");
        let out = build(&src).expect("web/src nicht lesbar");
        assert!(out.unknown.is_empty(), "Klassen ohne CSS – in utilities.rs ergänzen: {:?}", out.unknown);
        assert!(out.css.contains(".flex{display:flex}"));
    }

    #[test]
    fn unbekannte_klassen() {
        let out = generate(&[r#"div { class: "flex group nrz-plot gibtsnicht-4 {x}" }"#.to_string()]);
        assert_eq!(out.unknown, vec!["gibtsnicht-4".to_string()]);
    }
}
