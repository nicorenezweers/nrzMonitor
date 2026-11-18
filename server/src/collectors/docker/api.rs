//! Kleiner HTTP/1.1-Client für die Docker-API am Unix-Socket (nur lesend, ohne Zusatz-Crates).

use std::{path::PathBuf, time::Duration};

use anyhow::{Context, bail};
use serde::de::DeserializeOwned;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

/// Docker-API ab dieser Version (Docker 20.10+).
const API: &str = "/v1.41";

#[derive(Debug, Clone)]
pub struct Docker {
    sock: PathBuf,
}

impl Docker {
    pub fn new(sock: PathBuf) -> Self {
        Self { sock }
    }

    /// Antwort öffnen (Status, Kopf gelesen); der Inhalt wird mit [`Body`] gelesen.
    pub async fn open(&self, path: &str) -> anyhow::Result<Body> {
        let mut stream = UnixStream::connect(&self.sock).await.with_context(|| format!("Docker-Socket {}", self.sock.display()))?;
        let req = format!("GET {API}{path} HTTP/1.1\r\nHost: docker\r\nUser-Agent: nrzMonitor\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await?;
        let mut buf = Vec::with_capacity(16 * 1024);
        let head_end = loop {
            if let Some(p) = find(&buf, b"\r\n\r\n") {
                break p;
            }
            if buf.len() > 64 * 1024 {
                bail!("Docker: Antwortkopf zu gross");
            }
            let mut tmp = [0u8; 8192];
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                bail!("Docker: Verbindung vorzeitig beendet");
            }
            buf.extend_from_slice(&tmp[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let rest = buf[head_end + 4..].to_vec();
        let mut lines = head.split("\r\n");
        let status: u16 = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok()).unwrap_or(0);
        let (mut chunked, mut length) = (false, None);
        for l in lines {
            let Some((k, v)) = l.split_once(':') else { continue };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "transfer-encoding" && v.eq_ignore_ascii_case("chunked") {
                chunked = true;
            } else if k == "content-length" {
                length = v.parse::<usize>().ok();
            }
        }
        Ok(Body { stream, buf: rest, status, chunked, remaining: length, chunk_left: 0, done: false })
    }

    /// Ganze Antwort mit Zeitlimit; Fehler bei Status ausserhalb 2xx.
    pub async fn get(&self, path: &str, timeout: Duration) -> anyhow::Result<Vec<u8>> {
        let fut = async {
            let body = self.open(path).await?;
            let status = body.status;
            let data = body.all().await?;
            if !(200..300).contains(&status) {
                let msg = serde_json::from_slice::<serde_json::Value>(&data)
                    .ok()
                    .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
                    .unwrap_or_else(|| String::from_utf8_lossy(&data).chars().take(200).collect());
                bail!(ApiStatus { status, msg });
            }
            Ok(data)
        };
        tokio::time::timeout(timeout, fut).await.map_err(|_| anyhow::anyhow!("Docker: Zeitlimit für {path}"))?
    }

    pub async fn json<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let data = self.get(path, Duration::from_secs(15)).await?;
        serde_json::from_slice(&data).with_context(|| format!("Docker: Antwort von {path} nicht lesbar"))
    }
}

/// Fehlerstatus der Docker-API (404 = nicht gefunden).
#[derive(Debug)]
pub struct ApiStatus {
    pub status: u16,
    pub msg: String,
}

impl std::fmt::Display for ApiStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Docker {}: {}", self.status, self.msg)
    }
}

impl std::error::Error for ApiStatus {}

/// Inhalt einer Antwort, stückweise lesbar (für Ereignisströme und Logs).
pub struct Body {
    stream: UnixStream,
    buf: Vec<u8>,
    pub status: u16,
    chunked: bool,
    remaining: Option<usize>,
    chunk_left: usize,
    done: bool,
}

impl Body {
    async fn fill(&mut self) -> anyhow::Result<bool> {
        let mut tmp = [0u8; 16 * 1024];
        let n = self.stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(false);
        }
        self.buf.extend_from_slice(&tmp[..n]);
        Ok(true)
    }

    /// Nächstes Stück des (entpackten) Inhalts, `None` am Ende.
    pub async fn chunk(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        if self.done {
            return Ok(None);
        }
        if !self.chunked {
            if self.remaining == Some(0) {
                self.done = true;
                return Ok(None);
            }
            if self.buf.is_empty() && !self.fill().await? {
                self.done = true;
                return Ok(None);
            }
            let mut out = std::mem::take(&mut self.buf);
            if let Some(r) = self.remaining.as_mut() {
                out.truncate(*r);
                *r -= out.len();
            }
            return Ok(Some(out));
        }
        loop {
            if self.chunk_left > 0 {
                if self.buf.is_empty() && !self.fill().await? {
                    self.done = true;
                    return Ok(None);
                }
                let n = self.chunk_left.min(self.buf.len());
                let out: Vec<u8> = self.buf.drain(..n).collect();
                self.chunk_left -= n;
                return Ok(Some(out));
            }
            // Grössenzeile (davor ggf. das CRLF des letzten Stücks).
            while self.buf.starts_with(b"\r\n") {
                self.buf.drain(..2);
            }
            let Some(p) = find(&self.buf, b"\r\n") else {
                if !self.fill().await? {
                    self.done = true;
                    return Ok(None);
                }
                continue;
            };
            let line = String::from_utf8_lossy(&self.buf[..p]).to_string();
            self.buf.drain(..p + 2);
            let size = usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
            if size == 0 {
                self.done = true;
                return Ok(None);
            }
            self.chunk_left = size;
        }
    }

    pub async fn all(mut self) -> anyhow::Result<Vec<u8>> {
        let mut out = Vec::new();
        while let Some(c) = self.chunk().await? {
            out.extend_from_slice(&c);
        }
        Ok(out)
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Prozentkodierung für Query-Parameter.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Logs von Docker zerlegen. Ohne TTY kommen Rahmen (8 Byte Kopf: Strom, 0, 0, 0, Länge),
/// mit TTY roher Text. Liefert (stderr, Zeile) – Zeilen beginnen mit dem Zeitstempel.
pub fn demux(data: &[u8], tty: bool) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    let mut push = |err: bool, bytes: &[u8]| {
        for line in String::from_utf8_lossy(bytes).split('\n') {
            let line = line.trim_end_matches('\r');
            if !line.is_empty() {
                out.push((err, line.to_string()));
            }
        }
    };
    if tty {
        push(false, data);
        return out;
    }
    let mut i = 0;
    // Rahmen mit gleichem Strom zusammenfassen, damit über Rahmen geteilte Zeilen ganz bleiben.
    let mut cur: Option<(bool, Vec<u8>)> = None;
    while i + 8 <= data.len() {
        let kind = data[i];
        let len = u32::from_be_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        let end = (i + 8 + len).min(data.len());
        let err = kind == 2;
        match &mut cur {
            Some((e, b)) if *e == err => b.extend_from_slice(&data[i + 8..end]),
            _ => {
                if let Some((e, b)) = cur.take() {
                    push(e, &b);
                }
                cur = Some((err, data[i + 8..end].to_vec()));
            }
        }
        i = end;
    }
    if let Some((e, b)) = cur {
        push(e, &b);
    }
    out
}

/// RFC 3339 (`2024-05-01T12:00:00.123456789Z`, auch mit `+02:00`) → Unix-Nanosekunden.
pub fn parse_ts_ns(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[10] != b'T' {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut rest = &s[19..];
    let mut nanos = 0i64;
    if let Some(r) = rest.strip_prefix('.') {
        let digits: String = r.chars().take_while(char::is_ascii_digit).collect();
        rest = &r[digits.len()..];
        let mut padded = digits.clone();
        padded.truncate(9);
        while padded.len() < 9 {
            padded.push('0');
        }
        nanos = padded.parse().unwrap_or(0);
    }
    let offset = match rest.as_bytes().first() {
        Some(b'+' | b'-') if rest.len() >= 6 => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            sign * (rest[1..3].parse::<i64>().ok()? * 3600 + rest[4..6].parse::<i64>().ok()? * 60)
        }
        _ => 0,
    };
    let secs = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset;
    secs.checked_mul(1_000_000_000)?.checked_add(nanos)
}

/// Tage seit 1970-01-01 (Algorithmus von H. Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Zeitstempel als Unix-Sekunden (0 bei `0001-01-01…` oder unlesbar).
pub fn parse_ts(s: &str) -> i64 {
    match parse_ts_ns(s) {
        Some(ns) if ns > 0 => ns / 1_000_000_000,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(parse_ts("1970-01-01T00:00:10Z"), 10);
        assert_eq!(parse_ts("2024-05-01T12:00:00.5+02:00"), 1_714_557_600);
        assert_eq!(parse_ts_ns("2024-05-01T10:00:00.000000123Z"), Some(1_714_557_600_000_000_123));
        assert_eq!(parse_ts("0001-01-01T00:00:00Z"), 0);
    }

    #[test]
    fn demuxes_frames() {
        let mut data = vec![1, 0, 0, 0, 0, 0, 0, 5];
        data.extend_from_slice(b"a\nb\nc");
        data.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 2]);
        data.extend_from_slice(b"d\n");
        data.extend_from_slice(&[2, 0, 0, 0, 0, 0, 0, 4]);
        data.extend_from_slice(b"err\n");
        let lines = demux(&data, false);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[2], (false, "cd".to_string()));
        assert_eq!(lines[3], (true, "err".to_string()));
    }
}
