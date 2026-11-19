//! `nrzmonitor mcp`: Brücke stdin/stdout ↔ `/mcp` des laufenden Servers – für MCP-Programme, die nur
//! lokale Prozesse starten können. Im Container:
//!
//! ```text
//! docker exec -i nrzmonitor /app/nrzmonitor mcp
//! ```
//!
//! Adresse aus `NRZMON_MCP_URL` oder `NRZMON_BIND`, Schlüssel aus `NRZMON_MCP_TOKEN` oder
//! `<data>/settings.json` – im Container braucht es also keine Angaben.

use std::{sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
};

use crate::config::{self, Config};

struct Bridge {
    http: reqwest::Client,
    url: String,
    token: String,
    session: Mutex<Option<String>>,
    out: mpsc::Sender<String>,
}

pub fn run(cfg: Config) -> anyhow::Result<()> {
    let mut args = std::env::args().skip(2);
    let (mut url, mut token) = (config::var("NRZMON_MCP_URL"), config::var("NRZMON_MCP_TOKEN"));
    while let Some(a) = args.next() {
        match a.as_str() {
            "--url" => url = args.next(),
            "--token" => token = args.next(),
            "-h" | "--help" => {
                eprintln!(
                    "nrzmonitor mcp [--url http://127.0.0.1:8571/mcp] [--token SCHLUESSEL]\nBrücke stdin/stdout ↔ MCP-Server von nrzMonitor."
                );
                return Ok(());
            }
            _ => {}
        }
    }
    let url = url.unwrap_or_else(|| {
        let mut addr = cfg.bind[0];
        if addr.ip().is_unspecified() {
            addr.set_ip([127, 0, 0, 1].into());
        }
        format!("http://{addr}/mcp")
    });
    let token = token
        .or_else(|| {
            let s = std::fs::read_to_string(cfg.data_dir.join("settings.json")).ok()?;
            let v: Value = serde_json::from_str(&s).ok()?;
            v.pointer("/mcp/token").and_then(Value::as_str).map(String::from)
        })
        .unwrap_or_default();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(bridge(url, token))
}

async fn bridge(url: String, token: String) -> anyhow::Result<()> {
    let (out, mut rx) = mpsc::channel::<String>(256);
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(line) = rx.recv().await {
            if stdout.write_all(line.as_bytes()).await.is_err() || stdout.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = stdout.flush().await;
        }
    });
    let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(900)).build()?;
    let b = Arc::new(Bridge { http, url, token, session: Mutex::new(None), out });
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let is_init = line.contains("\"initialize\"");
        if is_init {
            b.clone().post(line).await;
        } else {
            tokio::spawn(b.clone().post(line));
        }
    }
    // stdin zu: Sitzung beenden.
    let session = b.session.lock().clone();
    if let Some(s) = session {
        let _ = b.http.delete(&b.url).bearer_auth(&b.token).header("mcp-session-id", s).send().await;
    }
    drop(b);
    let _ = writer.await;
    Ok(())
}

impl Bridge {
    fn emit(&self, line: String) {
        let _ = self.out.try_send(line);
    }

    fn error_for(&self, msg: &str, text: String) {
        let id = serde_json::from_str::<Value>(msg).ok().and_then(|v| v.get("id").cloned());
        if let Some(id) = id {
            self.emit(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": text}}).to_string());
        } else {
            eprintln!("nrzmonitor mcp: {text}");
        }
    }

    async fn post(self: Arc<Self>, msg: String) {
        let session = self.session.lock().clone();
        let mut req = self
            .http
            .post(&self.url)
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(msg.clone());
        if let Some(s) = &session {
            req = req.header("mcp-session-id", s);
        }
        let res = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                return self.error_for(&msg, format!("nrzMonitor unter {} nicht erreichbar: {e}", self.url));
            }
        };
        if let Some(s) = res.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            let new = s.to_string();
            let changed = self.session.lock().replace(new.clone()).as_deref() != Some(new.as_str());
            if changed {
                tokio::spawn(self.clone().listen(new));
            }
        }
        let status = res.status();
        let sse = res.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("text/event-stream"));
        if sse {
            self.read_sse(res).await;
            return;
        }
        let body = res.text().await.unwrap_or_default();
        if status.as_u16() == 202 || body.trim().is_empty() {
            return;
        }
        if !status.is_success() && serde_json::from_str::<Value>(&body).ok().and_then(|v| v.get("jsonrpc").cloned()).is_none() {
            return self.error_for(&msg, format!("Fehler {}: {}", status.as_u16(), body.trim()));
        }
        self.emit(body.trim().to_string());
    }

    async fn read_sse(&self, mut res: reqwest::Response) {
        let mut buf = String::new();
        while let Ok(Some(chunk)) = res.chunk().await {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = buf.find("\n\n") {
                let block: String = buf.drain(..end + 2).collect();
                let data: Vec<&str> = block.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim_start).collect();
                if !data.is_empty() {
                    self.emit(data.join("\n"));
                }
            }
        }
    }

    /// Benachrichtigungen der Sitzung (`GET /mcp`) weiterreichen; bricht er ab, neu verbinden.
    async fn listen(self: Arc<Self>, session: String) {
        loop {
            if self.session.lock().as_deref() != Some(session.as_str()) {
                return;
            }
            let res = self
                .http
                .get(&self.url)
                .bearer_auth(&self.token)
                .header("accept", "text/event-stream")
                .header("mcp-session-id", &session)
                .timeout(Duration::from_secs(365 * 86_400))
                .send()
                .await;
            match res {
                Ok(r) if r.status().is_success() => self.read_sse(r).await,
                Ok(r) if r.status().as_u16() == 404 => return,
                _ => {}
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }
}
