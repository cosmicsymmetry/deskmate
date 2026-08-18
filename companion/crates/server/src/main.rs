//! The Deskmate V2 server binary: reads its configuration from the
//! environment and serves the device-facing router until it receives
//! `SIGINT`/`SIGTERM`, per the deployment contract in `deploy/README.md`.

use std::path::PathBuf;

use server::firmware::FirmwareCatalog;
use server::{ServerState, app};

const DEFAULT_BIND_ADDRESS: &str = "0.0.0.0:8443";
const DEFAULT_FIRMWARE_DIR: &str = "firmware";
const DEFAULT_FIRMWARE_VERSION: &str = "1.0.0";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let bind_address =
        std::env::var("DESKMATE_SERVER_BIND").unwrap_or_else(|_| DEFAULT_BIND_ADDRESS.to_string());
    let firmware_dir = std::env::var("DESKMATE_FIRMWARE_DIR")
        .map_or_else(|_| PathBuf::from(DEFAULT_FIRMWARE_DIR), PathBuf::from);
    let firmware_version = std::env::var("DESKMATE_FIRMWARE_VERSION")
        .unwrap_or_else(|_| DEFAULT_FIRMWARE_VERSION.to_string());
    let admin_token = std::env::var("DESKMATE_ADMIN_TOKEN")
        .expect("DESKMATE_ADMIN_TOKEN must be set -- see deploy/README.md");

    let state = ServerState::new(
        admin_token,
        FirmwareCatalog::new(firmware_dir, firmware_version),
    );

    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .unwrap_or_else(|error| panic!("failed to bind {bind_address}: {error}"));
    let local_addr = listener
        .local_addr()
        .expect("a bound listener has a local address");

    println!("deskmate server listening on {local_addr}");
    tracing::info!(address = %local_addr, "deskmate server listening");

    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server exited with an error");
}

/// Resolves once `SIGINT` (Ctrl-C) or, on Unix, `SIGTERM` is received, so
/// `axum::serve` can drain in-flight connections before the process exits.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install the SIGINT handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install the SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }

    tracing::info!("shutdown signal received, draining connections");
}
