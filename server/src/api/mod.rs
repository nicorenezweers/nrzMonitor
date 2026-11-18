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
pub mod power;
pub mod query;
pub mod storage;
pub mod system;

pub fn router(core: Core) -> Router {
    let api = Router::new()
        .merge(base::routes())
        .merge(query::routes())
        .merge(storage::routes())
        .merge(system::routes())
        .merge(docker::routes())
        .merge(alerts::routes())
        .merge(dns::routes())
        .merge(integrations::routes())
        .merge(power::routes())
        .layer(middleware::from_fn_with_state(core.clone(), auth::require));
    Router::new().nest("/api/v1", api).fallback(web::serve).layer(CompressionLayer::new()).with_state(core)
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
