//! The Deskmate V2 server binary: reads its configuration from the
//! environment and serves the device-facing router until it receives
//! `SIGINT`/`SIGTERM`, per the deployment contract in `deploy/README.md`.

use std::path::PathBuf;

use server::firmware::FirmwareCatalog;
use server::{ServerState, app};

// Loopback, not `0.0.0.0`: per `deploy/README.md` §4, a Cloudflare Tunnel is
// the *only* sanctioned ingress. A default that binds every interface would
// mean setting only `DESKMATE_ADMIN_TOKEN` (skipping `DESKMATE_SERVER_BIND`)
// silently publishes plaintext HTTP -- no TLS, tunnel bypassed entirely.
const DEFAULT_BIND_ADDRESS: &str = "127.0.0.1:8443";
// Absolute, matching `deskmate-server.env.example`: launchd's plist sets no
// `WorkingDirectory`, and systemd's is unit-manager-defined, so a relative
// default would resolve against whatever directory happens to be current --
// unspecified in practice. Both directory variables are checked below and the
// process refuses to start with a relative override for the same reason.
const DEFAULT_FIRMWARE_DIR: &str = "/var/lib/deskmate/firmware";
const DEFAULT_CONFIG_DIR: &str = "/var/lib/deskmate/configs";
const DEFAULT_FIRMWARE_VERSION: &str = "1.0.0";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let bind_address =
        std::env::var("DESKMATE_SERVER_BIND").unwrap_or_else(|_| DEFAULT_BIND_ADDRESS.to_string());
    let firmware_dir = std::env::var("DESKMATE_FIRMWARE_DIR")
        .map_or_else(|_| PathBuf::from(DEFAULT_FIRMWARE_DIR), PathBuf::from);
    assert!(
        firmware_dir.is_absolute(),
        "DESKMATE_FIRMWARE_DIR must be an absolute path (got {}); a relative \
         path resolves against the process's working directory, which is \
         unspecified under launchd/systemd",
        firmware_dir.display()
    );
    let config_dir = std::env::var("DESKMATE_CONFIG_DIR")
        .map_or_else(|_| PathBuf::from(DEFAULT_CONFIG_DIR), PathBuf::from);
    assert!(
        config_dir.is_absolute(),
        "DESKMATE_CONFIG_DIR must be an absolute path (got {}); a relative \
         path resolves against the process's working directory, which is \
         unspecified under launchd/systemd",
        config_dir.display()
    );
    let firmware_version = std::env::var("DESKMATE_FIRMWARE_VERSION")
        .unwrap_or_else(|_| DEFAULT_FIRMWARE_VERSION.to_string());
    let admin_token = std::env::var("DESKMATE_ADMIN_TOKEN")
        .expect("DESKMATE_ADMIN_TOKEN must be set -- see deploy/README.md");

    let state = ServerState::new(
        admin_token,
        FirmwareCatalog::new(firmware_dir, firmware_version),
        config_dir,
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
