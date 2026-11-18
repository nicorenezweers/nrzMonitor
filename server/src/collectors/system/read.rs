//! Sparsame Lese-Helfer für `/proc` und `/sys`: wiederverwendete Puffer, keine Zuteilung je Wert.

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

/// Datei in einen wiederverwendeten Puffer lesen. `false`, wenn sie fehlt oder nicht lesbar ist.
pub fn into(buf: &mut String, path: &Path) -> bool {
    buf.clear();
    match File::open(path) {
        Ok(mut f) => f.read_to_string(buf).is_ok(),
        Err(_) => false,
    }
}

/// Datei als getrimmter Text (für einmalige, kleine Dateien).
pub fn text(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Erste Zahl einer Datei (`/sys/…/temp1_input`).
pub fn num<T: std::str::FromStr>(buf: &mut String, path: &Path) -> Option<T> {
    if !into(buf, path) {
        return None;
    }
    buf.trim().parse().ok()
}

/// Ganzzahl einer kleinen Datei ohne Puffer.
pub fn int(path: impl AsRef<Path>) -> Option<i64> {
    text(path)?.parse().ok()
}

/// Zahlen einer Zeile nach dem ersten Feld: `cpu  1 2 3` → [1, 2, 3].
pub fn fields_u64(line: &str, out: &mut Vec<u64>) {
    out.clear();
    out.extend(line.split_ascii_whitespace().skip(1).map(|v| v.parse::<u64>().unwrap_or(0)));
}

/// `Schlüssel: Wert kB`-Dateien (meminfo, vmstat, status): Wert zum Schlüssel, Einheit ignoriert.
pub fn kv<'a>(text: &'a str) -> impl Iterator<Item = (&'a str, u64)> + 'a {
    text.lines().filter_map(|l| {
        let mut it = l.split_ascii_whitespace();
        let key = it.next()?.trim_end_matches(':');
        let v = it.next()?.parse().ok()?;
        Some((key, v))
    })
}

/// Tabellen wie `/proc/net/snmp`: Kopfzeile `Tcp: A B C` gefolgt von Wertezeile `Tcp: 1 2 3`.
/// Liefert (Präfix, Name, Wert).
pub fn header_table(text: &str, mut f: impl FnMut(&str, &str, u64)) {
    let mut lines = text.lines();
    while let (Some(h), Some(v)) = (lines.next(), lines.next()) {
        let mut hs = h.split_ascii_whitespace();
        let mut vs = v.split_ascii_whitespace();
        let (Some(prefix), Some(_)) = (hs.next(), vs.next()) else { continue };
        let prefix = prefix.trim_end_matches(':');
        for (k, val) in hs.zip(vs) {
            // Negative Werte (MaxConn = -1) → 0.
            f(prefix, k, val.parse().unwrap_or(0));
        }
    }
}

/// Einträge eines Ordners (Dateinamen), sortiert. Fehlt der Ordner: leer.
pub fn dir(path: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(path)
        .map(|rd| rd.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    v.sort_by(|a, b| natural(a, b));
    v
}

/// Natürliche Sortierung: `cpu2` vor `cpu10`.
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| {
        let i = s.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        (s[..i].to_string(), s[i..].parse::<u64>().unwrap_or(0))
    };
    split(a).cmp(&split(b)).then_with(|| a.cmp(b))
}

/// Ziel eines symbolischen Links, nur der letzte Teil (`…/devices/…/nvme0` → `nvme0`).
pub fn link_name(path: &Path) -> Option<String> {
    let t = std::fs::read_link(path).ok()?;
    Some(t.file_name()?.to_string_lossy().into_owned())
}

/// Pfad unter einer Wurzel zusammensetzen, auch wenn `rel` absolut ist (`/mnt/data` → `<root>/mnt/data`).
pub fn under(root: &Path, rel: &str) -> PathBuf {
    root.join(rel.trim_start_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        let mut v = Vec::new();
        fields_u64("cpu  10 20 30", &mut v);
        assert_eq!(v, [10, 20, 30]);
        let m: Vec<_> = kv("MemTotal:  16 kB\nFoo: 3\n").collect();
        assert_eq!(m, [("MemTotal", 16), ("Foo", 3)]);
        let mut got = Vec::new();
        header_table("Tcp: RtoMin MaxConn Est\nTcp: 200 -1 7\nUdp: In\nUdp: 5\n", |p, k, v| got.push(format!("{p}.{k}={v}")));
        assert_eq!(got, ["Tcp.RtoMin=200", "Tcp.MaxConn=0", "Tcp.Est=7", "Udp.In=5"]);
        let mut names = vec!["cpu10", "cpu2", "cpu1"];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(names, ["cpu1", "cpu2", "cpu10"]);
        assert_eq!(under(Path::new("/host/root"), "/mnt/x"), PathBuf::from("/host/root/mnt/x"));
    }
}
