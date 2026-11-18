//! API des Bereichs `system` (noch leer – Routen hier ergänzen).

use axum::Router;

use crate::core::Core;

pub fn routes() -> Router<Core> {
    Router::new()
}
