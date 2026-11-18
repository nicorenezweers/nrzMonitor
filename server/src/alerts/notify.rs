//! Benachrichtigungen: ntfy, Webhook, Discord, Telegram. Geheimnisse kommen nur maskiert zurück.

use std::time::Duration;

use serde_json::{Value, json};
use shared::{
    alerts::{Channel, ChannelKind, MASK},
    events::Severity,
};

use crate::core::Core;

/// `••••abcd` (letzte vier Zeichen bei langen Werten), leer bleibt leer.
pub fn mask(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > 12 {
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{MASK}{tail}")
    } else {
        MASK.to_string()
    }
}

fn is_masked(s: &str) -> bool {
    s.starts_with(MASK)
}

/// URL enthält bei Discord und Webhook oft das Geheimnis.
fn url_secret(kind: ChannelKind) -> bool {
    matches!(kind, ChannelKind::Discord | ChannelKind::Webhook)
}

pub fn mask_channel(mut c: Channel) -> Channel {
    c.token = mask(&c.token);
    if url_secret(c.kind) {
        c.url = mask(&c.url);
    }
    c
}

/// Maskierte Felder durch die gespeicherten Werte desselben Kanals ersetzen.
pub fn unmask(mut c: Channel, stored: Option<&Channel>) -> Channel {
    let old = stored.filter(|o| o.kind == c.kind);
    if is_masked(&c.token) {
        c.token = old.map(|o| o.token.clone()).unwrap_or_default();
    }
    if is_masked(&c.url) {
        c.url = old.map(|o| o.url.clone()).unwrap_or_default();
    }
    c
}

pub fn tidy(mut c: Channel) -> Channel {
    for f in [&mut c.name, &mut c.url, &mut c.topic, &mut c.token, &mut c.chat_id] {
        *f = f.trim().to_string();
    }
    c.url = c.url.trim_end_matches('/').to_string();
    if c.kind == ChannelKind::Ntfy && c.url.is_empty() {
        c.url = "https://ntfy.sh".to_string();
    }
    if c.name.is_empty() {
        c.name = c.kind.label().to_string();
    }
    c
}

pub fn validate(c: &Channel) -> Result<(), String> {
    let url_ok = |u: &str| u.starts_with("http://") || u.starts_with("https://");
    match c.kind {
        ChannelKind::Ntfy => {
            if !url_ok(&c.url) {
                return Err("ntfy-Server muss mit http:// oder https:// beginnen".into());
            }
            if c.topic.is_empty() {
                return Err("Topic fehlt".into());
            }
        }
        ChannelKind::Discord | ChannelKind::Webhook => {
            if !url_ok(&c.url) {
                return Err("URL muss mit http:// oder https:// beginnen".into());
            }
        }
        ChannelKind::Telegram => {
            if c.token.is_empty() || c.chat_id.is_empty() {
                return Err("Bot-Token und Chat-ID nötig".into());
            }
        }
    }
    Ok(())
}

/// Eine Nachricht an die Kanäle.
#[derive(Debug, Clone)]
pub struct Message {
    pub title: String,
    pub body: String,
    pub severity: Severity,
    /// `raised`, `resolved`, `reminder`, `test`.
    pub status: &'static str,
    pub payload: Value,
}

/// An alle passenden Kanäle senden (im Hintergrund, je Kanal bis zu drei Versuche).
pub fn send_all(core: &Core, channels: &[Channel], msg: Message) {
    let targets: Vec<Channel> = channels
        .iter()
        .filter(|c| c.enabled && (msg.severity >= c.min_severity || msg.status == "resolved"))
        .cloned()
        .collect();
    if targets.is_empty() {
        return;
    }
    let http = core.http.clone();
    tokio::spawn(async move {
        for c in targets {
            let mut wait = 5;
            for attempt in 1..=3 {
                match send(&http, &c, &msg).await {
                    Ok(()) => break,
                    Err(e) if attempt == 3 => tracing::warn!("Benachrichtigung über „{}“ fehlgeschlagen: {e}", c.name),
                    Err(_) => {
                        tokio::time::sleep(Duration::from_secs(wait)).await;
                        wait *= 4;
                    }
                }
            }
        }
    });
}

pub async fn send(http: &reqwest::Client, c: &Channel, msg: &Message) -> Result<(), String> {
    let icon = match (msg.status, msg.severity) {
        ("resolved", _) => "✅",
        ("test", _) => "🔔",
        (_, Severity::Crit) => "🔴",
        _ => "🟠",
    };
    let title = format!("{icon} {}", msg.title);
    let req = match c.kind {
        ChannelKind::Ntfy => {
            let mut r = http.post(&c.url).json(&json!({
                "topic": c.topic,
                "title": msg.title,
                "message": msg.body,
                "tags": [match (msg.status, msg.severity) {
                    ("resolved", _) => "white_check_mark",
                    ("test", _) => "bell",
                    (_, Severity::Crit) => "rotating_light",
                    _ => "warning",
                }],
                "priority": match (msg.status, msg.severity) { ("resolved" | "test", _) => 3, (_, Severity::Crit) => 5, _ => 4 },
            }));
            if !c.token.is_empty() {
                r = r.bearer_auth(&c.token);
            }
            r
        }
        ChannelKind::Discord => http.post(&c.url).json(&json!({ "content": format!("**{title}**\n{}", msg.body) })),
        ChannelKind::Telegram => http
            .post(format!("https://api.telegram.org/bot{}/sendMessage", c.token))
            .json(&json!({ "chat_id": c.chat_id, "text": format!("{title}\n{}", msg.body) })),
        ChannelKind::Webhook => {
            let mut body = json!({ "title": msg.title, "message": msg.body, "status": msg.status, "severity": msg.severity });
            if let (Some(obj), Value::Object(extra)) = (body.as_object_mut(), &msg.payload) {
                obj.extend(extra.clone());
            }
            http.post(&c.url).json(&body)
        }
    };
    let res = req.timeout(Duration::from_secs(15)).send().await.map_err(|e| {
        // Nie Token oder Webhook-URL in Fehlermeldungen.
        if e.is_timeout() { "Zeitüberschreitung".to_string() } else { "Verbindung fehlgeschlagen".to_string() }
    })?;
    let status = res.status();
    if status.is_success() {
        Ok(())
    } else {
        let text: String = res.text().await.unwrap_or_default().chars().take(200).collect();
        Err(format!("HTTP {status}: {text}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masking_roundtrip() {
        assert_eq!(mask(""), "");
        assert_eq!(mask("short"), MASK);
        assert_eq!(mask("1234567890:ABCDEFGH"), format!("{MASK}EFGH"));
        let stored = Channel {
            id: "a".into(),
            kind: ChannelKind::Discord,
            url: "https://discord.com/api/webhooks/1/secret".into(),
            token: "tok".into(),
            ..Channel::default()
        };
        let masked = mask_channel(stored.clone());
        assert!(is_masked(&masked.url) && is_masked(&masked.token));
        assert_eq!(unmask(masked.clone(), Some(&stored)), stored);
        let fresh = unmask(masked, None);
        assert!(fresh.url.is_empty() && fresh.token.is_empty());
        assert!(validate(&tidy(Channel { topic: "x".into(), ..Channel::default() })).is_ok(), "ntfy.sh als Standard");
    }
}
