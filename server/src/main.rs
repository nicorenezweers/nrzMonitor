//! nrzMonitor – überwacht den Host, alle Container und das Netzwerk; speichert sparsam in DuckDB.

// Der Kern stellt Bausteine für alle Bereiche bereit; nicht jeder wird überall gebraucht.
#![allow(dead_code)]

mod alerts;
mod api;
mod auth;
mod collectors;
mod config;
mod core;
mod db;
mod frame;
mod live;
mod mcp;
mod registry;
mod ring;
mod settings;
mod snaps;
mod store;
mod util;
mod web;

use std::{net::SocketAddr, time::Duration};

use config::Config;

fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        // Docker-Healthcheck: nimmt der Server Verbindungen an?
        let mut addr = cfg.bind[0];
        if addr.ip().is_unspecified() {
            addr.set_ip([127, 0, 0, 1].into());
        }
        let ok = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3)).is_ok();
        std::process::exit(i32::from(!ok));
    }

    if std::env::args().nth(1).as_deref() == Some("mcp") {
        // MCP über stdin/stdout (z. B. `docker exec -i nrzmonitor nrzmonitor mcp`).
        return mcp::stdio(cfg);
    }

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .compact()
        .init();

    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().thread_name("nrz-async").build()?;
    runtime.block_on(run(cfg))
}

async fn run(cfg: Config) -> anyhow::Result<()> {
    tracing::info!(
        "nrzMonitor {} startet – Daten: {}, Takt: {} s{}",
        env!("CARGO_PKG_VERSION"),
        cfg.data_dir.display(),
        cfg.interval,
        if cfg.demo { ", Beispieldaten" } else { "" },
    );
    let core = core::Core::new(cfg)?;
    collectors::start(&core);
    alerts::start(&core);

    let app = api::router(core.clone());
    for addr in core.cfg.bind.clone() {
        let app = app.clone();
        tokio::spawn(async move {
            // Die Adresse kann beim Start noch fehlen (Docker-Netz) – darum wiederholen.
            let listener = loop {
                match tokio::net::TcpListener::bind(addr).await {
                    Ok(l) => break l,
                    Err(e) => {
                        tracing::warn!("{addr} nicht verfügbar ({e}) – neuer Versuch in 5 s");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            };
            tracing::info!("Oberfläche und API auf http://{addr}");
            if let Err(e) = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await {
                tracing::error!("Server auf {addr}: {e}");
            }
        });
    }

    shutdown_signal().await;
    tracing::info!("Beende – schreibe offene Daten");
    core.store.shutdown().await;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(t) => t,
            Err(_) => return std::future::pending().await,
        };
        tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}
