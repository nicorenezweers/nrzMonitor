//! Einstellungen des MCP-Servers (`/api/v1/mcp`): Schalter, Schlüssel anzeigen oder neu erzeugen.

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use shared::mcp::{McpSettings, McpToken, McpTokenReq, McpUpdate, McpView};

use super::{ApiErr, ApiResult};
use crate::{config, core::Core, mcp, util};

pub fn routes() -> Router<Core> {
    Router::new().route("/mcp", get(view).put(update)).route("/mcp/token", post(token))
}

fn make_view(core: &Core) -> McpView {
    let s = mcp::settings(core);
    let hint: String = s.token.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    McpView {
        enabled: s.enabled,
        allow_write: s.allow_write,
        token_hint: hint,
        token_env: config::var("NRZMON_MCP_TOKEN").is_some(),
        protocol: mcp::PROTOCOL.to_string(),
        sessions: mcp::hub().map_or(0, |h| h.sessions()),
        streams: mcp::hub().map_or(0, |h| h.streams.load(std::sync::atomic::Ordering::Relaxed)),
        tools: mcp::tool_catalog(),
        bind: core.cfg.bind.iter().map(ToString::to_string).collect(),
    }
}

async fn view(State(core): State<Core>) -> Json<McpView> {
    Json(make_view(&core))
}

async fn update(State(core): State<Core>, Json(u): Json<McpUpdate>) -> ApiResult<McpView> {
    let mut s: McpSettings = core.settings.get("mcp");
    s.enabled = u.enabled;
    s.allow_write = u.allow_write;
    core.settings.set("mcp", &s)?;
    Ok(Json(make_view(&core)))
}

async fn token(State(core): State<Core>, Json(r): Json<McpTokenReq>) -> ApiResult<McpToken> {
    match r.action.as_str() {
        "reveal" => Ok(Json(McpToken { token: mcp::settings(&core).token })),
        "new" => {
            if config::var("NRZMON_MCP_TOKEN").is_some() {
                return Err(ApiErr::bad("Der Schlüssel kommt aus NRZMON_MCP_TOKEN und lässt sich hier nicht ändern."));
            }
            let mut s: McpSettings = core.settings.get("mcp");
            s.token = util::random_hex(24);
            core.settings.set("mcp", &s)?;
            Ok(Json(McpToken { token: s.token }))
        }
        _ => Err(ApiErr::bad("`action`: reveal oder new.")),
    }
}
