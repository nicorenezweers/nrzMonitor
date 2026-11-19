//! HTTP-API unter `/api/v1`. Jeder Bereich hat eine eigene Datei mit `routes()`.

use axum::{
    Json, Router,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
};
use shared::api::ApiError;
use tower_http::compression::CompressionLayer;

use crate::{auth, core::Core, web};

pub mod alerts;
pub mod base;
pub mod dns;
pub mod docker;
pub mod integrations;
pub mod mcp;
pub mod power;
pub mod query;
pub mod storage;
pub mod system;

/// Alle Routen unter `/api/v1`, noch ohne Anmeldung (die setzt [`router`] davor; MCP ruft sie intern auf).
pub fn routes() -> Router<Core> {
    Router::new()
        .merge(base::routes())
        .merge(query::routes())
        .merge(storage::routes())
        .merge(system::routes())
        .merge(docker::routes())
        .merge(alerts::routes())
        .merge(dns::routes())
        .merge(integrations::routes())
        .merge(power::routes())
        .merge(mcp::routes())
}

pub fn router(core: Core) -> Router {
    let api = routes().layer(middleware::from_fn_with_state(core.clone(), auth::require));
    // Für MCP: dieselbe API ohne Anmeldung (MCP prüft seinen eigenen Schlüssel).
    let internal = Router::new().nest("/api/v1", routes()).with_state(core.clone());
    Router::new()
        .nest("/api/v1", api)
        .merge(crate::mcp::routes(&core, internal))
        .fallback(web::serve)
        .layer(CompressionLayer::new())
        .with_state(core)
}

/// Fehlerantwort: Statuscode und `{"error": "…"}`.
#[derive(Debug)]
pub struct ApiErr(pub StatusCode, pub String);

impl ApiErr {
    pub fn bad(msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, msg.into())
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        (self.0, Json(ApiError { error: self.1 })).into_response()
    }
}

impl From<anyhow::Error> for ApiErr {
    fn from(e: anyhow::Error) -> Self {
        tracing::warn!("API-Fehler: {e:#}");
        Self(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

pub type ApiResult<T> = Result<Json<T>, ApiErr>;
