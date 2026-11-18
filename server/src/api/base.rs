//! Grundlegendes: Info, Anmeldung, Katalog, Momentaufnahmen, Live-Kanal.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use shared::{
    api::{Info, LoginReq, Snapshot},
    model::ChartMeta,
};

use super::{ApiErr, ApiResult};
use crate::{auth, core::Core, live, util};

pub fn routes() -> Router<Core> {
    Router::new()
        .route("/info", get(info))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/charts", get(charts))
        .route("/snap", get(snap_keys))
        .route("/snap/{key}", get(snap))
        .route("/live", get(live::upgrade))
}

async fn info(State(core): State<Core>, headers: HeaderMap) -> Json<Info> {
    Json(Info {
        version: env!("CARGO_PKG_VERSION").to_string(),
        hostname: core.hostname.clone(),
        demo: core.cfg.demo,
        started_at: core.started_at,
        now: util::now(),
        interval: core.cfg.interval,
        live_window: core.cfg.live_window,
        auth: core.cfg.password.is_some(),
        authenticated: auth::is_authenticated(&core, &headers),
        generation: core.registry.generation(),
    })
}

async fn login(State(core): State<Core>, Json(req): Json<LoginReq>) -> Result<Response, ApiErr> {
    let Some(password) = &core.cfg.password else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    if !core.sessions.allow_attempt() {
        return Err(ApiErr(StatusCode::TOO_MANY_REQUESTS, "Zu viele Versuche – bitte kurz warten.".into()));
    }
    if !util::constant_eq(req.password.as_bytes(), password.as_bytes()) {
        core.sessions.failed();
        return Err(ApiErr(StatusCode::UNAUTHORIZED, "Passwort stimmt nicht.".into()));
    }
    let token = core.sessions.create();
    let cookie = format!("{}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}", auth::COOKIE, 30 * 86_400);
    let mut res = StatusCode::NO_CONTENT.into_response();
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    Ok(res)
}

async fn logout(State(core): State<Core>, headers: HeaderMap) -> Response {
    if let Some(token) = auth::session_cookie(&headers) {
        core.sessions.remove(&token);
    }
    let cookie = format!("{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0", auth::COOKIE);
    ([(header::SET_COOKIE, cookie)], StatusCode::NO_CONTENT).into_response()
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ChartsQuery {
    /// Auch Diagramme, die nichts mehr liefern.
    retired: bool,
    section: Option<String>,
    context: Option<String>,
}

/// Katalog: alle Diagramme mit Dimensionen, sortiert wie in der Anzeige.
async fn charts(State(core): State<Core>, Query(q): Query<ChartsQuery>) -> Json<Vec<ChartMeta>> {
    let mut list = core.registry.list(q.retired);
    list.retain(|c| q.section.as_deref().is_none_or(|s| c.section == s) && q.context.as_deref().is_none_or(|x| c.context == x));
    for c in &mut list {
        c.last = c.dims.iter().filter_map(|d| core.ring.last_ts(d.series)).max().unwrap_or(0);
    }
    Json(list)
}

async fn snap_keys(State(core): State<Core>) -> Json<Vec<String>> {
    Json(core.snaps.keys())
}

async fn snap(State(core): State<Core>, Path(key): Path<String>) -> ApiResult<Snapshot<serde_json::Value>> {
    match core.snaps.get(&key) {
        Some(s) => Ok(Json(Snapshot { ts: s.ts, data: s.data.clone() })),
        None => Err(ApiErr::not_found(format!("Keine Momentaufnahme `{key}`."))),
    }
}
