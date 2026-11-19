//! MCP-Schnittstelle (Model Context Protocol): Einstellungen und Ansicht für die Oberfläche.
//!
//! Der Server bietet unter `/mcp` einen MCP-Server (Streamable HTTP) an, dazu einen einfachen
//! Live-Strom für Dashboards (`/mcp/stream`, Server-Sent Events) und die ganze API lesend unter
//! `/mcp/api/v1/…`. Alles ist mit einem eigenen Schlüssel geschützt (nicht dem Passwort der Oberfläche).

use serde::{Deserialize, Serialize};

/// Einstellungen (`settings.json`, Schlüssel `mcp`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    /// MCP, Live-Strom und API-Zugang über `/mcp` anbieten.
    pub enabled: bool,
    /// Zugangsschlüssel (`Authorization: Bearer …`). Wird beim ersten Start erzeugt.
    pub token: String,
    /// Auch ändernde Aufrufe erlauben (Einstellungen, Alarmregeln …). Standard: nur lesen.
    pub allow_write: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self { enabled: true, token: String::new(), allow_write: false }
    }
}

/// `GET /api/v1/mcp` – Stand für die Seite „Einstellungen“ (ohne den Schlüssel im Klartext).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpView {
    pub enabled: bool,
    pub allow_write: bool,
    /// Letzte vier Zeichen des Schlüssels.
    pub token_hint: String,
    /// Der Schlüssel kommt aus `NRZMON_MCP_TOKEN` (in der Oberfläche nicht änderbar).
    pub token_env: bool,
    /// Protokollversion, die der Server bevorzugt.
    pub protocol: String,
    /// Offene MCP-Sitzungen und Live-Ströme.
    pub sessions: usize,
    pub streams: usize,
    pub tools: Vec<McpToolInfo>,
    /// Adressen, unter denen der Server lauscht (`NRZMON_BIND`).
    pub bind: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpToolInfo {
    pub name: String,
    pub title: String,
    pub description: String,
    /// Ändert etwas (nur mit `allow_write`).
    pub write: bool,
}

/// `PUT /api/v1/mcp`
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpUpdate {
    pub enabled: bool,
    pub allow_write: bool,
}

/// `POST /api/v1/mcp/token` – `action`: `reveal` (anzeigen) oder `new` (neu erzeugen).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpTokenReq {
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpToken {
    pub token: String,
}
