//! API des Bereichs `dns`: Abfrageprotokoll, Ranglisten, Verlauf, Clients und Domains.
//!
//! - `GET /dns/queries?from&to&client&name&qtype&kind&rcode&upstream&page&per_page` → [`QueryPage`]
//! - `GET /dns/top?from&to&client&limit` → [`DnsTop`]
//! - `GET /dns/timeline?from&to&client&name&kind&points` → [`Timeline`]
//! - `GET /dns/clients?from&to&limit` → `Vec<ClientRow>`
//! - `GET /dns/domain?name&from&to&client` → [`DomainInfo`]
//!
//! Zeiten in Unix-Sekunden. Ohne `from` gelten die letzten 24 Stunden (Protokoll: alles).

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde::Deserialize;
use shared::dns::{ClientRow, DnsTop, DomainInfo, QueryFilter, QueryPage, Timeline};

use super::{ApiErr, ApiResult};
use crate::{collectors::dns, core::Core, util};

pub fn routes() -> Router<Core> {
    Router::new()
        .route("/dns/queries", get(queries))
        .route("/dns/top", get(top))
        .route("/dns/timeline", get(timeline))
        .route("/dns/clients", get(clients))
        .route("/dns/domain", get(domain))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Params {
    from: Option<i64>,
    to: Option<i64>,
    client: String,
    name: String,
    qtype: String,
    kind: String,
    rcode: String,
    upstream: String,
    page: u32,
    per_page: u32,
    limit: u32,
    points: u32,
}

impl Params {
    fn filter(&self, default_day: bool) -> QueryFilter {
        let from = self.from.or_else(|| default_day.then(|| self.to.unwrap_or_else(util::now) - 86_400));
        QueryFilter {
            from,
            to: self.to,
            client: self.client.clone(),
            name: self.name.clone(),
            qtype: self.qtype.clone(),
            kind: self.kind.clone(),
            rcode: self.rcode.clone(),
            upstream: self.upstream.clone(),
            page: self.page,
            per_page: self.per_page,
        }
    }
}

/// Abfrage im Speicher-Thread ausführen.
async fn run<T: Send + 'static>(f: impl FnOnce(&dns::store::DnsStore) -> anyhow::Result<T> + Send + 'static) -> Result<T, ApiErr> {
    let Some(store) = dns::current_store() else {
        return Err(ApiErr::not_found("DNS ist noch nicht bereit."));
    };
    tokio::task::spawn_blocking(move || f(&store))
        .await
        .map_err(|e| ApiErr::from(anyhow::anyhow!("Abfrage abgebrochen: {e}")))?
        .map_err(ApiErr::from)
}

async fn queries(Query(p): Query<Params>) -> ApiResult<QueryPage> {
    let f = p.filter(false);
    Ok(Json(run(move |s| s.page(&f)).await?))
}

async fn top(Query(p): Query<Params>) -> ApiResult<DnsTop> {
    let f = p.filter(true);
    let limit = if p.limit == 0 { 10 } else { p.limit };
    Ok(Json(run(move |s| s.top(&f, limit)).await?))
}

async fn timeline(Query(p): Query<Params>) -> ApiResult<Timeline> {
    let f = p.filter(true);
    let points = if p.points == 0 { 120 } else { p.points };
    Ok(Json(run(move |s| s.timeline(&f, points)).await?))
}

async fn clients(Query(p): Query<Params>) -> ApiResult<Vec<ClientRow>> {
    let f = p.filter(true);
    let limit = if p.limit == 0 { 200 } else { p.limit };
    Ok(Json(run(move |s| s.clients(&f, limit)).await?))
}

async fn domain(State(_core): State<Core>, Query(p): Query<Params>) -> ApiResult<DomainInfo> {
    if p.name.trim().is_empty() {
        return Err(ApiErr::bad("Parameter `name` fehlt."));
    }
    let f = p.filter(true);
    let name = p.name.clone();
    Ok(Json(run(move |s| s.domain(&name, &f)).await?))
}
