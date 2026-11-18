//! Zeitreihen, Ereignisse, Auswertungen und CSV-Export.

use axum::{
    Json, Router,
    extract::{Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use shared::{
    events::{EventPage, EventQuery},
    query::{QueryReq, QueryResp},
    storage::{CorrelateReq, CorrelateResp, ExportQuery, StatsReq, StatsResp, TopReq, TopResp},
};

use super::{ApiErr, ApiResult, storage::ensure_publisher};
use crate::core::Core;

pub fn routes() -> Router<Core> {
    Router::new()
        .route("/query", post(query))
        .route("/events", get(events))
        .route("/analyze/stats", post(stats))
        .route("/analyze/top", post(top))
        .route("/analyze/correlate", post(correlate))
        .route("/export.csv", get(export))
}

async fn query(State(core): State<Core>, Json(req): Json<QueryReq>) -> ApiResult<QueryResp> {
    ensure_publisher(&core);
    if req.charts.len() > 500 {
        return Err(ApiErr::bad("Höchstens 500 Diagramme je Abfrage."));
    }
    Ok(Json(core.store.query(&req).await?))
}

async fn events(State(core): State<Core>, Query(q): Query<EventQuery>) -> ApiResult<EventPage> {
    Ok(Json(core.store.events(&q).await?))
}

async fn stats(State(core): State<Core>, Json(req): Json<StatsReq>) -> ApiResult<StatsResp> {
    if req.charts.len() > 200 {
        return Err(ApiErr::bad("Höchstens 200 Diagramme je Auswertung."));
    }
    Ok(Json(core.store.analyze_stats(&req).await?))
}

async fn top(State(core): State<Core>, Json(req): Json<TopReq>) -> ApiResult<TopResp> {
    if req.context.is_empty() {
        return Err(ApiErr::bad("Kontext fehlt."));
    }
    Ok(Json(core.store.analyze_top(&req).await?))
}

async fn correlate(State(core): State<Core>, Json(req): Json<CorrelateReq>) -> ApiResult<CorrelateResp> {
    Ok(Json(core.store.analyze_correlate(&req).await?))
}

async fn export(State(core): State<Core>, Query(q): Query<ExportQuery>) -> Result<Response, ApiErr> {
    let charts: Vec<String> = q.charts.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
    if charts.is_empty() || charts.len() > 100 {
        return Err(ApiErr::bad("1 bis 100 Diagramme angeben (charts=a,b)."));
    }
    let csv = core.store.export_csv(&charts, q.after, q.before, q.points).await?;
    let name = format!("attachment; filename=\"nrzmonitor-{}.csv\"", core.store.now());
    Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, name)], csv).into_response())
}
