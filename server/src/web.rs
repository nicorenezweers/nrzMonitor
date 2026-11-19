//! Liefert die Oberfläche aus: im Programm eingebaut (`web/dist`, gebaut mit `cargo xtask build-web`)
//! oder aus einem Ordner (`NRZMON_WEB_DIR`). Vorkomprimierte `.gz`-Dateien werden bevorzugt.

use std::borrow::Cow;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

use crate::core::Core;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../web/dist"]
struct Assets;

fn load(core: &Core, path: &str) -> Option<Cow<'static, [u8]>> {
    if path.contains("..") {
        return None;
    }
    match &core.cfg.web_dir {
        Some(dir) => std::fs::read(dir.join(path)).ok().map(Cow::Owned),
        None => Assets::get(path).map(|f| f.data),
    }
}

pub async fn serve(State(core): State<Core>, uri: Uri, headers: HeaderMap) -> Response {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.starts_with("api/") {
        return (StatusCode::NOT_FOUND, "Nicht gefunden").into_response();
    }
    if path.is_empty() {
        path = "index.html".to_string();
    }
    let accepts_gzip =
        headers.get(header::ACCEPT_ENCODING).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("gzip"));

    // Unbekannte Pfade ohne Dateiendung gehören der Oberfläche (Router im Browser).
    let (path, data, gzip) = match find(&core, &path, accepts_gzip) {
        Some((data, gzip)) => (path, data, gzip),
        None if !path.rsplit('/').next().unwrap_or_default().contains('.') => {
            match find(&core, "index.html", accepts_gzip) {
                Some((data, gzip)) => ("index.html".to_string(), data, gzip),
                None => return placeholder(),
            }
        }
        None => return (StatusCode::NOT_FOUND, "Nicht gefunden").into_response(),
    };

    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    let cache = if path == "index.html" {
        "no-cache"
    } else if path.starts_with("assets/") || path.starts_with("wasm/") {
        // dx hängt den Inhalts-Hash an die Dateinamen.
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    };
    let mut res = Response::new(Body::from(data.into_owned()));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if gzip {
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        h.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    }
    res
}

fn find(core: &Core, path: &str, accepts_gzip: bool) -> Option<(Cow<'static, [u8]>, bool)> {
    if accepts_gzip {
        if let Some(data) = load(core, &format!("{path}.gz")) {
            return Some((data, true));
        }
    }
    load(core, path).map(|d| (d, false))
}

fn placeholder() -> Response {
    let html = "<!doctype html><html lang=\"de\" class=\"dark\"><meta charset=\"utf-8\"><title>nrzMonitor</title>\
        <body style=\"background:#1a1a1a;color:#eee;font-family:system-ui;padding:3rem\">\
        <h1>nrzMonitor</h1><p>Der Server läuft, aber die Oberfläche ist nicht eingebaut.</p>\
        <p>Bauen: <code>cargo xtask build-web</code> – oder für die Entwicklung <code>cd web &amp;&amp; dx serve</code>.</p>\
        <p>API: <a style=\"color:#eee\" href=\"/api/v1/info\">/api/v1/info</a></p></body></html>";
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
}
