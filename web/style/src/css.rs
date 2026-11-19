//! Baumstruktur des Stylesheets (wie bei Tailwind: Deklaration, Regel, At-Regel) und Ausgabe als
//! flaches, kompaktes CSS. Verschachtelte Regeln (`&:hover`) werden beim Ausgeben aufgelöst.

use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// Eigenschaft, Wert
    Decl(String, String),
    /// Selektor (`&` steht für die Elternregel), Inhalt
    Rule(String, Vec<Node>),
    /// z. B. `@media (width >= 40rem)`, Inhalt
    At(String, Vec<Node>),
}

pub fn d(p: &str, v: impl Into<String>) -> Node {
    Node::Decl(p.to_string(), v.into())
}

pub fn rule(sel: impl Into<String>, nodes: Vec<Node>) -> Node {
    Node::Rule(sel.into(), nodes)
}

pub fn at(name: impl Into<String>, nodes: Vec<Node>) -> Node {
    Node::At(name.into(), nodes)
}

/// Feste Stilregeln als Rust-Daten (Grundstile, Thema, Animationen).
pub enum S {
    /// Selektor, Deklarationen
    R(&'static str, &'static [(&'static str, &'static str)]),
    /// At-Regel, Inhalt
    A(&'static str, &'static [S]),
}

impl S {
    pub fn to_node(&self) -> Node {
        match self {
            S::R(sel, decls) => rule(*sel, decls.iter().map(|(p, v)| d(p, *v)).collect()),
            S::A(name, kids) => at(*name, kids.iter().map(S::to_node).collect()),
        }
    }
}

/// `CSS.escape` für Klassennamen (`2xl:hidden` → `\32 xl\:hidden`).
pub fn escape(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        let code = c as u32;
        if code == 0 {
            out.push('\u{FFFD}');
        } else if (1..=0x1f).contains(&code)
            || code == 0x7f
            || (i == 0 && c.is_ascii_digit())
            || (i == 1 && c.is_ascii_digit() && chars[0] == '-')
        {
            let _ = write!(out, "\\{code:x} ");
        } else if i == 0 && c == '-' && chars.len() == 1 {
            out.push_str("\\-");
        } else if code >= 0x80 || c == '-' || c == '_' || c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push('\\');
            out.push(c);
        }
    }
    out
}

/// Text an `sep` teilen, aber nicht innerhalb von Klammern oder Anführungszeichen.
pub fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quote, mut start, mut esc) = (0i32, None::<char>, 0usize, false);
    for (i, c) in s.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        if c == '\\' {
            esc = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ if c == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Eine flache Regel: umgebende At-Regeln, Selektor, Deklarationen.
pub struct Flat {
    pub ats: Vec<String>,
    pub sel: String,
    pub decls: Vec<(String, String)>,
}

/// Verschachtelten Inhalt einer Regel in flache Regeln auflösen.
pub fn flatten(sel: &str, nodes: &[Node], ats: &mut Vec<String>, out: &mut Vec<Flat>) {
    let mut decls = Vec::new();
    for n in nodes {
        match n {
            Node::Decl(p, v) => decls.push((p.clone(), v.clone())),
            Node::Rule(s, kids) => {
                push(out, ats, sel, &mut decls);
                flatten(&nest(sel, s), kids, ats, out);
            }
            Node::At(name, kids) => {
                push(out, ats, sel, &mut decls);
                ats.push(name.clone());
                flatten(sel, kids, ats, out);
                ats.pop();
            }
        }
    }
    push(out, ats, sel, &mut decls);
}

fn push(out: &mut Vec<Flat>, ats: &[String], sel: &str, decls: &mut Vec<(String, String)>) {
    if !decls.is_empty() {
        out.push(Flat { ats: ats.to_vec(), sel: sel.to_string(), decls: std::mem::take(decls) });
    }
}

/// `&` im Kind-Selektor durch den Eltern-Selektor ersetzen (wie CSS-Verschachtelung).
fn nest(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        return child.to_string();
    }
    let p = if split_top(parent, ',').len() > 1 { format!(":is({parent})") } else { parent.to_string() };
    if child.contains('&') { child.replace('&', &p) } else { format!("{p} {child}") }
}

/// Flache Regeln kompakt ausgeben; aufeinanderfolgende Regeln teilen sich ihre At-Regeln.
pub fn print(flats: &[Flat], out: &mut String) {
    let mut open: Vec<&str> = Vec::new();
    for f in flats {
        let common = open.iter().zip(&f.ats).take_while(|(a, b)| **a == b.as_str()).count();
        while open.len() > common {
            out.push('}');
            open.pop();
        }
        for a in &f.ats[common..] {
            out.push_str(a);
            out.push('{');
            open.push(a);
        }
        out.push_str(&f.sel);
        out.push('{');
        for (i, (p, v)) in f.decls.iter().enumerate() {
            if i > 0 {
                out.push(';');
            }
            out.push_str(p);
            out.push(':');
            out.push_str(v);
        }
        out.push('}');
    }
    for _ in open {
        out.push('}');
    }
}

/// Feste Regeln ausgeben (ohne Elternregel), z. B. Grundstile oder `@keyframes`.
pub fn print_static(items: &[S], out: &mut String) {
    let nodes: Vec<Node> = items.iter().map(S::to_node).collect();
    let mut flats = Vec::new();
    flatten("", &nodes, &mut Vec::new(), &mut flats);
    print(&flats, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_wie_css_escape() {
        assert_eq!(escape("2xl:hidden"), "\\32 xl\\:hidden");
        assert_eq!(escape("gap-1.5"), "gap-1\\.5");
        assert_eq!(escape("[&_svg]:size-4"), "\\[\\&_svg\\]\\:size-4");
        assert_eq!(escape("w-1/2"), "w-1\\/2");
    }

    #[test]
    fn verschachtelung_aufloesen() {
        let n = vec![rule("&:hover", vec![at("@media (hover: hover)", vec![d("color", "red")])])];
        let mut flats = Vec::new();
        flatten(".a", &n, &mut Vec::new(), &mut flats);
        let mut out = String::new();
        print(&flats, &mut out);
        assert_eq!(out, "@media (hover: hover){.a:hover{color:red}}");
    }
}
