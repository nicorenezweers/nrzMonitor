//! Typen des Bereichs `dns`: DNS über nrzNetwork (MCP) – Einstellungen, Abfrageprotokoll,
//! Ranglisten und die Momentaufnahme `dns.status`.

use serde::{Deserialize, Serialize};

/// Standard-Adresse des MCP-Servers von nrzNetwork.
pub const DEFAULT_MCP_URL: &str = "https://dns.zweers.ch/mcp";
/// Standard-Aufbewahrung des Abfrageprotokolls in Tagen.
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

/// Antwortquellen in Anzeigereihenfolge (wie `ResponseKind` in nrzNetwork).
pub const KINDS: &[(&str, &str)] = &[
    ("forwarded", "Weitergeleitet"),
    ("cached", "Cache"),
    ("blocked", "Gesperrt"),
    ("authoritative", "Eigene Zone"),
    ("local", "Lokal"),
    ("failed", "Fehlgeschlagen"),
    ("dropped", "Verworfen"),
];

/// Anzeigename einer Antwortquelle.
#[must_use]
pub fn kind_label(kind: &str) -> &str {
    KINDS.iter().find(|(k, _)| *k == kind).map_or(kind, |(_, l)| l)
}

/// Einstellungen (`core.settings` Schlüssel `dns`). Das Token verlässt den Server nie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsSettings {
    pub enabled: bool,
    /// MCP-Endpunkt, z. B. `https://dns.zweers.ch/mcp` (leer = `NRZMON_DNS_MCP_URL` oder Standard).
    pub url: String,
    /// Bearer-Token aus nrzNetwork → Einstellungen → MCP (leer = `NRZMON_DNS_MCP_TOKEN`).
    pub token: String,
    /// Tage, die das Abfrageprotokoll aufbewahrt wird.
    pub retention_days: u32,
}

impl Default for DnsSettings {
    fn default() -> Self {
        Self { enabled: true, url: String::new(), token: String::new(), retention_days: DEFAULT_RETENTION_DAYS }
    }
}

/// Eine DNS-Anfrage, wie nrzMonitor sie speichert.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QueryEntry {
    /// Unix-Millisekunden.
    pub ts: i64,
    /// Nummer in nrzNetwork (zum Erkennen von Doppelten).
    pub id: i64,
    pub client: String,
    /// `udp`, `tcp`, `https`
    pub proto: String,
    pub name: String,
    pub qtype: String,
    pub rcode: String,
    /// Antwortquelle: siehe [`KINDS`].
    pub kind: String,
    pub answer: String,
    /// Antwortzeit in Millisekunden.
    pub ms: u32,
    pub upstream: String,
}

/// Filter für `GET /api/v1/dns/queries` (und teilweise `top`, `timeline`, `clients`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QueryFilter {
    /// Unix-Sekunden (einschliesslich).
    pub from: Option<i64>,
    /// Unix-Sekunden (ausschliesslich).
    pub to: Option<i64>,
    /// Genau dieser Client.
    pub client: String,
    /// Teil des Namens (ohne Gross-/Kleinschreibung).
    pub name: String,
    pub qtype: String,
    pub kind: String,
    pub rcode: String,
    pub upstream: String,
    /// Ab 1.
    pub page: u32,
    /// 1–500 (Standard 100).
    pub per_page: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QueryPage {
    /// Neueste zuerst.
    pub entries: Vec<QueryEntry>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

/// Eintrag einer Rangliste.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TopEntry {
    pub name: String,
    pub count: u64,
    /// Davon gesperrt.
    pub blocked: u64,
}

/// `GET /api/v1/dns/top`
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsTop {
    pub total: u64,
    pub blocked: u64,
    pub cached: u64,
    pub failed: u64,
    pub clients_count: u64,
    pub domains_count: u64,
    /// Mittlere Antwortzeit in Millisekunden.
    pub avg_ms: f64,
    pub clients: Vec<TopEntry>,
    pub domains: Vec<TopEntry>,
    pub blocked_domains: Vec<TopEntry>,
    pub qtypes: Vec<TopEntry>,
    pub upstreams: Vec<TopEntry>,
    pub kinds: Vec<TopEntry>,
    pub rcodes: Vec<TopEntry>,
}

/// Ein Abschnitt des Verlaufs (`GET /api/v1/dns/timeline`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TimelinePoint {
    /// Beginn des Abschnitts (Unix-Sekunden).
    pub ts: i64,
    pub total: u64,
    pub blocked: u64,
    pub cached: u64,
    pub forwarded: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Timeline {
    /// Sekunden je Abschnitt.
    pub step: u32,
    pub points: Vec<TimelinePoint>,
}

/// Zeile der Client-Liste (`GET /api/v1/dns/clients`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientRow {
    pub client: String,
    pub queries: u64,
    pub blocked: u64,
    /// Verschiedene Namen.
    pub domains: u64,
    /// Unix-Millisekunden der letzten Anfrage.
    pub last: i64,
    pub avg_ms: f64,
}

/// Alles zu einer Domain (`GET /api/v1/dns/domain?name=`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DomainInfo {
    pub name: String,
    pub queries: u64,
    pub blocked: u64,
    /// Unix-Millisekunden.
    pub first: i64,
    pub last: i64,
    pub avg_ms: f64,
    pub clients: Vec<TopEntry>,
    pub qtypes: Vec<TopEntry>,
    pub kinds: Vec<TopEntry>,
    pub rcodes: Vec<TopEntry>,
    /// Häufigste Antworten.
    pub answers: Vec<TopEntry>,
    pub timeline: Timeline,
}

/// Weiterleitung (vorgelagerter Server) laut nrzNetwork.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpstreamInfo {
    pub upstream: String,
    pub name: String,
    pub ok: u64,
    pub failed: u64,
    pub avg_ms: f64,
    pub last_error: Option<String>,
    pub last_error_at: Option<i64>,
}

/// Sperrliste laut nrzNetwork.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BlocklistInfo {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub domains: u64,
    /// Unix-Sekunden.
    pub updated_at: Option<i64>,
    pub error: Option<String>,
}

/// Momentaufnahme `dns.status`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsStatus {
    /// Token und Adresse sind hinterlegt.
    pub configured: bool,
    /// Beispieldaten (nichts eingerichtet).
    pub demo: bool,
    /// Letzte Abfrage hat geklappt.
    pub connected: bool,
    pub error: Option<String>,
    pub url: String,
    /// Unix-Millisekunden der letzten erfolgreichen Abfrage.
    pub last_poll: i64,
    /// Bis hierhin ist das Protokoll lückenlos abgeholt (Unix-Sekunden).
    pub synced_until: i64,
    /// Es wird gerade ein Rückstand aufgeholt.
    pub catching_up: bool,
    /// Gespeicherte Anfragen (alle Tage).
    pub stored: u64,
    /// Belegter Platz (Datenbank-Anteil geschätzt + Parquet-Dateien).
    pub stored_bytes: u64,
    /// Bytes je gespeicherter Anfrage (in den abgeschlossenen Tagen gemessen).
    pub bytes_per_query: f64,
    /// Ältester gespeicherter Eintrag (Unix-Millisekunden).
    pub oldest: i64,
    pub retention_days: u32,
    /// MCP-Aufrufe seit dem Start und übertragene Bytes.
    pub calls: u64,
    pub rx_bytes: u64,

    // Zustand von nrzNetwork (`get_server_status`, `get_blocking_status`)
    pub version: String,
    pub uptime_secs: u64,
    pub queries_per_second: f64,
    pub cache_entries: u64,
    pub zones: u32,
    pub blocked_domains: u64,
    pub blocking_enabled: bool,
    /// Unix-Sekunden.
    pub paused_until: Option<i64>,
    pub custom_blocked: u32,
    pub allowed: u32,
    pub upstreams: Vec<UpstreamInfo>,
    pub blocklists: Vec<BlocklistInfo>,
    pub listen: Vec<String>,
    pub listen_errors: Vec<String>,
}

// --- Integrationen (gemeinsam für DNS, Cloudflare, NetBird, Erreichbarkeit) -----------------------

/// Zustand einer Integration für die Einstellungen.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationState {
    /// Zugangsdaten hinterlegt (Einstellungen oder Umgebung).
    pub configured: bool,
    /// Letzte Abfrage hat geklappt.
    pub ok: bool,
    pub error: Option<String>,
    /// Unix-Millisekunden der letzten erfolgreichen Abfrage.
    pub last_ok: i64,
    /// Kurz, z. B. „12 Peers, 9 verbunden“.
    pub summary: String,
}

/// Formular einer Integration mit Token: `GET` liefert `token: None` und `token_set`;
/// `PUT` nimmt `token: None` = unverändert, `Some("")` = löschen, `Some(t)` = neu.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TokenForm {
    pub enabled: bool,
    /// Adresse (DNS: MCP-Endpunkt, NetBird: Management-API; Cloudflare: leer).
    pub url: String,
    pub token: Option<String>,
    pub token_set: bool,
    /// Token kommt aus der Umgebung (`NRZMON_…`).
    pub token_from_env: bool,
    /// DNS: Aufbewahrung in Tagen.
    pub retention_days: u32,
    /// Cloudflare: nur diese Zonen (leer = alle gefundenen).
    pub zones: Vec<String>,
    /// Cloudflare: Konto-ID (leer = aus den Zonen).
    pub account_id: String,
}

/// `GET /api/v1/integrations`
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationsView {
    pub demo: bool,
    pub dns: TokenForm,
    pub dns_state: IntegrationState,
    pub cloudflare: TokenForm,
    pub cloudflare_state: IntegrationState,
    pub netbird: TokenForm,
    pub netbird_state: IntegrationState,
    pub probes: crate::probes::ProbeSettings,
    pub probes_state: IntegrationState,
}

/// Antwort auf `POST /api/v1/integrations/{name}/test`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
    /// Einzelheiten (gefundene Zonen, fehlende Rechte …).
    pub details: Vec<String>,
}
