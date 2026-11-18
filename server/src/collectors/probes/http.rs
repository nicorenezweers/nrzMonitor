//! HTTP(S)-Prüfung mit Zeitanteilen: DNS, Verbinden, TLS, Warten (bis zum ersten Byte), Übertragen.
//! Eigene kleine HTTP/1.1-Anfrage, damit sich jede Phase messen lässt.

use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, pki_types::ServerName},
};

/// Gelesene Antwort höchstens so gross.
const MAX_BODY: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HttpResult {
    pub status: u16,
    pub dns_ms: f64,
    pub connect_ms: f64,
    pub tls_ms: f64,
    pub wait_ms: f64,
    pub transfer_ms: f64,
    pub total_ms: f64,
}

fn tls() -> Result<TlsConnector, String> {
    static CFG: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    let cfg = CFG.get_or_init(|| {
        let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
        ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map(|b| Arc::new(b.with_root_certificates(roots).with_no_client_auth()))
            .map_err(|e| e.to_string())
    });
    cfg.clone().map(TlsConnector::from)
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// Eine Prüfung; Fehler als lesbarer Text.
pub async fn check(url: &str, timeout: Duration) -> Result<HttpResult, String> {
    tokio::time::timeout(timeout, run(url)).await.map_err(|_| "Zeitüberschreitung".to_string())?
}

async fn run(url: &str) -> Result<HttpResult, String> {
    let u = reqwest::Url::parse(url).map_err(|e| format!("Adresse ungültig: {e}"))?;
    let https = match u.scheme() {
        "https" => true,
        "http" => false,
        s => return Err(format!("Schema {s} nicht unterstützt")),
    };
    let host = u.host_str().ok_or("Adresse ohne Rechnername")?.trim_start_matches('[').trim_end_matches(']').to_string();
    let port = u.port_or_known_default().unwrap_or(if https { 443 } else { 80 });
    let mut path = u.path().to_string();
    if let Some(q) = u.query() {
        path.push('?');
        path.push_str(q);
    }
    let mut r = HttpResult::default();
    let start = Instant::now();
    let t = Instant::now();
    let addr = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("DNS: {e}"))?
        .next()
        .ok_or("DNS: keine Adresse")?;
    r.dns_ms = ms(t);
    let t = Instant::now();
    let tcp = TcpStream::connect(addr).await.map_err(|e| format!("Verbinden: {e}"))?;
    let _ = tcp.set_nodelay(true);
    r.connect_ms = ms(t);
    let host_header = if (https && port == 443) || (!https && port == 80) { host.clone() } else { format!("{host}:{port}") };
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: nrzMonitor/{}\r\nAccept: */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
        env!("CARGO_PKG_VERSION")
    );
    if https {
        let t = Instant::now();
        let name = ServerName::try_from(host.clone()).map_err(|e| format!("TLS: {e}"))?;
        let stream = tls()?.connect(name, tcp).await.map_err(|e| format!("TLS: {e}"))?;
        r.tls_ms = ms(t);
        exchange(stream, &req, &mut r).await?;
    } else {
        exchange(tcp, &req, &mut r).await?;
    }
    r.total_ms = ms(start);
    Ok(r)
}

async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(mut s: S, req: &str, r: &mut HttpResult) -> Result<(), String> {
    let t = Instant::now();
    s.write_all(req.as_bytes()).await.map_err(|e| format!("Senden: {e}"))?;
    let mut buf = vec![0u8; 16 * 1024];
    let n = s.read(&mut buf).await.map_err(|e| format!("Lesen: {e}"))?;
    if n == 0 {
        return Err("Verbindung ohne Antwort geschlossen".into());
    }
    r.wait_ms = ms(t);
    let mut head = buf[..n].to_vec();
    let t = Instant::now();
    let line = String::from_utf8_lossy(&head[..head.iter().position(|b| *b == b'\n').unwrap_or(head.len())]).to_string();
    r.status = line.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| format!("keine HTTP-Antwort: {}", line.trim()))?;
    // Rest lesen (bis Content-Length, Ende oder Obergrenze).
    let mut total = n;
    let header_end = |h: &[u8]| h.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
    let mut expected: Option<usize> = None;
    loop {
        if expected.is_none()
            && let Some(end) = header_end(&head)
        {
            let headers = String::from_utf8_lossy(&head[..end]).to_ascii_lowercase();
            let len = headers.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok());
            expected = Some(len.map_or(usize::MAX, |l| end + l));
            head.truncate(end);
        }
        if expected.is_some_and(|e| total >= e) || total >= MAX_BODY {
            break;
        }
        match s.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(k) => {
                total += k;
                if expected.is_none() {
                    head.extend_from_slice(&buf[..k]);
                }
            }
        }
    }
    r.transfer_ms = ms(t);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_http_phases() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut b = [0u8; 1024];
            let _ = s.read(&mut b).await;
            tokio::time::sleep(Duration::from_millis(30)).await;
            s.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 5\r\n\r\nhallo").await.unwrap();
        });
        let r = check(&format!("http://{addr}/gesund"), Duration::from_secs(3)).await.unwrap();
        assert_eq!(r.status, 204);
        assert!(r.wait_ms >= 25.0, "{r:?}");
        assert!(r.total_ms >= r.wait_ms);
        assert!(check("ftp://x", Duration::from_secs(1)).await.is_err());
    }
}
