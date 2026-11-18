//! Einstellungen der Integrationen (DNS über nrzNetwork, Cloudflare, NetBird, Erreichbarkeit).
//!
//! - `GET  /integrations` → [`IntegrationsView`] (Tokens nur als „gesetzt“)
//! - `PUT  /integrations/{dns|cloudflare|netbird}` ← [`TokenForm`] (`token: null` = unverändert)
//! - `PUT  /integrations/probes` ← [`ProbeSettings`]
//! - `POST /integrations/{dns|cloudflare|netbird}/test` ← [`TokenForm`] (fehlendes Token = gespeichertes)
//! - `POST /integrations/probes/targets` ← [`ProbeTarget`], `DELETE /integrations/probes/targets/{id}`
//! - `POST /integrations/probes/test` ← [`ProbeTarget`]
//!
//! Änderungen wirken sofort: die Sammler hören auf `core.settings.subscribe()`.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use serde::de::DeserializeOwned;
use shared::{
    cloudflare::{CfStatus, CloudflareSettings},
    dns::{DnsSettings, DnsStatus, IntegrationState, IntegrationsView, TestResult, TokenForm},
    netbird::{NetbirdSettings, NetbirdStatus},
    probes::{ProbeKind, ProbeSettings, ProbeTarget, ProbesStatus},
};

use super::{ApiErr, ApiResult};
use crate::{
    collectors::{cloudflare, dns, netbird, probes},
    core::Core,
};

pub fn routes() -> Router<Core> {
    Router::new()
        .route("/integrations", get(view))
        .route("/integrations/dns", put(put_dns))
        .route("/integrations/cloudflare", put(put_cloudflare))
        .route("/integrations/netbird", put(put_netbird))
        .route("/integrations/probes", put(put_probes))
        .route("/integrations/probes/targets", post(add_target))
        .route("/integrations/probes/targets/{id}", delete(remove_target))
        .route("/integrations/probes/test", post(test_target))
        .route("/integrations/{name}/test", post(test))
}

fn snap<T: DeserializeOwned + Default>(core: &Core, key: &str) -> Option<T> {
    core.snaps.get(key).and_then(|s| serde_json::from_value(s.data.clone()).ok())
}

/// Neues Token übernehmen: `None` = unverändert, sonst ersetzen (leer = löschen).
fn merge_token(old: &str, new: &Option<String>) -> String {
    match new {
        None => old.to_string(),
        Some(t) => t.trim().to_string(),
    }
}

fn dns_form(core: &Core) -> TokenForm {
    let s = core.settings.get::<DnsSettings>("dns");
    let e = dns::effective_from(&s);
    TokenForm {
        enabled: s.enabled,
        url: e.url,
        token: None,
        token_set: !e.token.is_empty(),
        token_from_env: e.token_from_env,
        retention_days: e.retention_days,
        ..TokenForm::default()
    }
}

fn cf_form(core: &Core) -> TokenForm {
    let s = core.settings.get::<CloudflareSettings>("cloudflare");
    let e = cloudflare::effective_from(&s);
    TokenForm {
        enabled: s.enabled,
        token: None,
        token_set: !e.token.is_empty(),
        token_from_env: e.token_from_env,
        zones: s.zones,
        account_id: s.account_id,
        ..TokenForm::default()
    }
}

fn nb_form(core: &Core) -> TokenForm {
    let s = core.settings.get::<NetbirdSettings>("netbird");
    let e = netbird::effective_from(&s);
    TokenForm { enabled: s.enabled, url: e.url, token: None, token_set: !e.token.is_empty(), token_from_env: e.token_from_env, ..TokenForm::default() }
}

async fn view(State(core): State<Core>) -> ApiResult<IntegrationsView> {
    let dns_state = snap::<DnsStatus>(&core, "dns.status").map(|s| IntegrationState {
        configured: s.configured,
        ok: s.connected,
        error: s.error.clone(),
        last_ok: s.last_poll,
        summary: if s.configured || s.demo {
            format!("{} Anfragen gespeichert, {:.1} Bytes je Anfrage", s.stored, s.bytes_per_query)
        } else {
            String::new()
        },
    });
    let cf_state = snap::<CfStatus>(&core, "cloudflare.status").map(|s| IntegrationState {
        configured: s.configured,
        ok: s.ok,
        error: s.error.clone(),
        last_ok: s.last_poll,
        summary: if s.configured || s.demo { format!("{} Zonen, {} Tunnel", s.zones.len(), s.tunnels.len()) } else { String::new() },
    });
    let nb_state = snap::<NetbirdStatus>(&core, "netbird.status").map(|s| IntegrationState {
        configured: s.configured,
        ok: s.ok,
        error: s.error.clone(),
        last_ok: s.last_poll,
        summary: if s.configured || s.demo { format!("{} Peers, {} verbunden", s.peers, s.connected) } else { String::new() },
    });
    let pr_state = snap::<ProbesStatus>(&core, "probes.status").map(|s| {
        let up = s.targets.iter().filter(|t| t.up).count();
        IntegrationState {
            configured: true,
            ok: up == s.targets.len(),
            error: s.icmp_error.clone(),
            last_ok: 0,
            summary: format!("{} Ziele, {up} erreichbar", s.targets.len()),
        }
    });
    Ok(Json(IntegrationsView {
        demo: core.cfg.demo,
        dns: dns_form(&core),
        dns_state: dns_state.unwrap_or_default(),
        cloudflare: cf_form(&core),
        cloudflare_state: cf_state.unwrap_or_default(),
        netbird: nb_form(&core),
        netbird_state: nb_state.unwrap_or_default(),
        probes: core.settings.get::<ProbeSettings>("probes"),
        probes_state: pr_state.unwrap_or_default(),
    }))
}

fn check_url(url: &str) -> Result<(), ApiErr> {
    let u = url.trim();
    if !u.is_empty() && !(u.starts_with("https://") || u.starts_with("http://")) {
        return Err(ApiErr::bad("Adresse muss mit https:// oder http:// beginnen."));
    }
    Ok(())
}

fn dns_settings(core: &Core, f: &TokenForm) -> Result<DnsSettings, ApiErr> {
    check_url(&f.url)?;
    let old = core.settings.get::<DnsSettings>("dns");
    let url = f.url.trim();
    Ok(DnsSettings {
        enabled: f.enabled,
        // Standard nicht festschreiben – dann wirkt eine spätere Umgebungsvariable.
        url: if url == shared::dns::DEFAULT_MCP_URL && old.url.is_empty() { String::new() } else { url.to_string() },
        token: merge_token(&old.token, &f.token),
        retention_days: if f.retention_days == 0 { old.retention_days } else { f.retention_days.clamp(1, 3650) },
    })
}

fn cf_settings(core: &Core, f: &TokenForm) -> CloudflareSettings {
    let old = core.settings.get::<CloudflareSettings>("cloudflare");
    CloudflareSettings {
        enabled: f.enabled,
        token: merge_token(&old.token, &f.token),
        zones: f.zones.iter().map(|z| z.trim().to_string()).filter(|z| !z.is_empty()).collect(),
        account_id: f.account_id.trim().to_string(),
    }
}

fn nb_settings(core: &Core, f: &TokenForm) -> Result<NetbirdSettings, ApiErr> {
    check_url(&f.url)?;
    let old = core.settings.get::<NetbirdSettings>("netbird");
    let url = f.url.trim().trim_end_matches('/');
    Ok(NetbirdSettings {
        enabled: f.enabled,
        url: if url == shared::netbird::DEFAULT_URL && old.url.is_empty() { String::new() } else { url.to_string() },
        token: merge_token(&old.token, &f.token),
    })
}

async fn put_dns(State(core): State<Core>, Json(f): Json<TokenForm>) -> ApiResult<IntegrationsView> {
    let s = dns_settings(&core, &f)?;
    core.settings.set("dns", &s)?;
    view(State(core)).await
}

async fn put_cloudflare(State(core): State<Core>, Json(f): Json<TokenForm>) -> ApiResult<IntegrationsView> {
    core.settings.set("cloudflare", &cf_settings(&core, &f))?;
    view(State(core)).await
}

async fn put_netbird(State(core): State<Core>, Json(f): Json<TokenForm>) -> ApiResult<IntegrationsView> {
    let s = nb_settings(&core, &f)?;
    core.settings.set("netbird", &s)?;
    view(State(core)).await
}

/// Ziel prüfen und vervollständigen (ID aus dem Namen).
fn clean_target(t: &ProbeTarget, taken: &[String]) -> Result<ProbeTarget, ApiErr> {
    let mut t = t.clone();
    t.name = t.name.trim().to_string();
    t.host = t.host.trim().to_string();
    if t.host.is_empty() {
        return Err(ApiErr::bad("Ziel fehlt (Rechner, IP oder Adresse)."));
    }
    match t.kind {
        ProbeKind::Http if !(t.host.starts_with("https://") || t.host.starts_with("http://")) => {
            return Err(ApiErr::bad("HTTP-Ziel muss mit https:// oder http:// beginnen."));
        }
        ProbeKind::Tcp if t.port == 0 => return Err(ApiErr::bad("TCP-Ziel braucht einen Port.")),
        ProbeKind::Ping | ProbeKind::Http if t.host.contains(' ') => return Err(ApiErr::bad("Ziel enthält Leerzeichen.")),
        _ => {}
    }
    if t.name.is_empty() {
        t.name = probes::describe(&t);
    }
    if t.id.trim().is_empty() {
        let base = probes::make_id(&t.name);
        let mut id = base.clone();
        let mut n = 2;
        while taken.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        t.id = id;
    } else {
        t.id = probes::make_id(&t.id);
    }
    Ok(t)
}

async fn put_probes(State(core): State<Core>, Json(s): Json<ProbeSettings>) -> ApiResult<IntegrationsView> {
    let mut targets: Vec<ProbeTarget> = Vec::new();
    for t in &s.targets {
        let taken: Vec<String> = targets.iter().map(|t| t.id.clone()).collect();
        let t = clean_target(t, &taken)?;
        if taken.contains(&t.id) {
            return Err(ApiErr::bad(format!("ID `{}` doppelt.", t.id)));
        }
        targets.push(t);
    }
    core.settings.set("probes", &ProbeSettings { defaults: s.defaults, targets })?;
    view(State(core)).await
}

async fn add_target(State(core): State<Core>, Json(t): Json<ProbeTarget>) -> ApiResult<IntegrationsView> {
    let mut s = core.settings.get::<ProbeSettings>("probes");
    let mut taken: Vec<String> = s.targets.iter().map(|t| t.id.clone()).collect();
    taken.extend(["gateway", "cloudflare-dns", "quad9"].map(String::from));
    let mut t = clean_target(&ProbeTarget { id: String::new(), enabled: true, ..t }, &taken)?;
    t.enabled = true;
    s.targets.push(t);
    core.settings.set("probes", &s)?;
    view(State(core)).await
}

async fn remove_target(State(core): State<Core>, Path(id): Path<String>) -> ApiResult<IntegrationsView> {
    let mut s = core.settings.get::<ProbeSettings>("probes");
    let before = s.targets.len();
    s.targets.retain(|t| t.id != id);
    if s.targets.len() == before {
        return Err(ApiErr::not_found(format!("Kein eigenes Ziel `{id}`.")));
    }
    core.settings.set("probes", &s)?;
    view(State(core)).await
}

async fn test_target(Json(t): Json<ProbeTarget>) -> ApiResult<TestResult> {
    let t = clean_target(&t, &[])?;
    Ok(Json(probes::test(&t).await))
}

async fn test(State(core): State<Core>, Path(name): Path<String>, body: Option<Json<TokenForm>>) -> Result<Json<TestResult>, ApiErr> {
    let f = body.map(|Json(f)| f);
    let res = match name.as_str() {
        "dns" => {
            let s = match &f {
                Some(f) => dns_settings(&core, &TokenForm { enabled: true, ..f.clone() })?,
                None => core.settings.get("dns"),
            };
            dns::test(&core, &s).await
        }
        "cloudflare" => {
            let s = match &f {
                Some(f) => cf_settings(&core, &TokenForm { enabled: true, ..f.clone() }),
                None => core.settings.get("cloudflare"),
            };
            cloudflare::test(&core, &s).await
        }
        "netbird" => {
            let s = match &f {
                Some(f) => nb_settings(&core, &TokenForm { enabled: true, ..f.clone() })?,
                None => core.settings.get("netbird"),
            };
            netbird::test(&core, &s).await
        }
        _ => return Err(ApiErr(StatusCode::NOT_FOUND, format!("Unbekannte Integration `{name}`."))),
    };
    Ok(Json(res))
}
