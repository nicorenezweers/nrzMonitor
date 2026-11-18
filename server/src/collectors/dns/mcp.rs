//! Schlanker MCP-Client (JSON-RPC 2.0 über „Streamable HTTP“) für nrzNetwork.
//!
//! nrzNetwork ist zustandslos: jede Anfrage ein `POST` mit `Authorization: Bearer <token>`, die
//! Antwort kommt als JSON (oder als SSE-Strom, falls ein Server das so macht). Werkzeug-Ergebnisse
//! stehen in `structuredContent` oder als JSON-Text in `content[0].text`.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Fehler eines Aufrufs – verständlich formuliert, unterscheidet „Token falsch“ von „nicht erreichbar“.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpError {
    Unreachable(String),
    Auth(String),
    Http(u16, String),
    Rpc(String),
    Tool(String),
    Parse(String),
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(e) => write!(f, "nrzNetwork nicht erreichbar: {e}"),
            Self::Auth(m) => write!(f, "nrzNetwork lehnt das Token ab{} – Token prüfen", if m.is_empty() { String::new() } else { format!(" ({m})") }),
            Self::Http(s, m) => write!(f, "nrzNetwork antwortet mit HTTP {s}: {m}"),
            Self::Rpc(m) => write!(f, "MCP-Fehler: {m}"),
            Self::Tool(m) => write!(f, "Werkzeug meldet: {m}"),
            Self::Parse(m) => write!(f, "Antwort nicht lesbar: {m}"),
        }
    }
}

impl std::error::Error for McpError {}

#[derive(Clone)]
pub struct Mcp {
    http: reqwest::Client,
    url: String,
    token: String,
}

impl Mcp {
    pub fn new(http: reqwest::Client, url: &str, token: &str) -> Self {
        Self { http, url: url.trim().to_string(), token: token.trim().to_string() }
    }

    /// JSON-RPC-Aufruf; liefert `result` und die Grösse der Antwort in Bytes.
    pub async fn rpc(&self, method: &str, params: Value) -> Result<(Value, usize), McpError> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let res = self
            .http
            .post(&self.url)
            .bearer_auth(&self.token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-06-18")
            .timeout(Duration::from_secs(30))
            .json(&body)
            .send()
            .await
            .map_err(|e| McpError::Unreachable(short(&e)))?;
        let status = res.status();
        let sse = res
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream"));
        let text = res.text().await.map_err(|e| McpError::Unreachable(short(&e)))?;
        let size = text.len();
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| text.chars().take(200).collect());
            if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
                return Err(McpError::Auth(msg));
            }
            return Err(McpError::Http(status.as_u16(), msg));
        }
        let messages: Vec<Value> = if sse {
            text.lines().filter_map(|l| l.strip_prefix("data:")).filter_map(|d| serde_json::from_str(d.trim()).ok()).collect()
        } else {
            match serde_json::from_str::<Value>(&text).map_err(|e| McpError::Parse(e.to_string()))? {
                Value::Array(list) => list,
                v => vec![v],
            }
        };
        let msg = messages
            .into_iter()
            .find(|m| m.get("id").and_then(Value::as_u64) == Some(id))
            .ok_or_else(|| McpError::Parse("keine Antwort mit passender ID".into()))?;
        if let Some(err) = msg.get("error") {
            return Err(McpError::Rpc(err.get("message").and_then(Value::as_str).unwrap_or("unbekannt").to_string()));
        }
        Ok((msg.get("result").cloned().unwrap_or(Value::Null), size))
    }

    /// Verbindung aufbauen und Server-Namen lesen (für „Verbindung testen“).
    pub async fn initialize(&self) -> Result<String, McpError> {
        let params = json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "nrzMonitor", "version": env!("CARGO_PKG_VERSION") }
        });
        let (r, _) = self.rpc("initialize", params).await?;
        let info = r.get("serverInfo").cloned().unwrap_or(Value::Null);
        let name = info.get("title").or_else(|| info.get("name")).and_then(Value::as_str).unwrap_or("MCP-Server");
        let version = info.get("version").and_then(Value::as_str).unwrap_or("");
        Ok(format!("{name} {version}").trim().to_string())
    }

    /// Werkzeug aufrufen; liefert das Ergebnis und die Antwortgrösse in Bytes.
    pub async fn call(&self, name: &str, args: Value) -> Result<(Value, usize), McpError> {
        let (r, size) = self.rpc("tools/call", json!({ "name": name, "arguments": args })).await?;
        Ok((tool_result(r)?, size))
    }

    pub async fn call_as<T: DeserializeOwned>(&self, name: &str, args: Value) -> Result<(T, usize), McpError> {
        let (v, size) = self.call(name, args).await?;
        let t = serde_json::from_value(v).map_err(|e| McpError::Parse(format!("{name}: {e}")))?;
        Ok((t, size))
    }
}

/// `structuredContent` bevorzugen, sonst den Text (als JSON, wenn möglich).
fn tool_result(r: Value) -> Result<Value, McpError> {
    let failed = r.get("isError").and_then(Value::as_bool).unwrap_or(false);
    if !failed && let Some(v) = r.get("structuredContent").filter(|v| !v.is_null()) {
        return Ok(v.clone());
    }
    let text: String = r
        .get("content")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(|c| c.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    if failed {
        return Err(McpError::Tool(if text.is_empty() { "Fehler ohne Text".into() } else { text }));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// Fehlertext von reqwest ohne die lange Kette.
fn short(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "Zeitüberschreitung".into();
    }
    if e.is_connect() {
        return "Verbindung fehlgeschlagen".into();
    }
    let s = e.to_string();
    s.chars().take(160).collect()
}
