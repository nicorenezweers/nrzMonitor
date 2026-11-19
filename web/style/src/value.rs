//! Werte in Klassen: `[…]`-Werte entschlüsseln, Datentyp erkennen, Farben mit Deckkraft,
//! Schattenfarben. Verhalten wie bei Tailwind CSS v4.1 (MIT-Lizenz, siehe `LICENSE-tailwindcss`).

use crate::css::split_top;
use crate::theme;

const MATH_FUNCTIONS: &[&str] = &[
    "calc", "min", "max", "clamp", "mod", "rem", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "pow", "sqrt",
    "hypot", "log", "exp", "round",
];

/// `[…]`-Wert entschlüsseln: `_` wird Leerzeichen (ausser in `url()` und im ersten Argument von
/// `var()`/`theme()`), in Rechenfunktionen kommen Leerzeichen um die Operatoren.
pub fn decode_arbitrary(input: &str) -> String {
    let s = underscores(input);
    if !input.contains('(') {
        return s;
    }
    add_math_spaces(&s)
}

fn underscores(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    // Stapel offener Funktionen: (Name, Zeichen seit Beginn des ersten Arguments, erstes Argument?)
    let mut stack: Vec<(String, bool)> = Vec::new();
    let mut word = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let keep = stack.last().is_some_and(|(name, first)| {
            name == "url" || name.ends_with("_url") || (*first && (name == "var" || name == "theme" || name.ends_with("_var")))
        });
        match c {
            '\\' if chars.get(i + 1) == Some(&'_') => {
                out.push('_');
                word.push('_');
                i += 2;
                continue;
            }
            '(' => {
                stack.push((word.clone(), true));
                word.clear();
                out.push(c);
            }
            ')' => {
                stack.pop();
                word.clear();
                out.push(c);
            }
            ',' => {
                if let Some(top) = stack.last_mut() {
                    top.1 = false;
                }
                word.clear();
                out.push(c);
            }
            '_' if !keep => {
                out.push(' ');
                word.clear();
            }
            _ => {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    word.push(c);
                } else {
                    word.clear();
                }
                out.push(c);
            }
        }
        i += 1;
    }
    out
}

/// Leerzeichen um `+ - * /` in Rechenfunktionen (`calc(100%-2rem)` → `calc(100% - 2rem)`).
fn add_math_spaces(input: &str) -> String {
    if !MATH_FUNCTIONS.iter().any(|f| input.contains(f)) {
        return input.to_string();
    }
    let b: Vec<char> = input.chars().collect();
    let mut result = String::with_capacity(input.len() + 8);
    let mut formattable: Vec<bool> = Vec::new();
    let is_op = |c: char| matches!(c, '+' | '-' | '*' | '/');
    for i in 0..b.len() {
        let c = b[i];
        let top = formattable.first().copied().unwrap_or(false);
        if c == '(' {
            result.push(c);
            let mut start = i;
            while start > 0 && (b[start - 1].is_ascii_digit() || b[start - 1].is_ascii_lowercase()) {
                start -= 1;
            }
            let name: String = b[start..i].iter().collect();
            if MATH_FUNCTIONS.contains(&name.as_str()) || (top && name.is_empty()) {
                formattable.insert(0, true);
            } else {
                formattable.insert(0, false);
            }
        } else if c == ')' {
            result.push(c);
            if !formattable.is_empty() {
                formattable.remove(0);
            }
        } else if c == ',' && top {
            result.push_str(", ");
        } else if c == ' ' && top && result.ends_with(' ') {
            continue;
        } else if is_op(c) && top {
            let trimmed = result.trim_end();
            let mut it = trimmed.chars().rev();
            let prev = it.next().unwrap_or('\0');
            let prev_prev = it.next().unwrap_or('\0');
            let next = b.get(i + 1).copied().unwrap_or('\0');
            // Exponent (`-3.4e-2`), nach einem Operator oder am Anfang eines Arguments: unverändert
            let exponent = (prev == 'e' || prev == 'E') && prev_prev.is_ascii_digit();
            if exponent || is_op(prev) || prev == '(' || prev == ',' {
                result.push(c);
            } else if i > 0 && b[i - 1] == ' ' {
                result.push(c);
                result.push(' ');
            } else if prev.is_ascii_digit() || next.is_ascii_digit() || prev == ')' || next == '(' || is_op(next) {
                result.push(' ');
                result.push(c);
                result.push(' ');
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Ungültige `[…]`-Werte verwerfen: unausgeglichene Klammern, Platzhalter aus Rust-Formatstrings.
pub fn valid_arbitrary(v: &str) -> bool {
    if v.trim().is_empty() || v.contains('{') || v.contains('}') || v.contains(';') {
        return false;
    }
    let mut stack = Vec::new();
    let mut quote = None;
    for c in v.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' => stack.push(c),
            ')' if stack.pop() != Some('(') => return false,
            ']' if stack.pop() != Some('[') => return false,
            _ => {}
        }
    }
    stack.is_empty() && quote.is_none()
}

const LENGTH_UNITS: &[&str] = &[
    "cm", "mm", "Q", "in", "pc", "pt", "px", "em", "ex", "ch", "rem", "lh", "rlh", "vw", "vh", "vmin", "vmax", "vb", "vi",
    "svw", "svh", "lvw", "lvh", "dvw", "dvh", "cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax",
];

fn split_number(v: &str) -> Option<(&str, &str)> {
    let v = v.strip_prefix(['+', '-']).unwrap_or(v);
    let end = v.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(v.len());
    let (num, unit) = v.split_at(end);
    (!num.is_empty() && num.chars().any(|c| c.is_ascii_digit()) && num.matches('.').count() <= 1).then_some((num, unit))
}

pub fn has_math_fn(v: &str) -> bool {
    v.contains('(') && MATH_FUNCTIONS.iter().any(|f| v.contains(&format!("{f}(")))
}

pub fn is_number(v: &str) -> bool {
    split_number(v).is_some_and(|(_, u)| u.is_empty()) || has_math_fn(v)
}

pub fn is_length(v: &str) -> bool {
    v == "0" || has_math_fn(v) || split_number(v).is_some_and(|(_, u)| LENGTH_UNITS.contains(&u))
}

pub fn is_percentage(v: &str) -> bool {
    split_number(v).is_some_and(|(_, u)| u == "%") || has_math_fn(v)
}

const NAMED_COLORS: &[&str] = &[
    "transparent", "currentcolor", "black", "white", "red", "green", "blue", "yellow", "orange", "purple", "gray", "grey",
    "pink", "brown", "cyan", "magenta", "lime", "navy", "teal", "silver", "maroon", "olive", "aqua", "fuchsia", "gold",
];

pub fn is_color(v: &str) -> bool {
    let l = v.to_ascii_lowercase();
    (l.starts_with('#') && l.len() > 1 && l[1..].chars().all(|c| c.is_ascii_hexdigit()))
        || ["rgb(", "rgba(", "hsl(", "hsla(", "hwb(", "lab(", "lch(", "oklab(", "oklch(", "color(", "color-mix(", "light-dark("]
            .iter()
            .any(|f| l.starts_with(f))
        || NAMED_COLORS.contains(&l.as_str())
}

/// Datentyp eines `[…]`-Werts erkennen (wie `inferDataType`); `var(…)` bleibt offen.
pub fn infer(v: &str, types: &[&str]) -> Option<&'static str> {
    if v.starts_with("var(") {
        return None;
    }
    for t in types {
        let ok = match *t {
            "color" => is_color(v),
            "length" => is_length(v),
            "percentage" => is_percentage(v),
            "number" => is_number(v),
            "absolute-size" => {
                ["xx-small", "x-small", "small", "medium", "large", "x-large", "xx-large", "xxx-large"].contains(&v)
            }
            "relative-size" => v == "larger" || v == "smaller",
            "url" => v.starts_with("url("),
            "image" => v.starts_with("url(") || v.contains("gradient("),
            "generic-name" => ["serif", "sans-serif", "monospace", "cursive", "fantasy", "system-ui", "ui-serif", "ui-sans-serif", "ui-monospace", "ui-rounded", "math", "emoji", "fangsong"].contains(&v),
            "family-name" => v.contains(',') || v.starts_with('"') || v.starts_with('\''),
            _ => false,
        };
        if ok {
            return Some(match *t {
                "color" => "color",
                "length" => "length",
                "percentage" => "percentage",
                "number" => "number",
                "absolute-size" => "absolute-size",
                "relative-size" => "relative-size",
                "url" => "url",
                "image" => "image",
                "generic-name" => "generic-name",
                _ => "family-name",
            });
        }
    }
    None
}

/// Deckkraft auf eine Farbe anwenden (`color-mix`), wie Tailwinds `withAlpha`.
pub fn with_alpha(color: &str, alpha: &str) -> String {
    let alpha = match alpha.parse::<f64>() {
        Ok(n) => format!("{}%", js_number(n * 100.0)),
        Err(_) => alpha.to_string(),
    };
    if alpha == "100%" {
        return color.to_string();
    }
    format!("color-mix(in oklab, {color} {alpha}, transparent)")
}

/// Zahl so ausgeben wie JavaScript (`7.000000000000001`, `50`, `0.5`).
pub fn js_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e21 { format!("{}", n as i64) } else { format!("{n}") }
}

/// `theme(colors.border)` in `[…]`-Werten durch den Thema-Wert ersetzen.
pub fn substitute_theme(v: &str) -> String {
    let mut out = v.to_string();
    while let Some(start) = out.find("theme(") {
        let Some(len) = out[start..].find(')') else { break };
        let inner = out[start + 6..start + len].trim().to_string();
        let path = inner.split(',').next().unwrap_or("").trim().trim_matches(|c| c == '"' || c == '\'');
        let name = if path.starts_with("--") {
            path.to_string()
        } else {
            let mut parts = path.split('.');
            let ns = match parts.next().unwrap_or("") {
                "colors" => "--color",
                "spacing" => "--spacing",
                "borderRadius" => "--radius",
                "fontSize" => "--text",
                other => other,
            };
            let rest: Vec<&str> = parts.collect();
            if rest.is_empty() { ns.to_string() } else { format!("{ns}-{}", rest.join("-")) }
        };
        let value = theme::var(&name).unwrap_or_else(|| format!("var({name})"));
        out.replace_range(start..start + len + 1, &value);
    }
    out
}

/// Farbe in einem Schatten durch `var(--tw-shadow-color, farbe)` ersetzen (wie `replaceShadowColors`).
pub fn replace_shadow_colors(input: &str, wrap: &dyn Fn(&str) -> String) -> String {
    split_top(input, ',')
        .into_iter()
        .map(|shadow| {
            let shadow = shadow.trim();
            let parts: Vec<&str> = split_top(shadow, ' ').into_iter().filter(|p| !p.trim().is_empty()).collect();
            let (mut color, mut x, mut y) = (None, None, None);
            for part in parts {
                if ["inset", "inherit", "initial", "revert", "unset"].contains(&part) {
                    continue;
                }
                let starts_length = {
                    let p = part.strip_prefix('-').unwrap_or(part);
                    p.starts_with(|c: char| c.is_ascii_digit()) || (p.starts_with('.') && p[1..].starts_with(|c: char| c.is_ascii_digit()))
                };
                if starts_length {
                    if x.is_none() {
                        x = Some(part);
                    } else if y.is_none() {
                        y = Some(part);
                    }
                } else if color.is_none() {
                    color = Some(part);
                }
            }
            if x.is_none() || y.is_none() {
                return shadow.to_string();
            }
            match color {
                Some(c) => shadow.replacen(c, &wrap(c), 1),
                None => format!("{shadow} {}", wrap("currentcolor")),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `state=open` → `state="open"` (Attributwerte in Varianten wie `data-[…]`).
pub fn quote_attribute_value(input: &str) -> String {
    let Some(eq) = input.find('=') else { return input.to_string() };
    let (name, rest) = input.split_at(eq);
    let value = &rest[1..];
    if value.starts_with('"') || value.starts_with('\'') {
        return input.to_string();
    }
    let b = value.as_bytes();
    if b.len() > 2 && b[b.len() - 2] == b' ' && matches!(b[b.len() - 1], b'i' | b'I' | b's' | b'S') {
        return format!("{name}=\"{}\" {}", &value[..value.len() - 2], &value[value.len() - 1..]);
    }
    format!("{name}=\"{value}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entschluesseln() {
        assert_eq!(decode_arbitrary("calc(100%-2rem)"), "calc(100% - 2rem)");
        assert_eq!(decode_arbitrary("calc(100%+4px)"), "calc(100% + 4px)");
        assert_eq!(decode_arbitrary("1.5rem_minmax(0,1fr)_auto"), "1.5rem minmax(0,1fr) auto");
        assert_eq!(decode_arbitrary("cubic-bezier(0.68,-0.55,0.265,1.55)"), "cubic-bezier(0.68,-0.55,0.265,1.55)");
        assert_eq!(decode_arbitrary("inset_0_0_0_1px_theme(colors.border)"), "inset 0 0 0 1px theme(colors.border)");
        assert_eq!(decode_arbitrary("var(--my_var)"), "var(--my_var)");
        assert_eq!(decode_arbitrary("&_svg:not([class*='size-'])"), "& svg:not([class*='size-'])");
    }

    #[test]
    fn deckkraft() {
        assert_eq!(with_alpha("var(--card)", "50%"), "color-mix(in oklab, var(--card) 50%, transparent)");
        assert_eq!(with_alpha("var(--card)", "0.07"), "color-mix(in oklab, var(--card) 7.000000000000001%, transparent)");
        assert_eq!(with_alpha("var(--card)", "100%"), "var(--card)");
    }

    #[test]
    fn schatten() {
        let w = |c: &str| format!("var(--tw-shadow-color, {c})");
        assert_eq!(replace_shadow_colors("inset 0 0 0 1px var(--border)", &w), "inset 0 0 0 1px var(--tw-shadow-color, var(--border))");
        assert_eq!(
            replace_shadow_colors("0 1px 3px 0 rgb(0 0 0 / 0.1), 0 1px 2px -1px rgb(0 0 0 / 0.1)", &w),
            "0 1px 3px 0 var(--tw-shadow-color, rgb(0 0 0 / 0.1)), 0 1px 2px -1px var(--tw-shadow-color, rgb(0 0 0 / 0.1))"
        );
    }

    #[test]
    fn thema_funktion() {
        assert_eq!(substitute_theme("inset 0 0 0 1px theme(colors.border)"), "inset 0 0 0 1px var(--border)");
    }
}
