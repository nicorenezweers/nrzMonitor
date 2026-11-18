//! API des Bereichs `alerts`.
//!
//! - `GET /alerts/active` → `Vec<ActiveAlert>`
//! - `GET /alerts/rules` → `Vec<RuleStatus>`; `PUT /alerts/rules` (Regel speichern),
//!   `POST /alerts/rules/{id}/reset` (Standard wiederherstellen), `DELETE /alerts/rules/{id}` (eigene löschen)
//! - `GET/PUT /alerts/config` → `NotifyConfig` (Kanäle maskiert); `POST /alerts/test` (Kanal prüfen)

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use shared::{
    alerts::{ActiveAlert, AlertConfig, Channel, Expr, NotifyConfig, Rule, RuleKind, RuleStatus, TestResult},
    events::Severity,
    model::Unit,
};

use super::{ApiErr, ApiResult};
use crate::{
    alerts::{self, notify, rules},
    core::Core,
};

pub fn routes() -> Router<Core> {
    Router::new()
        .route("/alerts/active", get(active))
        .route("/alerts/rules", get(list_rules).put(save_rule))
        .route("/alerts/rules/{id}", axum::routing::delete(delete_rule))
        .route("/alerts/rules/{id}/reset", post(reset_rule))
        .route("/alerts/config", get(get_config).put(put_config))
        .route("/alerts/test", post(test_channel))
}

fn config(core: &Core) -> AlertConfig {
    core.settings.get("alerts")
}

async fn active(State(core): State<Core>) -> ApiResult<Vec<ActiveAlert>> {
    let list = core.snaps.get("alerts.active").and_then(|s| serde_json::from_value(s.data.clone()).ok()).unwrap_or_default();
    Ok(Json(list))
}

fn unit_of(core: &Core, r: &Rule) -> Unit {
    match r.kind {
        RuleKind::State => match r.check {
            shared::alerts::StateCheck::MemLimit => Unit::Percent,
            shared::alerts::StateCheck::NoData => Unit::Seconds,
            _ => Unit::Count,
        },
        RuleKind::Metric => {
            if Expr::parse(&r.expr).is_ok_and(|e| e.is_ratio()) {
                return Unit::Percent;
            }
            let first = r.context.split(',').map(str::trim).find(|c| !c.is_empty()).unwrap_or("");
            shared::catalog::find(first)
                .map(|t| t.unit)
                .or_else(|| core.registry.list(false).into_iter().find(|m| r.matches_context(&m.context)).map(|m| m.unit))
                .unwrap_or_default()
        }
    }
}

async fn list_rules(State(core): State<Core>) -> ApiResult<Vec<RuleStatus>> {
    let list = rules::effective(&config(&core))
        .into_iter()
        .map(|rule| {
            let (firing, checked, error) = alerts::rule_status(&rule.id);
            RuleStatus { unit: unit_of(&core, &rule), firing, checked, error, rule }
        })
        .collect();
    Ok(Json(list))
}

fn validate(r: &Rule) -> Result<(), ApiErr> {
    if r.name.trim().is_empty() {
        return Err(ApiErr::bad("Name fehlt"));
    }
    if r.kind == RuleKind::Metric {
        if r.context.trim().is_empty() {
            return Err(ApiErr::bad("Kontext fehlt"));
        }
        Expr::parse(&r.expr).map_err(|e| ApiErr::bad(format!("Ausdruck: {e}")))?;
        if r.warn.is_none() && r.crit.is_none() {
            return Err(ApiErr::bad("Mindestens eine Schwelle angeben"));
        }
    }
    for v in [r.warn, r.crit].into_iter().flatten() {
        if !v.is_finite() {
            return Err(ApiErr::bad("Schwelle ist keine Zahl"));
        }
    }
    if r.window > 86_400 || r.duration > 86_400 {
        return Err(ApiErr::bad("Fenster und Dauer höchstens 24 h"));
    }
    Ok(())
}

async fn save_rule(State(core): State<Core>, Json(mut rule): Json<Rule>) -> ApiResult<RuleStatus> {
    let mut cfg = config(&core);
    let builtin = rules::builtin();
    if let Some(base) = builtin.iter().find(|b| b.id == rule.id) {
        // Standardregel: nur änderbare Felder übernehmen.
        let mut r = base.clone();
        rules::apply(&mut r, &rule);
        rule = r;
    } else {
        rule.builtin = false;
        if rule.id.is_empty() || !cfg.rules.iter().any(|x| x.id == rule.id) {
            rule.id = format!("custom-{}", crate::util::random_hex(4));
        }
        if rule.group.trim().is_empty() {
            rule.group = "Eigene".to_string();
        }
    }
    rule.name = rule.name.trim().to_string();
    rule.context = rule.context.trim().to_string();
    rule.expr = rule.expr.trim().to_string();
    if rule.severity < Severity::Warn {
        rule.severity = Severity::Warn;
    }
    validate(&rule)?;
    match cfg.rules.iter_mut().find(|x| x.id == rule.id) {
        Some(slot) => *slot = rule.clone(),
        None => cfg.rules.push(rule.clone()),
    }
    core.settings.set("alerts", &cfg)?;
    let (firing, checked, error) = alerts::rule_status(&rule.id);
    Ok(Json(RuleStatus { unit: unit_of(&core, &rule), firing, checked, error, rule }))
}

async fn reset_rule(State(core): State<Core>, Path(id): Path<String>) -> Result<StatusCode, ApiErr> {
    let mut cfg = config(&core);
    if !rules::builtin().iter().any(|b| b.id == id) {
        return Err(ApiErr::not_found("Keine Standardregel"));
    }
    cfg.rules.retain(|r| r.id != id);
    core.settings.set("alerts", &cfg)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_rule(State(core): State<Core>, Path(id): Path<String>) -> Result<StatusCode, ApiErr> {
    let mut cfg = config(&core);
    if rules::builtin().iter().any(|b| b.id == id) {
        return Err(ApiErr::bad("Standardregeln lassen sich nur abschalten"));
    }
    let before = cfg.rules.len();
    cfg.rules.retain(|r| r.id != id);
    if cfg.rules.len() == before {
        return Err(ApiErr::not_found("Regel nicht gefunden"));
    }
    core.settings.set("alerts", &cfg)?;
    Ok(StatusCode::NO_CONTENT)
}

fn masked(cfg: &AlertConfig) -> NotifyConfig {
    NotifyConfig {
        channels: cfg.channels.iter().cloned().map(notify::mask_channel).collect(),
        repeat_minutes: cfg.repeat_minutes,
        max_repeats: cfg.max_repeats,
        notify_resolved: cfg.notify_resolved,
    }
}

async fn get_config(State(core): State<Core>) -> ApiResult<NotifyConfig> {
    Ok(Json(masked(&config(&core))))
}

async fn put_config(State(core): State<Core>, Json(n): Json<NotifyConfig>) -> ApiResult<NotifyConfig> {
    let mut cfg = config(&core);
    let mut channels = Vec::with_capacity(n.channels.len());
    for c in n.channels {
        let stored = cfg.channels.iter().find(|x| !c.id.is_empty() && x.id == c.id);
        let mut c = notify::tidy(notify::unmask(c, stored));
        notify::validate(&c).map_err(|e| ApiErr::bad(format!("{}: {e}", c.name)))?;
        if c.id.is_empty() || stored.is_none() {
            c.id = crate::util::random_hex(4);
        }
        channels.push(c);
    }
    cfg.channels = channels;
    cfg.repeat_minutes = n.repeat_minutes.min(24 * 60);
    cfg.max_repeats = n.max_repeats.min(50);
    cfg.notify_resolved = n.notify_resolved;
    core.settings.set("alerts", &cfg)?;
    Ok(Json(masked(&cfg)))
}

async fn test_channel(State(core): State<Core>, Json(c): Json<Channel>) -> ApiResult<TestResult> {
    let cfg = config(&core);
    let stored = cfg.channels.iter().find(|x| !c.id.is_empty() && x.id == c.id);
    let c = notify::tidy(notify::unmask(c, stored));
    if let Err(e) = notify::validate(&c) {
        return Ok(Json(TestResult { ok: false, message: e }));
    }
    let msg = notify::Message {
        title: "nrzMonitor: Test".to_string(),
        body: format!("Testnachricht von {} – der Kanal funktioniert.", core.hostname),
        severity: Severity::Info,
        status: "test",
        payload: serde_json::json!({ "event": "test", "host": core.hostname }),
    };
    Ok(Json(match notify::send(&core.http, &c, &msg).await {
        Ok(()) => TestResult { ok: true, message: format!("Testnachricht an „{}“ gesendet", c.name) },
        Err(e) => TestResult { ok: false, message: e },
    }))
}
