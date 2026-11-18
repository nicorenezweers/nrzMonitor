//! Anmeldung (nur wenn `NRZMON_PASSWORD` gesetzt ist): Sitzung im Cookie oder das Passwort als
//! `Authorization: Bearer …` für Skripte.

use std::collections::HashMap;

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use parking_lot::Mutex;

use crate::{api::ApiErr, core::Core, util};

pub const COOKIE: &str = "nrzmon_session";
const LIFETIME_SECS: i64 = 30 * 86_400;

#[derive(Default)]
pub struct Sessions {
    /// Sitzung → Ablauf (Unix-Sekunden)
    map: Mutex<HashMap<String, i64>>,
    /// Fehlversuche: (Fenster-Beginn, Anzahl)
    failures: Mutex<(i64, u32)>,
}

impl Sessions {
    pub fn create(&self) -> String {
        let token = util::random_hex(32);
        let now = util::now();
        let mut map = self.map.lock();
        map.retain(|_, exp| *exp > now);
        map.insert(token.clone(), now + LIFETIME_SECS);
        token
    }

    pub fn valid(&self, token: &str) -> bool {
        self.map.lock().get(token).is_some_and(|exp| *exp > util::now())
    }

    pub fn remove(&self, token: &str) {
        self.map.lock().remove(token);
    }

    /// Höchstens 10 Fehlversuche je Minute (für alle zusammen).
    pub fn allow_attempt(&self) -> bool {
        let now = util::now();
        let mut f = self.failures.lock();
        if now - f.0 >= 60 {
            *f = (now, 0);
        }
        f.1 < 10
    }

    pub fn failed(&self) {
        self.failures.lock().1 += 1;
    }
}

pub fn session_cookie(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').filter_map(|c| c.trim().split_once('=')).find(|(k, _)| *k == COOKIE).map(|(_, v)| v.to_string())
}

pub fn is_authenticated(core: &Core, headers: &HeaderMap) -> bool {
    let Some(password) = &core.cfg.password else { return true };
    if session_cookie(headers).is_some_and(|t| core.sessions.valid(&t)) {
        return true;
    }
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| util::constant_eq(t.as_bytes(), password.as_bytes()))
}

/// Schützt alle API-Routen ausser Info und Anmeldung.
pub async fn require(State(core): State<Core>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let open = path.ends_with("/info") || path.ends_with("/login") || path.ends_with("/logout");
    if open || is_authenticated(&core, req.headers()) {
        next.run(req).await
    } else {
        ApiErr(StatusCode::UNAUTHORIZED, "Nicht angemeldet.".into()).into_response()
    }
}
