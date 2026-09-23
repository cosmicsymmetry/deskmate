//! The Deskmate server binary reads its configuration from the environment and
//! serves the device and companion routes until it receives `SIGINT`/`SIGTERM`,
//! following the deployment contract in `deploy/README.md`.

use std::path::PathBuf;
use std::sync::Arc;

use server::firmware::FirmwareCatalog;
use server::{ServerOptions, ServerState, app_with_web};

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

const ENV_GOOGLE_CLIENT_ID: &str = "DESKMATE_GOOGLE_CLIENT_ID";
const ENV_GOOGLE_CLIENT_SECRET: &str = "DESKMATE_GOOGLE_CLIENT_SECRET";
const ENV_GOOGLE_CLIENT_SECRET_FILE: &str = "DESKMATE_GOOGLE_CLIENT_SECRET_FILE";
const ENV_GOOGLE_REDIRECT_URI: &str = "DESKMATE_GOOGLE_REDIRECT_URI";
const ENV_GOOGLE_AUTH_URI: &str = "DESKMATE_GOOGLE_AUTH_URI";
const ENV_GOOGLE_TOKEN_URI: &str = "DESKMATE_GOOGLE_TOKEN_URI";
const ENV_GOOGLE_REVOKE_URI: &str = "DESKMATE_GOOGLE_REVOKE_URI";

/// Environment values are captured first so validation is pure and can be
/// tested without mutating process-global environment variables.
#[derive(Default)]
struct GoogleOAuthEnv {
    client_id: Option<String>,
    client_secret: Option<String>,
    /// Path form of the client secret, preferred over the inline value for the
    /// same reason `DESKMATE_SECRETS_KEY_FILE` is preferred over
    /// `DESKMATE_SECRETS_KEY`: an environment value is readable through
    /// `systemctl show --property=Environment`, `/proc/<pid>/environ`, and any
    /// crash dump. Spec section 6 requires the client secret to live in a keyed
    /// store or a `0600` file, not in a plaintext config.
    client_secret_file: Option<String>,
    redirect_uri: Option<String>,
    auth_uri: Option<String>,
    token_uri: Option<String>,
    revoke_uri: Option<String>,
}

impl GoogleOAuthEnv {
    fn read() -> Result<Self, GoogleOAuthConfigError> {
        Ok(Self {
            client_id: read_google_env(ENV_GOOGLE_CLIENT_ID)?,
            client_secret: read_google_env(ENV_GOOGLE_CLIENT_SECRET)?,
            client_secret_file: read_google_env(ENV_GOOGLE_CLIENT_SECRET_FILE)?,
            redirect_uri: read_google_env(ENV_GOOGLE_REDIRECT_URI)?,
            auth_uri: read_google_env(ENV_GOOGLE_AUTH_URI)?,
            token_uri: read_google_env(ENV_GOOGLE_TOKEN_URI)?,
            revoke_uri: read_google_env(ENV_GOOGLE_REVOKE_URI)?,
        })
    }

    fn is_absent(&self) -> bool {
        self.client_id.is_none()
            && self.client_secret.is_none()
            && self.client_secret_file.is_none()
            && self.redirect_uri.is_none()
            && self.auth_uri.is_none()
            && self.token_uri.is_none()
            && self.revoke_uri.is_none()
    }
}

/// Invalid Google OAuth startup configuration. Variants deliberately retain
/// only the environment-variable name, never the supplied value: one of those
/// values is the OAuth client secret.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
enum GoogleOAuthConfigError {
    #[error("{variable} is not valid Unicode")]
    NotUnicode { variable: &'static str },
    #[error("{variable} must be set when Google OAuth is configured")]
    Missing { variable: &'static str },
    #[error("{variable} must not be empty")]
    Empty { variable: &'static str },
    #[error("{variable} must be an absolute https:// URL")]
    InvalidUrl { variable: &'static str },
    #[error(
        "{ENV_GOOGLE_CLIENT_SECRET_FILE} could not be read: {detail}. \
         It must be an existing file readable only by this service"
    )]
    SecretFileUnreadable { detail: String },
    #[error(
        "{ENV_GOOGLE_CLIENT_SECRET_FILE} is reachable by group or other (mode {mode:04o}); \
         it holds an OAuth client secret and must be 0600"
    )]
    SecretFilePermissive { mode: u32 },
}

fn read_google_env(variable: &'static str) -> Result<Option<String>, GoogleOAuthConfigError> {
    match std::env::var(variable) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(GoogleOAuthConfigError::NotUnicode { variable })
        }
    }
}

/// Builds a `GoogleOAuthConfig` from the environment, or `None` when no Google
/// variable is set. Partial or malformed configuration stops startup rather
/// than being mistaken for an intentionally disabled integration.
fn google_oauth_config_from_env() -> Option<server::oauth::GoogleOAuthConfig> {
    let values = GoogleOAuthEnv::read()
        .unwrap_or_else(|error| panic!("invalid Google OAuth configuration: {error}"));
    google_oauth_config_from_values(values)
        .unwrap_or_else(|error| panic!("invalid Google OAuth configuration: {error}"))
}

fn google_oauth_config_from_values(
    values: GoogleOAuthEnv,
) -> Result<Option<server::oauth::GoogleOAuthConfig>, GoogleOAuthConfigError> {
    if values.is_absent() {
        return Ok(None);
    }

    let mut config = server::oauth::GoogleOAuthConfig {
        client_id: required_google_value(values.client_id, ENV_GOOGLE_CLIENT_ID)?,
        client_secret: resolve_client_secret(values.client_secret, values.client_secret_file)?,
        redirect_uri: required_google_value(values.redirect_uri, ENV_GOOGLE_REDIRECT_URI)?,
        ..server::oauth::GoogleOAuthConfig::default()
    };
    set_optional_google_url(&mut config.auth_uri, values.auth_uri, ENV_GOOGLE_AUTH_URI)?;
    set_optional_google_url(
        &mut config.token_uri,
        values.token_uri,
        ENV_GOOGLE_TOKEN_URI,
    )?;
    set_optional_google_url(
        &mut config.revoke_uri,
        values.revoke_uri,
        ENV_GOOGLE_REVOKE_URI,
    )?;
    validate_google_url(&config.redirect_uri, ENV_GOOGLE_REDIRECT_URI)?;
    Ok(Some(config))
}

/// Resolves the OAuth client secret from its file form when present, otherwise
/// from the inline environment value. The file wins, matching how
/// `secrets::acquire_key` resolves the master key, so a deployment can migrate to
/// the file without a flag day and the inline value is simply ignored once the
/// file is set.
fn resolve_client_secret(
    inline: Option<String>,
    file: Option<String>,
) -> Result<String, GoogleOAuthConfigError> {
    let Some(path) = file else {
        return required_google_value(inline, ENV_GOOGLE_CLIENT_SECRET);
    };
    if path.trim().is_empty() {
        return Err(GoogleOAuthConfigError::Empty {
            variable: ENV_GOOGLE_CLIENT_SECRET_FILE,
        });
    }
    read_secret_file(std::path::Path::new(&path))
}

fn read_secret_file(path: &std::path::Path) -> Result<String, GoogleOAuthConfigError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path).map_err(|error| {
            GoogleOAuthConfigError::SecretFileUnreadable {
                detail: error.to_string(),
            }
        })?;
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(GoogleOAuthConfigError::SecretFilePermissive { mode });
        }
    }
    let value = std::fs::read_to_string(path).map_err(|error| {
        GoogleOAuthConfigError::SecretFileUnreadable {
            detail: error.to_string(),
        }
    })?;
    // A secret written with `printf` or an editor commonly ends in a newline;
    // trailing whitespace is never part of a Google client secret.
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(GoogleOAuthConfigError::Empty {
            variable: ENV_GOOGLE_CLIENT_SECRET_FILE,
        });
    }
    Ok(value)
}

fn required_google_value(
    value: Option<String>,
    variable: &'static str,
) -> Result<String, GoogleOAuthConfigError> {
    let value = value.ok_or(GoogleOAuthConfigError::Missing { variable })?;
    if value.trim().is_empty() {
        return Err(GoogleOAuthConfigError::Empty { variable });
    }
    Ok(value)
}

fn set_optional_google_url(
    target: &mut String,
    value: Option<String>,
    variable: &'static str,
) -> Result<(), GoogleOAuthConfigError> {
    if let Some(value) = value {
        if value.trim().is_empty() {
            return Err(GoogleOAuthConfigError::Empty { variable });
        }
        validate_google_url(&value, variable)?;
        *target = value;
    }
    Ok(())
}

/// Every one of these four variables is on the credential path: the redirect URI
/// receives an authorization code, and the token and revoke endpoints receive the
/// client secret and the refresh token as a form body. `http` is therefore refused
/// outright rather than merely discouraged. The egress guard permits `http` for
/// server-configured data sources, so it will not catch a typo here; all four
/// OAuth URLs need this stricter credential-path rule.
fn validate_google_url(value: &str, variable: &'static str) -> Result<(), GoogleOAuthConfigError> {
    let parsed =
        url::Url::parse(value).map_err(|_| GoogleOAuthConfigError::InvalidUrl { variable })?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(GoogleOAuthConfigError::InvalidUrl { variable });
    }
    Ok(())
}

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
    // The browser companion's built assets. Unset means the UI routes are not
    // mounted, which lets API-only deployments run without shipping a `dist/`
    // directory or answering every unknown path with the SPA's 404 page.
    let web_dir = std::env::var("DESKMATE_WEB_DIR").ok().map(PathBuf::from);
    if let Some(web_dir) = web_dir.as_ref() {
        assert!(
            web_dir.is_absolute(),
            "DESKMATE_WEB_DIR must be an absolute path (got {}); a relative path \
             resolves against the process's working directory, which is \
             unspecified under launchd/systemd",
            web_dir.display()
        );
        assert!(
            web_dir.join("index.html").is_file(),
            "DESKMATE_WEB_DIR ({}) has no index.html -- point it at the built \
             companion (apps/deskmate/dist), not at its parent",
            web_dir.display()
        );
    }
    let firmware_version = required_firmware_version(std::env::var("DESKMATE_FIRMWARE_VERSION"));
    let admin_token = std::env::var("DESKMATE_ADMIN_TOKEN")
        .expect("DESKMATE_ADMIN_TOKEN must be set -- see deploy/README.md");
    let google_oauth = google_oauth_config_from_env();
    let server_options = server_options_from_env();

    // `config_dir` is cloned because the integration store below opens against
    // it after the state has taken ownership.
    let state = ServerState::new_with_options(
        admin_token,
        FirmwareCatalog::new(firmware_dir, firmware_version).unwrap_or_else(|error| {
            panic!(
                "DESKMATE_FIRMWARE_VERSION is invalid ({error}); copy the exact value from \
                 firmware/version.txt and see deploy/README.md"
            )
        }),
        config_dir.clone(),
        server_options,
    );

    if let Some(oauth) = google_oauth {
        let store = server::secrets::open_integration_store(&config_dir).unwrap_or_else(|error| {
            panic!(
                "Google OAuth is configured but the integration secrets store could not open \
                 (fail-closed): {error}"
            )
        });
        let token_manager = Arc::new(server::oauth::TokenManager::new(
            Arc::new(store),
            Arc::new(server::oauth::transport::EgressTransport),
            oauth,
        ));
        let runtime = Arc::new(server::oauth::IntegrationRuntime::new(token_manager));
        state.set_integrations(runtime);
        tracing::info!("google oauth integration enabled");
    }

    // Server-rendered data cards, if this deployment has any. The specs are
    // read before the listener binds so a malformed file fails the start
    // rather than leaving a server up with silently missing cards.
    //
    // A refresh task is spawned per card and lives as long as the process; the
    // graceful shutdown below drains connections, and an in-flight fetch is
    // abandoned with it. That is safe because a card's durable state is the
    // frame already in the store: losing a refresh loses nothing but the tick.
    if let Some(faces) = faces_command() {
        server::set_faces(&state, faces);
    } else {
        tracing::info!("no faces package configured (DESKMATE_FACES_DIR unset)");
    }
    server::start_data_cards(&state)
        .unwrap_or_else(|error| panic!("an account's data-cards.json is unreadable: {error}"));

    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .unwrap_or_else(|error| panic!("failed to bind {bind_address}: {error}"));
    let local_addr = listener
        .local_addr()
        .expect("a bound listener has a local address");

    println!("deskmate server listening on {local_addr}");
    tracing::info!(address = %local_addr, "deskmate server listening");

    let shutdown_state = state.clone();
    if let Some(web_dir) = web_dir.as_ref() {
        tracing::info!(directory = %web_dir.display(), "serving the web companion");
    } else {
        tracing::info!("no web companion configured (DESKMATE_WEB_DIR unset)");
    }

    axum::serve(
        listener,
        app_with_web(state, web_dir).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_state.clone()))
    .await
    .expect("server exited with an error");
    tokio::task::spawn_blocking(move || shutdown_state.shutdown())
        .await
        .expect("device runtime shutdown worker panicked");
}

fn server_options_from_env() -> ServerOptions {
    let public_url = std::env::var("DESKMATE_PUBLIC_URL")
        .expect("DESKMATE_PUBLIC_URL must be set -- see deploy/README.md");
    let public_url = url::Url::parse(&public_url)
        .expect("DESKMATE_PUBLIC_URL must be an absolute URL -- see deploy/README.md");
    let signups_default = match std::env::var("DESKMATE_SIGNUPS").as_deref() {
        Ok("open") => true,
        Ok("closed") | Err(std::env::VarError::NotPresent) => false,
        Ok(_) => panic!("DESKMATE_SIGNUPS must be either 'open' or 'closed'"),
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("DESKMATE_SIGNUPS must be valid Unicode")
        }
    };
    ServerOptions {
        public_url,
        signups_default,
        ..ServerOptions::default()
    }
}

fn required_firmware_version(value: Result<String, std::env::VarError>) -> String {
    value.expect(
        "DESKMATE_FIRMWARE_VERSION must be set to the published image's exact \
         firmware/version.txt value -- see deploy/README.md",
    )
}

/// How to run the faces package, if this deployment has one.
///
/// `DESKMATE_FACES_DIR` is a checkout of `companion/faces/` with its
/// `node_modules` installed. Like `DESKMATE_WEB_DIR` it is read at use, not
/// embedded, so a face change is an rsync with no Rust build and no restart.
/// `DESKMATE_BUN` names the runtime; both must be absolute, because the child's
/// environment is cleared and a relative path would resolve against nothing the
/// operator chose.
fn faces_command() -> Option<server::FaceCommand> {
    let faces_dir = PathBuf::from(std::env::var("DESKMATE_FACES_DIR").ok()?);
    let bun =
        std::env::var("DESKMATE_BUN").map_or_else(|_| "/usr/local/bin/bun".into(), PathBuf::from);
    assert!(
        faces_dir.is_absolute() && bun.is_absolute(),
        "DESKMATE_FACES_DIR ({}) and DESKMATE_BUN ({}) must be absolute paths",
        faces_dir.display(),
        bun.display()
    );
    assert!(
        faces_dir.join("src/main.ts").is_file(),
        "DESKMATE_FACES_DIR ({}) has no src/main.ts -- point it at companion/faces",
        faces_dir.display()
    );
    assert!(
        bun.is_file(),
        "DESKMATE_BUN ({}) does not exist; the faces package needs the Bun runtime",
        bun.display()
    );
    Some(server::FaceCommand::bun(bun, &faces_dir))
}

/// `axum::serve` can drain in-flight connections before the process exits.
async fn shutdown_signal(state: ServerState) {
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

    state.begin_shutdown();
    tracing::info!("shutdown signal received, draining connections");
}

#[cfg(test)]
mod tests {
    fn complete_google_env() -> super::GoogleOAuthEnv {
        super::GoogleOAuthEnv {
            client_id: Some("client-id.apps.googleusercontent.com".to_string()),
            client_secret: Some("client-secret-value".to_string()),
            redirect_uri: Some(
                "https://deskmate.example/v1/integrations/google/callback".to_string(),
            ),
            ..super::GoogleOAuthEnv::default()
        }
    }

    #[test]
    fn google_config_is_none_when_every_google_variable_is_absent() {
        let config = super::google_oauth_config_from_values(super::GoogleOAuthEnv::default())
            .expect("absent config is valid");
        assert!(config.is_none());
    }

    #[test]
    fn google_config_requires_all_three_credentials_once_enabled() {
        let mut env = complete_google_env();
        env.client_secret = None;

        assert!(matches!(
            super::google_oauth_config_from_values(env),
            Err(super::GoogleOAuthConfigError::Missing {
                variable: super::ENV_GOOGLE_CLIENT_SECRET
            })
        ));
    }

    #[test]
    fn partial_google_config_without_client_id_is_an_error() {
        let env = super::GoogleOAuthEnv {
            client_secret: Some("client-secret-value".to_string()),
            ..super::GoogleOAuthEnv::default()
        };

        assert!(matches!(
            super::google_oauth_config_from_values(env),
            Err(super::GoogleOAuthConfigError::Missing {
                variable: super::ENV_GOOGLE_CLIENT_ID
            })
        ));
    }

    #[test]
    fn blank_required_google_value_is_rejected() {
        let mut env = complete_google_env();
        env.redirect_uri = Some("  ".to_string());

        assert!(matches!(
            super::google_oauth_config_from_values(env),
            Err(super::GoogleOAuthConfigError::Empty {
                variable: super::ENV_GOOGLE_REDIRECT_URI
            })
        ));
    }

    #[test]
    fn malformed_google_url_is_rejected_without_echoing_the_secret() {
        let mut env = complete_google_env();
        env.client_secret = Some("must-not-appear-in-errors".to_string());
        env.token_uri = Some("not a URL".to_string());

        let error = super::google_oauth_config_from_values(env).expect_err("invalid token URI");
        assert!(matches!(
            error,
            super::GoogleOAuthConfigError::InvalidUrl {
                variable: super::ENV_GOOGLE_TOKEN_URI
            }
        ));
        assert!(!error.to_string().contains("must-not-appear-in-errors"));
    }

    /// `http` on any of the four credential-path URLs would post the client secret
    /// and refresh token as a plaintext form body over port 80. The egress guard
    /// allows `http` for server-configured data sources, so this is the only
    /// check between a typo and cleartext credentials.
    #[test]
    fn a_plaintext_http_url_is_refused_on_every_credential_path_variable() {
        for (variable, apply) in [
            (
                super::ENV_GOOGLE_REDIRECT_URI,
                (|env: &mut super::GoogleOAuthEnv, value: String| env.redirect_uri = Some(value))
                    as fn(&mut super::GoogleOAuthEnv, String),
            ),
            (super::ENV_GOOGLE_AUTH_URI, |env, value| {
                env.auth_uri = Some(value);
            }),
            (super::ENV_GOOGLE_TOKEN_URI, |env, value| {
                env.token_uri = Some(value);
            }),
            (super::ENV_GOOGLE_REVOKE_URI, |env, value| {
                env.revoke_uri = Some(value);
            }),
        ] {
            let mut env = complete_google_env();
            apply(&mut env, "http://oauth2.googleapis.com/token".to_string());
            let error = super::google_oauth_config_from_values(env)
                .expect_err("an http URL on a credential path must be refused");
            assert!(
                matches!(
                    error,
                    super::GoogleOAuthConfigError::InvalidUrl { variable: got } if got == variable
                ),
                "{variable} accepted an http:// URL: {error}"
            );
        }
    }

    /// The file form exists so the client secret is not readable through
    /// `/proc/<pid>/environ`. It must therefore refuse a file anyone else can
    /// read, or it buys nothing.
    #[test]
    fn the_client_secret_file_is_preferred_and_must_not_be_group_or_world_readable() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("google-client-secret");
        let mut file = std::fs::File::create(&path).expect("create");
        // A trailing newline is what `printf` or an editor leaves behind.
        file.write_all(b"secret-from-the-file\n").expect("write");
        drop(file);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");

        // The file wins over the inline value, and the newline is trimmed.
        let mut env = complete_google_env();
        env.client_secret = Some("inline-value-that-must-lose".to_string());
        env.client_secret_file = Some(path.to_string_lossy().into_owned());
        let config = super::google_oauth_config_from_values(env)
            .expect("valid config")
            .expect("Google enabled");
        assert_eq!(config.client_secret, "secret-from-the-file");

        // Group-readable is refused: that is the whole point of the file form.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("chmod");
        let mut env = complete_google_env();
        env.client_secret_file = Some(path.to_string_lossy().into_owned());
        let error = super::google_oauth_config_from_values(env)
            .expect_err("a group-readable secret file must be refused");
        assert!(
            matches!(
                error,
                super::GoogleOAuthConfigError::SecretFilePermissive { mode: 0o640 }
            ),
            "unexpected error: {error}"
        );
        assert!(!error.to_string().contains("secret-from-the-file"));
    }

    /// A missing file is a hard startup failure, never a silent fall back to the
    /// inline value -- otherwise a typo in the path quietly reinstates the
    /// environment-variable exposure the file form exists to remove.
    #[test]
    fn a_missing_client_secret_file_does_not_fall_back_to_the_inline_value() {
        let mut env = complete_google_env();
        env.client_secret = Some("inline-value-that-must-not-be-used".to_string());
        env.client_secret_file = Some("/nonexistent/deskmate/google-client-secret".to_string());
        let error = super::google_oauth_config_from_values(env)
            .expect_err("a named-but-absent secret file must stop startup");
        assert!(
            matches!(
                error,
                super::GoogleOAuthConfigError::SecretFileUnreadable { .. }
            ),
            "unexpected error: {error}"
        );
        assert!(
            !error
                .to_string()
                .contains("inline-value-that-must-not-be-used")
        );
    }

    #[test]
    fn google_config_applies_optional_endpoint_overrides() {
        let mut env = complete_google_env();
        env.auth_uri = Some("https://identity.example/authorize".to_string());
        env.token_uri = Some("https://identity.example/token".to_string());
        env.revoke_uri = Some("https://identity.example/revoke".to_string());

        let config = super::google_oauth_config_from_values(env)
            .expect("valid config")
            .expect("Google enabled");
        assert_eq!(config.auth_uri, "https://identity.example/authorize");
        assert_eq!(config.token_uri, "https://identity.example/token");
        assert_eq!(config.revoke_uri, "https://identity.example/revoke");
    }

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
