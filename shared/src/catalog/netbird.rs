//! Vorlagen für NetBird (Management-API).

use super::{Tpl, d};
use crate::model::{ChartKind::*, Unit::*};

pub const STEP: u32 = 30;

pub const NETBIRD_PEERS: Tpl = Tpl::new(
    "netbird.peers", "netbird", "Peers", Count, Stacked, &[d("connected", "Verbunden"), d("disconnected", "Getrennt")], 100,
).every(STEP);
pub const NETBIRD_INVENTORY: Tpl = Tpl::new(
    "netbird.inventory", "netbird", "Bestand", Count, Line,
    &[
        d("users", "Benutzer"), d("groups", "Gruppen"), d("policies", "Regeln"), d("routes", "Routen"),
        d("networks", "Netzwerke"), d("setup_keys", "Setup-Schlüssel"), d("nameservers", "DNS-Server"),
    ],
    110,
).every(300);
pub const NETBIRD_PEER_ONLINE: Tpl = Tpl::new(
    "netbird.peer.online", "netbird", "Verbunden", State, Area, &[d("online", "Verbunden")], 200,
).per("peer").every(STEP);
pub const NETBIRD_EVENTS: Tpl = Tpl::new(
    "netbird.events", "netbird", "Aktivität", PerMin, Area, &[d("events", "Ereignisse")], 120,
).every(60);

pub const ALL: &[Tpl] = &[NETBIRD_PEERS, NETBIRD_INVENTORY, NETBIRD_PEER_ONLINE, NETBIRD_EVENTS];
