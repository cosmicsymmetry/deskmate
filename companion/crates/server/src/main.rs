//! The Deskmate V2 server binary: reads its configuration from the
//! environment and serves the device-facing router until it receives
//! `SIGINT`/`SIGTERM`, per the deployment contract in `deploy/README.md`.

use std::path::{Path, PathBuf};

use server::data_cards;
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
// unspecified in practice. The config directory owns both per-device configs
// and the digest-only identity registry. Both directory variables are checked
// below and the process refuses to start with a relative override for the same
// reason.
const DEFAULT_FIRMWARE_DIR: &str = "/var/lib/deskmate/firmware";
const DEFAULT_CONFIG_DIR: &str = "/var/lib/deskmate/configs";

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
    let data_card_spec_path = data_card_spec_path(&config_dir);
    let firmware_version = required_firmware_version(std::env::var("DESKMATE_FIRMWARE_VERSION"));
    let admin_token = std::env::var("DESKMATE_ADMIN_TOKEN")
        .expect("DESKMATE_ADMIN_TOKEN must be set -- see deploy/README.md");
    let state = ServerState::new(
        admin_token,
        FirmwareCatalog::new(firmware_dir, firmware_version),
        config_dir,
    );

    // Server-rendered data cards, if this deployment has any. The specs are
    // read before the listener binds so a malformed file fails the start
    // rather than leaving a server up with silently missing cards.
    //
    // A refresh task is spawned per card and lives as long as the process; the
    // graceful shutdown below drains connections, and an in-flight fetch is
    // abandoned with it. That is safe because a card's durable state is the
    // frame already in the store: losing a refresh loses nothing but the tick.
    let data_card_specs = data_cards::load_specs(&data_card_spec_path)
        .unwrap_or_else(|error| panic!("DESKMATE_DATA_CARDS is unreadable: {error}"));
    if data_card_specs.is_empty() {
        tracing::info!("no server-rendered data cards configured");
    } else {
        data_cards::spawn_refreshers(&state, data_card_specs);
    }

    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .unwrap_or_else(|error| panic!("failed to bind {bind_address}: {error}"));
    let local_addr = listener
        .local_addr()
        .expect("a bound listener has a local address");

    println!("deskmate server listening on {local_addr}");
    tracing::info!(address = %local_addr, "deskmate server listening");

    let shutdown_state = state.clone();
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server exited with an error");
    tokio::task::spawn_blocking(move || shutdown_state.shutdown())
        .await
        .expect("device runtime shutdown worker panicked");
}

fn required_firmware_version(value: Result<String, std::env::VarError>) -> String {
    value.expect(
        "DESKMATE_FIRMWARE_VERSION must be set to the published image's exact \
         firmware/version.txt value -- see deploy/README.md",
    )
}

/// Resolves once `SIGINT` (Ctrl-C) or, on Unix, `SIGTERM` is received, so
/// Where the server-rendered card specs live.
///
/// `DESKMATE_DATA_CARDS` overrides it; the default sits beside the rest of the
/// server's state in the config directory, so a deployment that backs that up
/// backs up its cards too.
fn data_card_spec_path(config_dir: &Path) -> PathBuf {
    std::env::var("DESKMATE_DATA_CARDS")
        .map_or_else(|_| config_dir.join("data-cards.json"), PathBuf::from)
}

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

#[cfg(test)]
mod tests {
    #[test]
    fn missing_firmware_version_names_the_authoritative_file_and_runbook() {
        let panic = std::panic::catch_unwind(|| {
            super::required_firmware_version(Err(std::env::VarError::NotPresent));
        })
        .expect_err("a missing firmware version must stop startup");
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .expect("startup panic carries text");
        assert!(message.contains("DESKMATE_FIRMWARE_VERSION"));
        assert!(message.contains("firmware/version.txt"));
        assert!(message.contains("deploy/README.md"));
    }
}
