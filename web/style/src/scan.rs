//! Klassen im Quelltext finden – wie der Scanner von Tailwind: jede zusammenhängende Zeichenfolge
//! ist ein möglicher Kandidat (auch in Kommentaren). Was keine bekannte Klasse ist, fällt später weg.

use std::collections::BTreeSet;

/// Mögliche Klassen und benutzte CSS-Variablen (`--name`) eines Quelltexts sammeln.
pub fn scan(src: &str, candidates: &mut BTreeSet<String>, vars: &mut BTreeSet<String>) {
    tokenize(src, candidates, vars);
    // Inhalt jeder Zeichenkette noch einmal für sich (findet z. B. `flex` in `["flex"]`)
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            let start = i + 1;
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            tokenize(&src[start..i.min(b.len())], candidates, vars);
        }
        i += 1;
    }
}

fn tokenize(src: &str, candidates: &mut BTreeSet<String>, vars: &mut BTreeSet<String>) {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !starts_candidate(b[i]) {
            i += 1;
            continue;
        }
        let start = i;
        let (mut depth, mut quote) = (0i32, None::<u8>);
        while i < b.len() {
            let c = b[i];
            if let Some(q) = quote {
                if c == b'\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                } else if c == b'\n' {
                    break;
                }
                i += 1;
                continue;
            }
            if depth > 0 {
                match c {
                    b'[' | b'(' => depth += 1,
                    b']' | b')' => depth -= 1,
                    b'\'' | b'"' => quote = Some(c),
                    b'\\' => i += 1,
                    _ if c.is_ascii_whitespace() => break,
                    _ => {}
                }
            } else {
                match c {
                    b'[' => depth += 1,
                    _ if c.is_ascii_whitespace() || is_delimiter(c) => break,
                    _ => {}
                }
            }
            i += 1;
        }
        if depth != 0 || quote.is_some() {
            // Unvollständig (z. B. `vec![1,`): ab dem nächsten Zeichen neu suchen
            i = start + 1;
            continue;
        }
        let token = &src[start..i.min(b.len())];
        // Direkt gefolgt von `(`: Funktionsaufruf wie `!table()`, keine Klasse
        let call = b.get(i) == Some(&b'(');
        if token.starts_with("--") {
            let name: String = token.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
            if name.len() > 2 {
                vars.insert(name);
            }
        } else if !call && !token.is_empty() && !token.ends_with(':') {
            candidates.insert(token.to_string());
        }
    }
}

fn starts_candidate(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'!' | b'@' | b'*' | b'[')
}

/// Zeichen, die ausserhalb von `[…]` einen Kandidaten beenden.
fn is_delimiter(c: u8) -> bool {
    matches!(
        c,
        b'"' | b'\'' | b'`' | b'{' | b'}' | b'(' | b')' | b']' | b';' | b',' | b'=' | b'<' | b'>' | b'\\' | b'#' | b'$' | b'|' | b'^' | b'?' | b'&' | b'+' | b'~'
    )
}

/// Klassen in `class: "…"`-Angaben (für Warnungen bei unbekannten Klassen).
pub fn class_attributes(src: &str, out: &mut BTreeSet<String>) {
    let mut rest = src;
    while let Some(pos) = rest.find("class: \"") {
        rest = &rest[pos + 8..];
        let Some(end) = rest.find('"') else { break };
        for token in rest[..end].split_whitespace() {
            if !token.contains('{') && !token.contains('}') {
                out.insert(token.to_string());
            }
        }
        rest = &rest[end..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &str) -> (Vec<String>, Vec<String>) {
        let (mut c, mut v) = (BTreeSet::new(), BTreeSet::new());
        scan(src, &mut c, &mut v);
        (c.into_iter().collect(), v.into_iter().collect())
    }

    #[test]
    fn klassen_aus_rust() {
        let (c, _) = tokens(r#"div { class: "flex {x} sm:grid-cols-[1fr_auto] [&_svg:not([class*='size-'])]:size-4", Some("p-2") }"#);
        for k in ["flex", "sm:grid-cols-[1fr_auto]", "[&_svg:not([class*='size-'])]:size-4", "p-2", "div", "x"] {
            assert!(c.contains(&k.to_string()), "{k} fehlt in {c:?}");
        }
        assert!(!c.iter().any(|t| t.contains('{')));
    }

    #[test]
    fn variablen() {
        let (_, v) = tokens("background-color: var(--color-muted);");
        assert_eq!(v, vec!["--color-muted".to_string()]);
    }
}
