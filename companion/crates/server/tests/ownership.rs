//! Ownership, exercised with a fake device socket rather than a board.

use futures_util::SinkExt;
use protocol::{DeviceEvent, EventAction, EventKind, Message};
use server::{ServerState, app};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message as WsMessage;

mod support;

/// Boots a server on a loopback port and mints one device identity.
/// Mirrors `tests/device_link.rs`'s helper; kept separate so the two files
/// can diverge without one silently changing the other's fixture.
async fn spawn() -> (String, server::registry::DeviceIdentity, String) {
    spawn_state(ServerState::in_memory(), support::IN_MEMORY_ADMIN_TOKEN).await
}

async fn spawn_state(
    state: ServerState,
    admin_token: &str,
) -> (String, server::registry::DeviceIdentity, String) {
    let identity = state.registry().mint().expect("mint identity");
    let admin_token = admin_token.to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    (
        format!("127.0.0.1:{}", address.port()),
        identity,
        admin_token,
    )
}

// The Err type is tungstenite's, so its size is not ours to reduce, and boxing
// it in a test helper would add indirection at every call site for nothing.
#[allow(clippy::result_large_err)]
async fn connect_device(
    host: &str,
    token: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Error,
> {
    let request = http::Request::builder()
        .uri(format!("ws://{host}/v1/device/link"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Host", host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap();
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(s, _)| s)
}

#[test]
fn minted_identity_authenticates_after_registry_reload() {
    // Catches a registry that still keeps identities only in process memory:
    // the second ServerState has no shared state except this directory.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    let identity = {
        let state = ServerState::new(
            "reload-admin-token".to_string(),
            server::firmware::FirmwareCatalog::in_memory(),
            config_root.clone(),
        );
        let identity = state.registry().mint().expect("mint identity");
        assert_eq!(
            state.registry().authenticate(&identity.token),
            Some(identity.device_id.clone())
        );
        identity
    };

    let reloaded = ServerState::new(
        "reload-admin-token".to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root,
    );
    assert_eq!(
        reloaded.registry().authenticate(&identity.token),
        Some(identity.device_id)
    );
}

#[test]
fn two_mints_both_authenticate_after_registry_reload() {
    // Catches serializing only the newest record: deleting the composition of
    // prior records with the new record would silently deprovision the first.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    let (first, second) = {
        let state = ServerState::new(
            "multi-reload-admin-token".to_string(),
            server::firmware::FirmwareCatalog::in_memory(),
            config_root.clone(),
        );
        (
            state.registry().mint().expect("first mint"),
            state.registry().mint().expect("second mint"),
        )
    };

    let reloaded = ServerState::new(
        "multi-reload-admin-token".to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root,
    );
    assert_eq!(
        reloaded.registry().authenticate(&first.token),
        Some(first.device_id)
    );
    assert_eq!(
        reloaded.registry().authenticate(&second.token),
        Some(second.device_id)
    );
}

#[test]
fn absent_registry_is_not_a_load_failure_and_creates_no_archive() {
    // Catches classifying a fresh install as a failed load, which would emit
    // the wrong auth warning and could activate the corrupt-store archive path.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    let state = ServerState::new(
        "absent-admin-token".to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root.clone(),
    );
    assert!(!state.registry().store_load_failed());
    state.registry().mint().expect("mint from absent store");
    assert!(archived_registry_files(&config_root).is_empty());
}

#[test]
fn persisted_registry_contains_digest_and_never_plaintext_token() {
    // Catches the security-critical regression where persistence writes the
    // bearer token itself, and also catches hashing a different value.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    let state = ServerState::new(
        "digest-admin-token".to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root.clone(),
    );
    let identity = state.registry().mint().expect("mint identity");
    let persisted = std::fs::read_to_string(config_root.join("device-identities.json"))
        .expect("persisted identity registry");
    let expected_digest = Sha256::digest(identity.token.as_bytes()).iter().fold(
        String::with_capacity(64),
        |mut hex, byte| {
            use std::fmt::Write;
            write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
            hex
        },
    );

    assert!(persisted.contains(&expected_digest));
    assert!(
        persisted.ends_with("}\n"),
        "persisted registry must end with exactly one newline"
    );
    assert!(
        !persisted.contains(&identity.token),
        "persisted registry exposed the literal bearer token"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(config_root.join("device-identities.json"))
            .expect("registry metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "identity registry was not created private");
    }
}

#[test]
fn corrupt_or_truncated_registry_degrades_to_empty() {
    // Catches propagating parse failures into a startup panic/error, accepting
    // partial state, or losing the diagnostic that explains ensuing 401s.
    for bytes in [
        b"this is not JSON".as_slice(),
        br#"{"schema_version":1,"next_sequence":7,"devices":["#.as_slice(),
    ] {
        let temp = tempfile::tempdir().expect("registry test temp dir");
        let config_root = temp.path().join("configs");
        std::fs::create_dir_all(&config_root).expect("create config root");
        std::fs::write(config_root.join("device-identities.json"), bytes)
            .expect("write broken registry");

        let state = ServerState::new(
            "corrupt-admin-token".to_string(),
            server::firmware::FirmwareCatalog::in_memory(),
            config_root,
        );
        assert_eq!(state.registry().authenticate("any-token"), None);
        assert!(!state.registry().contains_device("dev-0007"));
        assert!(state.registry().store_load_failed());
    }
}

#[test]
fn failed_store_is_archived_before_a_replacement_is_minted() {
    // Catches overwriting the only copy of a transiently unreadable or
    // operator-repairable store when the first replacement identity is minted.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    std::fs::create_dir_all(&config_root).expect("create config root");
    let store_path = config_root.join("device-identities.json");
    let original = b"operator may still recover these exact bytes";
    std::fs::write(&store_path, original).expect("write broken registry");

    let state = ServerState::new(
        "archive-admin-token".to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root.clone(),
    );
    let identity = state.registry().mint().expect("replacement mint");
    assert_eq!(identity.device_id, "dev-0001");
    assert_eq!(
        state.registry().authenticate(&identity.token),
        Some(identity.device_id)
    );
    assert_eq!(
        state
            .registry()
            .mint()
            .expect("second replacement mint")
            .device_id,
        "dev-0002"
    );

    let archives = archived_registry_files(&config_root);
    assert_eq!(
        archives.len(),
        1,
        "failed store was not archived exactly once"
    );
    assert_eq!(
        std::fs::read(&archives[0]).expect("read archived registry"),
        original
    );
    assert_ne!(
        std::fs::read(&store_path).expect("read replacement registry"),
        original
    );
}

#[test]
fn failed_store_sequence_recovers_from_existing_config_high_water() {
    // Catches resetting to dev-0001 after a discarded store and making the
    // replacement device inherit an earlier device's id-keyed config.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::write(config_root.join("device-identities.json"), b"broken")
        .expect("write broken registry");
    std::fs::write(config_root.join("dev-0001.json"), b"{}").expect("write first device config");
    std::fs::write(config_root.join("dev-0042.json"), b"{}")
        .expect("write high-water device config");
    std::fs::write(config_root.join("dev-9999.txt"), b"not a config")
        .expect("write irrelevant file");

    let state = ServerState::new(
        "high-water-admin-token".to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root,
    );
    assert_eq!(
        state.registry().mint().expect("replacement mint").device_id,
        "dev-0043"
    );
}

#[test]
fn unreadable_registry_degrades_to_empty() {
    // Catches treating an I/O failure as a fatal startup error. A directory at
    // the file path is deterministically unreadable as a registry on every OS.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    std::fs::create_dir_all(config_root.join("device-identities.json"))
        .expect("create unreadable registry stand-in");

    let state = ServerState::new(
        "unreadable-admin-token".to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root,
    );
    assert_eq!(state.registry().authenticate("any-token"), None);
    assert!(state.registry().store_load_failed());
}

#[test]
fn mint_does_not_release_a_token_when_persistence_fails() {
    // Catches returning a credential that works only in memory: the config
    // root is a regular file, so its registry child cannot be atomically saved.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("not-a-directory");
    std::fs::write(&config_root, b"blocks registry directory creation")
        .expect("create unwritable registry parent");
    let state = ServerState::new(
        "failed-mint-admin-token".to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root,
    );

    assert!(state.registry().mint().is_err());
    assert!(!state.registry().contains_device("dev-0001"));
}

#[tokio::test]
async fn failed_mint_http_response_keeps_the_store_error_contract() {
    const ADMIN_TOKEN: &str = "failed-mint-http-admin-token";

    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("not-a-directory");
    let state = ServerState::new(
        ADMIN_TOKEN.to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root.clone(),
    );
    std::fs::write(&config_root, b"blocks registry directory creation")
        .expect("create unwritable registry parent");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    tokio::spawn(async move {
        axum::serve(listener, app(state))
            .await
            .expect("serve admin routes");
    });

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/devices"))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("failed mint response");
    assert_eq!(response.status(), 500);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("store error body"))
            .expect("store error JSON");
    assert_eq!(body["kind"], "store");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("create device identity store directory:")),
        "unexpected failed-mint response: {body}"
    );
}

#[test]
fn mint_sequence_remains_monotonic_after_reload() {
    // Catches reconstructing an empty/default sequence on every process start,
    // which would reissue a device id and alias its per-device config file.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    {
        let state = ServerState::new(
            "sequence-admin-token".to_string(),
            server::firmware::FirmwareCatalog::in_memory(),
            config_root.clone(),
        );
        assert_eq!(
            state.registry().mint().expect("first mint").device_id,
            "dev-0001"
        );
        assert_eq!(
            state.registry().mint().expect("second mint").device_id,
            "dev-0002"
        );
    }

    let reloaded = ServerState::new(
        "sequence-admin-token".to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root,
    );
    assert_eq!(
        reloaded
            .registry()
            .mint()
            .expect("mint after reload")
            .device_id,
        "dev-0003"
    );
}

#[test]
fn unsupported_registry_schema_degrades_to_empty() {
    // Catches deleting the schema-version guard and silently accepting a
    // future format whose meaning this server does not know.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    write_registry_json(
        &config_root,
        &serde_json::json!({"schema_version": 2, "next_sequence": 0, "devices": []}),
    );
    let state = persistent_test_state(config_root);
    assert!(state.registry().store_load_failed());
    assert!(!state.registry().contains_device("dev-0001"));
}

#[test]
fn duplicate_registry_records_degrade_to_empty() {
    // Catches deleting duplicate-id/digest validation and loading ambiguous
    // authentication records from otherwise valid JSON.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    let digest = "11".repeat(32);
    write_registry_json(
        &config_root,
        &serde_json::json!({
            "schema_version": 1,
            "next_sequence": 2,
            "devices": [
                {"device_id": "dev-0001", "token_sha256": digest},
                {"device_id": "dev-0002", "token_sha256": digest}
            ]
        }),
    );
    let state = persistent_test_state(config_root);
    assert!(state.registry().store_load_failed());
    assert!(!state.registry().contains_device("dev-0001"));
}

#[test]
fn registry_sequence_below_an_existing_id_degrades_to_empty() {
    // Catches deleting the sequence consistency guard and loading state that
    // would reissue an already-present device id on the next mint.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    write_registry_json(
        &config_root,
        &serde_json::json!({
            "schema_version": 1,
            "next_sequence": 6,
            "devices": [
                {"device_id": "dev-0007", "token_sha256": "22".repeat(32)}
            ]
        }),
    );
    let state = persistent_test_state(config_root);
    assert!(state.registry().store_load_failed());
    assert!(!state.registry().contains_device("dev-0007"));
}

#[test]
fn oversized_registry_degrades_to_empty() {
    // Catches deleting the 64 KiB read bound and accepting an unbounded
    // operator-controlled file merely because its trailing bytes are whitespace.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    std::fs::create_dir_all(&config_root).expect("create config root");
    let mut bytes = serde_json::to_vec(
        &serde_json::json!({"schema_version": 1, "next_sequence": 0, "devices": []}),
    )
    .expect("encode valid registry");
    bytes.resize(64 * 1_024 + 1, b' ');
    std::fs::write(config_root.join("device-identities.json"), bytes)
        .expect("write oversized registry");

    let state = persistent_test_state(config_root);
    assert!(state.registry().store_load_failed());
}

#[test]
fn invalid_digest_encodings_degrade_to_empty() {
    // Catches deleting the exact-length or lowercase-hex digest guards, which
    // would accept truncated or noncanonical authentication material.
    for digest in ["33".repeat(31), "AA".repeat(32)] {
        let temp = tempfile::tempdir().expect("registry test temp dir");
        let config_root = temp.path().join("configs");
        write_registry_json(
            &config_root,
            &serde_json::json!({
                "schema_version": 1,
                "next_sequence": 1,
                "devices": [{"device_id": "dev-0001", "token_sha256": digest}]
            }),
        );
        let state = persistent_test_state(config_root);
        assert!(state.registry().store_load_failed());
        assert!(!state.registry().contains_device("dev-0001"));
    }
}

fn persistent_test_state(config_root: std::path::PathBuf) -> ServerState {
    ServerState::new(
        "persisted-fixture-admin-token".to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root,
    )
}

fn write_registry_json(config_root: &std::path::Path, value: &serde_json::Value) {
    std::fs::create_dir_all(config_root).expect("create config root");
    std::fs::write(
        config_root.join("device-identities.json"),
        serde_json::to_vec(value).expect("encode registry fixture"),
    )
    .expect("write registry fixture");
}

fn archived_registry_files(config_root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut archives: Vec<_> = std::fs::read_dir(config_root)
        .expect("read config root")
        .map(|entry| entry.expect("read config entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| name.starts_with("device-identities.json.corrupt-"))
        })
        .collect();
    archives.sort();
    archives
}

#[tokio::test]
async fn config_written_by_admin_reaches_the_device() {
    let (host, identity, admin_token) = spawn().await;
    let mut socket = connect_device(&host, &identity.token)
        .await
        .expect("connect");

    let config = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/one-clock-card.json"
    ))
    .expect("fixture");

    let admin_request = reqwest::Client::new()
        .put(format!(
            "http://{host}/v1/devices/{}/config",
            identity.device_id
        ))
        .bearer_auth(&admin_token)
        .header("Content-Type", "application/json")
        .body(config)
        .send();
    let (response, applied) = tokio::join!(
        admin_request,
        support::drive_until_config(&mut socket, "clock-1")
    );
    let response = response.expect("admin request");
    assert_eq!(response.status(), 200);

    assert_eq!(applied.cards.len(), 1);
    assert_eq!(applied.cards[0].card_id, "clock-1");

    // The config transaction ends at activation. Scene negotiation is the
    // event-driven follow-up: it must still happen, but its ACK must not hold
    // the admin response hostage on a slow or briefly stalled device link.
    let scene = support::drive_until_scene(&mut socket).await;
    assert_eq!(scene.card_id, "clock-1");
}

#[tokio::test]
async fn digital_clock_scene_written_by_admin_reaches_the_device() {
    let (host, identity, admin_token) = spawn().await;
    let mut socket = connect_device(&host, &identity.token)
        .await
        .expect("connect");
    support::bootstrap_runtime(&mut socket).await;
    let local_now = "2026-08-25T14:37:42";

    let admin_request = reqwest::Client::new()
        .post(format!(
            "http://{host}/v1/devices/{}/scene",
            identity.device_id
        ))
        .bearer_auth(&admin_token)
        .header("Content-Type", "application/json")
        .body(
            serde_json::json!({
                "card_id": "clock-1",
                "revision": 17,
                "template": "digital_clock",
                "show_seconds": true,
                "local_now": local_now,
            })
            .to_string(),
        )
        .send();
    let (response, pushed) = tokio::join!(admin_request, support::drive_until_scene(&mut socket));
    assert_eq!(response.expect("admin request").status(), 200);
    assert_eq!(pushed.card_id, "clock-1");
    assert_eq!(pushed.revision, 17);
    assert_eq!(
        pushed.scene,
        app_core::build_digital_clock_scene(&app_core::ClockCard {
            revision: 17,
            show_seconds: true,
        })
    );
}

#[tokio::test]
async fn scene_route_rejects_an_unknown_template_with_a_typed_bad_request() {
    let (host, identity, admin_token) = spawn().await;
    let response = reqwest::Client::new()
        .post(format!(
            "http://{host}/v1/devices/{}/scene",
            identity.device_id
        ))
        .bearer_auth(admin_token)
        .header("Content-Type", "application/json")
        .body(
            serde_json::json!({
                "card_id": "clock-1",
                "revision": 17,
                "template": "analogue_clock",
                "show_seconds": true,
                "local_now": "2026-08-25T14:37:42",
            })
            .to_string(),
        )
        .send()
        .await
        .expect("scene request");
    assert_eq!(response.status(), 400);
    let error: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("typed error body"))
            .expect("typed error JSON");
    assert_eq!(error["kind"], "invalid-scene");
    assert!(error["message"].as_str().unwrap().contains("template"));
}

#[tokio::test]
async fn scene_route_rejects_a_zero_revision_with_a_typed_bad_request() {
    let (host, identity, admin_token) = spawn().await;
    let response = reqwest::Client::new()
        .post(format!(
            "http://{host}/v1/devices/{}/scene",
            identity.device_id
        ))
        .bearer_auth(admin_token)
        .header("Content-Type", "application/json")
        .body(
            serde_json::json!({
                "card_id": "clock-1",
                "revision": 0,
                "template": "digital_clock",
                "show_seconds": true,
                "local_now": "2026-08-25T14:37:42",
            })
            .to_string(),
        )
        .send()
        .await
        .expect("scene request");
    assert_eq!(response.status(), 400);
    let error: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("typed error body"))
            .expect("typed error JSON");
    assert_eq!(error["kind"], "invalid-scene");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("scene revision")
    );
}

#[tokio::test]
async fn scene_route_rejects_an_overlong_card_id_with_a_typed_bad_request() {
    let (host, identity, admin_token) = spawn().await;
    let response = reqwest::Client::new()
        .post(format!(
            "http://{host}/v1/devices/{}/scene",
            identity.device_id
        ))
        .bearer_auth(admin_token)
        .header("Content-Type", "application/json")
        .body(
            serde_json::json!({
                "card_id": "clock-id-that-is-more-than-32-bytes",
                "revision": 17,
                "template": "digital_clock",
                "show_seconds": true,
                "local_now": "2026-08-25T14:37:42",
            })
            .to_string(),
        )
        .send()
        .await
        .expect("scene request");
    assert_eq!(response.status(), 400);
    let error: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("typed error body"))
            .expect("typed error JSON");
    assert_eq!(error["kind"], "invalid-scene");
    assert!(error["message"].as_str().unwrap().contains("card id"));
}

#[tokio::test]
async fn scene_route_rejects_a_malformed_local_instant_with_a_typed_bad_request() {
    let (host, identity, admin_token) = spawn().await;
    let response = reqwest::Client::new()
        .post(format!(
            "http://{host}/v1/devices/{}/scene",
            identity.device_id
        ))
        .bearer_auth(admin_token)
        .header("Content-Type", "application/json")
        .body(
            serde_json::json!({
                "card_id": "clock-1",
                "revision": 17,
                "template": "digital_clock",
                "show_seconds": true,
                "local_now": "2026-08-25 14:37:42",
            })
            .to_string(),
        )
        .send()
        .await
        .expect("scene request");
    assert_eq!(response.status(), 400);
    let error: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("typed error body"))
            .expect("typed error JSON");
    assert_eq!(error["kind"], "invalid-scene");
    assert!(error["message"].as_str().unwrap().contains("local_now"));
}

#[tokio::test]
async fn a_second_socket_for_the_same_device_is_refused() {
    // The single-owner invariant at the server's own boundary. A second link
    // must be refused rather than silently taking over -- a takeover is how
    // two owners appear without anyone deciding to allow them.
    let (host, identity, _admin_token) = spawn().await;
    let _first = connect_device(&host, &identity.token)
        .await
        .expect("first connects");
    let error = connect_device(&host, &identity.token)
        .await
        .expect_err("a second link for a device that already has one was accepted");
    let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
        panic!("second link failed without the intended HTTP refusal: {error}");
    };
    assert_eq!(response.status(), http::StatusCode::CONFLICT);
}

#[tokio::test]
async fn running_pomodoro_survives_link_close_and_reattach() {
    let (host, identity, admin_token) = spawn().await;
    let client = reqwest::Client::new();
    install_pomodoro_config(&client, &host, &identity.device_id, &admin_token).await;

    let mut first = connect_device(&host, &identity.token)
        .await
        .expect("first link connects");
    support::drive_until_config(&mut first, "pomodoro").await;
    start_pomodoro(&mut first).await;

    let before = wait_for_pomodoro(&client, &host, &identity.device_id, &admin_token, |timer| {
        timer["state"] == "running"
    })
    .await;
    let before_remaining = before["remaining_seconds"]
        .as_u64()
        .expect("remaining seconds before disconnect");
    drop(first);

    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
    let detached = wait_for_pomodoro(&client, &host, &identity.device_id, &admin_token, |timer| {
        timer["state"] == "running"
            && timer["remaining_seconds"]
                .as_u64()
                .is_some_and(|remaining| {
                    remaining < before_remaining && before_remaining.saturating_sub(remaining) <= 5
                })
    })
    .await;
    let detached_remaining = detached["remaining_seconds"]
        .as_u64()
        .expect("remaining seconds while detached");

    let after_config_remaining = reapply_config_while_running(
        &client,
        &host,
        &identity.device_id,
        &admin_token,
        detached_remaining,
    )
    .await;

    let mut second = connect_after_release(&host, &identity.token).await;
    let replayed = support::drive_until_config(&mut second, "pomodoro").await;
    assert!(
        replayed.cards.iter().any(|card| card.card_id == "pomodoro"),
        "reattach did not replay the layout"
    );
    let current_push = support::drive_until_push(&mut second, "pomodoro").await;
    assert!(current_push.running);
    let replayed_remaining = u64::from(current_push.remaining_ms) / 1_000;

    let after = wait_for_pomodoro(&client, &host, &identity.device_id, &admin_token, |timer| {
        timer["state"] == "running"
    })
    .await;
    let after_remaining = after["remaining_seconds"]
        .as_u64()
        .expect("remaining seconds after reattach");
    assert!(
        replayed_remaining <= after_config_remaining,
        "reattach replayed stale timer fields: before={after_config_remaining}, pushed={replayed_remaining}"
    );
    assert!(
        after_remaining <= after_config_remaining,
        "reattach moved the running pomodoro backwards: before={after_config_remaining}, after={after_remaining}"
    );
    assert!(
        before_remaining.saturating_sub(after_remaining) <= 8,
        "test reconnect took implausibly long: before={before_remaining}, after={after_remaining}"
    );
}

async fn install_pomodoro_config(
    client: &reqwest::Client,
    host: &str,
    device_id: &str,
    admin_token: &str,
) {
    let config = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/one-pomodoro-card.json"
    ))
    .expect("fixture");
    let saved = client
        .put(format!("http://{host}/v1/devices/{device_id}/config"))
        .bearer_auth(admin_token)
        .header("Content-Type", "application/json")
        .body(config)
        .send()
        .await
        .expect("config write");
    assert_eq!(saved.status(), 200);
}

async fn reapply_config_while_running(
    client: &reqwest::Client,
    host: &str,
    device_id: &str,
    admin_token: &str,
    maximum_remaining: u64,
) -> u64 {
    install_pomodoro_config(client, host, device_id, admin_token).await;
    let timer = wait_for_pomodoro(client, host, device_id, admin_token, |timer| {
        timer["state"] == "running"
            && timer["remaining_seconds"]
                .as_u64()
                .is_some_and(|remaining| remaining <= maximum_remaining)
    })
    .await;
    timer["remaining_seconds"]
        .as_u64()
        .expect("remaining seconds after config apply")
}

async fn start_pomodoro(socket: &mut support::DeviceSocket) {
    socket
        .send(WsMessage::Binary(
            protocol::encode_message(
                0,
                &Message::DeviceEvent(DeviceEvent {
                    sequence: 1,
                    kind: EventKind::Tap,
                    card_id: "pomodoro".into(),
                    action: EventAction::StartPause,
                    interrupt_token: None,
                }),
            )
            .expect("encode tap event"),
        ))
        .await
        .expect("send tap event");
    support::drive_until_push(socket, "pomodoro").await;
}

async fn connect_after_release(host: &str, token: &str) -> support::DeviceSocket {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Ok(socket) = connect_device(host, token).await {
            return socket;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "ownership slot remained reserved after the first link closed"
        );
        tokio::task::yield_now().await;
    }
}

async fn wait_for_pomodoro(
    client: &reqwest::Client,
    host: &str,
    device_id: &str,
    admin_token: &str,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let response = client
            .get(format!("http://{host}/v1/devices/{device_id}"))
            .bearer_auth(admin_token)
            .send()
            .await
            .expect("status request");
        assert_eq!(response.status(), 200);
        let status: serde_json::Value =
            serde_json::from_str(&response.text().await.expect("status response body"))
                .expect("status JSON");
        if let Some(timer) = status["snapshot"]["pomodoros"]
            .as_array()
            .and_then(|timers| timers.iter().find(|timer| timer["card_id"] == "pomodoro"))
            && predicate(timer)
        {
            return timer.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "pomodoro snapshot did not reach the expected state: {status}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn config_is_written_under_the_explicit_config_directory() {
    const ADMIN_TOKEN: &str = "explicit-config-admin-token";

    // Catches deriving config storage from the firmware path: production's
    // systemd sandbox only makes /var/lib/deskmate writable, so a firmware
    // override must not redirect config writes outside the configured root.
    let temp = tempfile::tempdir().expect("config test temp dir");
    let config_root = temp.path().join("explicit-configs");
    let firmware = server::firmware::FirmwareCatalog::in_memory();
    let former_derived_root = firmware.directory().parent().unwrap().join("configs");
    let state = ServerState::new(ADMIN_TOKEN.to_string(), firmware, config_root.clone());
    let (host, identity, admin_token) = spawn_state(state, ADMIN_TOKEN).await;
    let config = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/one-clock-card.json"
    ))
    .expect("fixture");

    let response = reqwest::Client::new()
        .put(format!(
            "http://{host}/v1/devices/{}/config",
            identity.device_id
        ))
        .bearer_auth(admin_token)
        .header("Content-Type", "application/json")
        .body(config)
        .send()
        .await
        .expect("config write");

    assert_eq!(response.status(), 200);
    assert!(config_root.join("dev-0001.json").is_file());
    assert!(!former_derived_root.join("dev-0001.json").exists());
}

#[tokio::test]
async fn admin_routes_refuse_a_bad_admin_token() {
    let (host, identity, _admin_token) = spawn().await;
    let client = reqwest::Client::new();

    let write = client
        .put(format!(
            "http://{host}/v1/devices/{}/config",
            identity.device_id
        ))
        .bearer_auth("not-the-admin-token")
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await
        .expect("request");
    assert_eq!(write.status(), 401);

    let scene = client
        .post(format!(
            "http://{host}/v1/devices/{}/scene",
            identity.device_id
        ))
        .bearer_auth("not-the-admin-token")
        .header("Content-Type", "application/json")
        .body(
            serde_json::json!({
                "card_id": "clock-1",
                "revision": 17,
                "template": "digital_clock",
                "show_seconds": true,
                "local_now": "2026-08-25T14:37:42",
            })
            .to_string(),
        )
        .send()
        .await
        .expect("request");
    assert_eq!(scene.status(), 401);

    let read = client
        .get(format!("http://{host}/v1/devices/{}", identity.device_id))
        .bearer_auth("not-the-admin-token")
        .send()
        .await
        .expect("request");
    assert_eq!(read.status(), 401);

    // A device's own token must not open the admin surface either.
    let crossover = client
        .get(format!("http://{host}/v1/devices/{}", identity.device_id))
        .bearer_auth(&identity.token)
        .send()
        .await
        .expect("request");
    assert_eq!(crossover.status(), 401);
}

#[tokio::test]
async fn admin_can_mint_a_device_identity_once() {
    // Catches a missing POST route or a response that omits the only copy of
    // the device bearer token; GET status is separately checked not to echo it.
    let (host, _identity, admin_token) = spawn().await;
    let response = reqwest::Client::new()
        .post(format!("http://{host}/v1/devices"))
        .bearer_auth(admin_token)
        .send()
        .await
        .expect("mint request");
    assert_eq!(response.status(), 200);
    let identity: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("identity body"))
            .expect("identity JSON");
    assert_eq!(identity["device_id"], "dev-0002");
    assert_eq!(
        identity["token"].as_str().map(str::len),
        Some(64),
        "mint response did not contain the 32-byte hex bearer token"
    );
}

#[tokio::test]
async fn invalid_config_is_typed_and_does_not_replace_last_good() {
    // A validation failure must not be labeled last-good or displace the
    // genuinely working config.
    let (host, identity, admin_token) = spawn().await;
    let client = reqwest::Client::new();
    let valid = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/one-clock-card.json"
    ))
    .expect("fixture");
    let saved = client
        .put(format!(
            "http://{host}/v1/devices/{}/config",
            identity.device_id
        ))
        .bearer_auth(&admin_token)
        .header("Content-Type", "application/json")
        .body(valid.clone())
        .send()
        .await
        .expect("valid write");
    assert_eq!(saved.status(), 200);

    let mut invalid: serde_json::Value = serde_json::from_str(&valid).unwrap();
    invalid["cards"][0]["id"] = "not-last-good".into();
    // A dwell below MIN_DWELL_SECONDS: since schema v10 dwell is a card field,
    // so this is the invalid document a playlist reference used to be.
    invalid["cards"][0]["dwell_seconds"] = 1.into();
    let rejected = client
        .put(format!(
            "http://{host}/v1/devices/{}/config",
            identity.device_id
        ))
        .bearer_auth(&admin_token)
        .header("Content-Type", "application/json")
        .body(serde_json::to_vec(&invalid).unwrap())
        .send()
        .await
        .expect("invalid write");
    assert_eq!(rejected.status(), 422);
    let error: serde_json::Value =
        serde_json::from_str(&rejected.text().await.expect("typed validation body"))
            .expect("typed validation JSON");
    assert_eq!(error["kind"], "invalid-config");
    assert!(error["issues"].as_array().is_some_and(|issues| {
        issues
            .iter()
            .any(|issue| issue["path"] == "cards[0].dwell_seconds")
    }));

    let mut socket = connect_device(&host, &identity.token)
        .await
        .expect("connect after invalid write");
    let applied = support::drive_until_config(&mut socket, "clock-1").await;
    assert_eq!(applied.cards[0].card_id, "clock-1");
    assert!(
        applied
            .cards
            .iter()
            .all(|card| card.card_id != "not-last-good"),
        "the invalid config displaced the genuine last-good config"
    );
}

#[tokio::test]
async fn config_without_a_wire_lowering_is_typed_and_does_not_replace_last_good() {
    let (host, identity, admin_token) = spawn().await;
    let client = reqwest::Client::new();
    let valid = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/one-clock-card.json"
    ))
    .expect("fixture");
    let saved = client
        .put(format!(
            "http://{host}/v1/devices/{}/config",
            identity.device_id
        ))
        .bearer_auth(&admin_token)
        .header("Content-Type", "application/json")
        .body(valid.clone())
        .send()
        .await
        .expect("valid write");
    assert_eq!(saved.status(), 200);

    let mut unsupported: serde_json::Value = serde_json::from_str(&valid).unwrap();
    unsupported["cards"][0]["id"] = "not-last-good".into();
    unsupported["cards"][0]["tap_action"] = serde_json::json!({
        "kind": "open-url",
        "url": "https://example.test/action",
    });
    let parsed: app_core::AppConfig =
        serde_json::from_value(unsupported.clone()).expect("deserializable configuration");
    parsed
        .validate()
        .expect("host action passes ordinary validation");

    let rejected = client
        .put(format!(
            "http://{host}/v1/devices/{}/config",
            identity.device_id
        ))
        .bearer_auth(&admin_token)
        .header("Content-Type", "application/json")
        .body(serde_json::to_vec(&unsupported).unwrap())
        .send()
        .await
        .expect("compile-only invalid write");
    assert_eq!(rejected.status(), 422);
    let error: serde_json::Value =
        serde_json::from_str(&rejected.text().await.expect("typed validation body"))
            .expect("typed validation JSON");
    assert_eq!(error["kind"], "invalid-config");
    assert!(error["issues"].as_array().is_some_and(|issues| {
        issues
            .iter()
            .any(|issue| issue["path"] == "cards[0]" && issue["code"] == "requires-capability")
    }));

    let mut socket = connect_device(&host, &identity.token)
        .await
        .expect("connect after compile-only invalid write");
    let applied = support::drive_until_config(&mut socket, "clock-1").await;
    assert_eq!(applied.cards[0].card_id, "clock-1");
    assert!(
        applied
            .cards
            .iter()
            .all(|card| card.card_id != "not-last-good"),
        "the compile-only invalid config displaced the genuine last-good config"
    );
}

#[tokio::test]
async fn config_request_body_is_bounded_before_json_parsing() {
    // Catches deletion of the route-level body limit; ConfigStore's on-disk
    // limit is too late to stop Axum buffering an attacker-sized request.
    let (host, identity, admin_token) = spawn().await;
    let response = reqwest::Client::new()
        .put(format!(
            "http://{host}/v1/devices/{}/config",
            identity.device_id
        ))
        .bearer_auth(admin_token)
        .header("Content-Type", "application/json")
        .body(vec![b' '; app_core::MAX_CONFIG_FILE_BYTES + 1])
        .send()
        .await
        .expect("oversized write");
    assert_eq!(response.status(), 413);
}

#[tokio::test]
async fn admin_status_reports_live_state_without_device_secrets() {
    // Catches a status implementation that either omits the app-core snapshot
    // or serializes the registry's bearer token back to an admin client.
    let (host, identity, admin_token) = spawn().await;
    let mut socket = connect_device(&host, &identity.token)
        .await
        .expect("connect");
    support::bootstrap_runtime(&mut socket).await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{host}/v1/devices/{}", identity.device_id))
        .bearer_auth(&admin_token)
        .send()
        .await
        .expect("status request");
    assert_eq!(response.status(), 200);
    let text = response.text().await.expect("status body");
    assert!(!text.contains(&identity.token));
    assert!(!text.contains(&admin_token));
    let status: serde_json::Value = serde_json::from_str(&text).expect("status JSON");
    assert_eq!(status["device_id"], identity.device_id);
    assert_eq!(status["connected"], true);
    assert!(status["last_seen_unix_ms"].as_u64().is_some());
    assert_eq!(
        status["snapshot"]["device"]["connection"]["kind"], "online",
        "status omitted the runtime's live app-core connection state"
    );
    assert_eq!(
        status["snapshot"]["device"]["last_ota_error"], "download: ESP_ERR_NO_MEM",
        "status omitted the device's actionable OTA failure reason"
    );
    // The runtime is retained across a link drop, so every field under `device`
    // is the last value received rather than a current one, and a frozen
    // uptime_ms reads exactly like a live one. This is what says how old the
    // sample is, and it has to sit next to the values it qualifies -- the
    // top-level `connected` was missed once already.
    assert!(
        status["snapshot"]["device"]["observed_age_seconds"]
            .as_u64()
            .is_some_and(|age| age < 10),
        "status omitted how old the device sample is, or reported a nonsense age: {status}"
    );
}

#[tokio::test]
async fn admin_status_reports_defaults_used_after_stored_config_validation_failure() {
    const ADMIN_TOKEN: &str = "fallback-admin-token";

    // Catches calling a fresh process's factory defaults "last-good" and
    // catches omitting fallback state from the only admin status endpoint.
    let temp = tempfile::tempdir().expect("config test temp dir");
    let config_root = temp.path().join("configs");
    std::fs::create_dir_all(&config_root).expect("create config root");
    let state = ServerState::new(
        ADMIN_TOKEN.to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root.clone(),
    );
    let identity = state.registry().mint().expect("mint identity");
    let admin_token = ADMIN_TOKEN.to_string();
    let mut invalid: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/one-clock-card.json"
        ))
        .expect("fixture"),
    )
    .expect("fixture JSON");
    invalid["cards"][0]["dwell_seconds"] = 1.into();
    std::fs::write(
        config_root.join(format!("{}.json", identity.device_id)),
        serde_json::to_vec(&invalid).expect("invalid config JSON"),
    )
    .expect("write invalid stored config");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    let host = format!("127.0.0.1:{}", address.port());
    let mut socket = connect_device(&host, &identity.token)
        .await
        .expect("connect with invalid stored config");
    support::bootstrap_runtime(&mut socket).await;

    let response = reqwest::Client::new()
        .get(format!("http://{host}/v1/devices/{}", identity.device_id))
        .bearer_auth(admin_token)
        .send()
        .await
        .expect("status request");
    assert_eq!(response.status(), 200);
    let status: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("status response body"))
            .expect("status JSON");
    assert_eq!(status["config"]["origin"], "defaults");
    assert_eq!(status["config"]["using_fallback"], true);
    assert_eq!(status["config"]["fallback_reason"], "validation-failed");
}

#[tokio::test]
async fn ownership_slot_is_released_after_disconnect() {
    // Catches a leaked ownership reservation that would strand a real device
    // after WiFi/NAT reconnect despite correctly refusing concurrent sockets.
    let (host, identity, _admin_token) = spawn().await;
    let first = connect_device(&host, &identity.token)
        .await
        .expect("first connects");
    drop(first);

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        if let Ok(second) = connect_device(&host, &identity.token).await {
            drop(second);
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "ownership slot remained reserved after disconnect"
        );
        tokio::task::yield_now().await;
    }
}
