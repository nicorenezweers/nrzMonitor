//! Zugriff auf die API des Servers (`/api/v1`). Fehler kommen als lesbarer Text zurück.

use dioxus::prelude::*;
use gloo_net::http::{Request, Response};
use serde::{Serialize, de::DeserializeOwned};
use shared::{
    api::{ApiError, Snapshot},
    query::{QueryReq, QueryResp},
};

/// Der Server hat „nicht angemeldet“ geantwortet → die Oberfläche zeigt die Anmeldung.
pub static UNAUTHORIZED: GlobalSignal<bool> = Signal::global(|| false);

fn url(path: &str) -> String {
    format!("/api/v1{path}")
}

async fn check(resp: Response) -> Result<Response, String> {
    if resp.status() == 401 {
        *UNAUTHORIZED.write() = true;
        return Err("Nicht angemeldet.".to_string());
    }
    if !resp.ok() {
        let status = resp.status();
        return Err(resp.json::<ApiError>().await.map(|e| e.error).unwrap_or_else(|_| format!("Fehler {status}")));
    }
    Ok(resp)
}

async fn parse<T: DeserializeOwned>(resp: Response) -> Result<T, String> {
    check(resp).await?.json::<T>().await.map_err(|e| format!("Antwort nicht lesbar: {e}"))
}

fn net(e: gloo_net::Error) -> String {
    format!("Server nicht erreichbar ({e})")
}

pub async fn get<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    parse(Request::get(&url(path)).send().await.map_err(net)?).await
}

pub async fn post<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> Result<T, String> {
    parse(Request::post(&url(path)).json(body).map_err(net)?.send().await.map_err(net)?).await
}

pub async fn put<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> Result<T, String> {
    parse(Request::put(&url(path)).json(body).map_err(net)?.send().await.map_err(net)?).await
}

/// `POST` ohne Antwortinhalt (204).
pub async fn post_empty<B: Serialize>(path: &str, body: &B) -> Result<(), String> {
    check(Request::post(&url(path)).json(body).map_err(net)?.send().await.map_err(net)?).await.map(|_| ())
}

/// `PUT` ohne Antwortinhalt (204).
pub async fn put_empty<B: Serialize>(path: &str, body: &B) -> Result<(), String> {
    check(Request::put(&url(path)).json(body).map_err(net)?.send().await.map_err(net)?).await.map(|_| ())
}

pub async fn delete(path: &str) -> Result<(), String> {
    check(Request::delete(&url(path)).send().await.map_err(net)?).await.map(|_| ())
}

/// Zeitreihen abfragen.
pub async fn query(req: &QueryReq) -> Result<QueryResp, String> {
    post("/query", req).await
}

/// Momentaufnahme einmalig lesen (laufend: `live::use_snap`).
pub async fn snap<T: DeserializeOwned>(key: &str) -> Result<Snapshot<T>, String> {
    get(&format!("/snap/{key}")).await
}

/// Query-Parameter anhängen: `qs("/events", &[("limit", "50".into())])`. Leere Werte entfallen.
pub fn qs(path: &str, params: &[(&str, String)]) -> String {
    let mut out = path.to_string();
    let mut first = !path.contains('?');
    for (k, v) in params {
        if v.is_empty() {
            continue;
        }
        out.push(if first { '?' } else { '&' });
        first = false;
        out.push_str(k);
        out.push('=');
        out.push_str(&String::from(js_sys::encode_uri_component(v)));
    }
    out
}
