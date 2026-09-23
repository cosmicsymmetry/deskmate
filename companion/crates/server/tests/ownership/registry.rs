use server::{ServerState, app};
use sha2::{Digest, Sha256};

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
fn invalid_registry_files_degrade_to_empty() {
    let mut oversized =
        registry_json(&serde_json::json!({"schema_version": 1, "next_sequence": 0, "devices": []}));
    oversized.resize(64 * 1_024 + 1, b' ');
    let duplicate_digest = "11".repeat(32);
    let cases = [
        // Catches propagating parse failures into a startup panic/error,
        // accepting partial state, or losing the diagnostic that explains
        // ensuing 401s.
        (
            "malformed-json",
            b"this is not JSON".to_vec(),
            "dev-0007",
            "corrupt-admin-token",
        ),
        (
            "truncated-json",
            br#"{"schema_version":1,"next_sequence":7,"devices":["#.to_vec(),
            "dev-0007",
            "corrupt-admin-token",
        ),
        // Catches deleting the schema-version guard and silently accepting a
        // future format whose meaning this server does not know.
        (
            "unsupported-schema",
            registry_json(
                &serde_json::json!({"schema_version": 2, "next_sequence": 0, "devices": []}),
            ),
            "dev-0001",
            "persisted-fixture-admin-token",
        ),
        // Catches deleting duplicate-id/digest validation and loading
        // ambiguous authentication records from otherwise valid JSON.
        (
            "duplicate-records",
            registry_json(&serde_json::json!({
                "schema_version": 1,
                "next_sequence": 2,
                "devices": [
                    {"device_id": "dev-0001", "token_sha256": duplicate_digest},
                    {"device_id": "dev-0002", "token_sha256": duplicate_digest}
                ]
            })),
            "dev-0001",
            "persisted-fixture-admin-token",
        ),
        // Catches deleting the sequence consistency guard and loading state
        // that would reissue an already-present device id on the next mint.
        (
            "sequence-below-existing-id",
            registry_json(&serde_json::json!({
                "schema_version": 1,
                "next_sequence": 6,
                "devices": [
                    {"device_id": "dev-0007", "token_sha256": "22".repeat(32)}
                ]
            })),
            "dev-0007",
            "persisted-fixture-admin-token",
        ),
        // Catches deleting the 64 KiB read bound and accepting an unbounded
        // operator-controlled file merely because its trailing bytes are
        // whitespace.
        (
            "oversized",
            oversized,
            "dev-0001",
            "persisted-fixture-admin-token",
        ),
        // Catches deleting the exact-length or lowercase-hex digest guards,
        // which would accept truncated or noncanonical authentication
        // material.
        (
            "short-digest",
            registry_json(&serde_json::json!({
                "schema_version": 1,
                "next_sequence": 1,
                "devices": [{"device_id": "dev-0001", "token_sha256": "33".repeat(31)}]
            })),
            "dev-0001",
            "persisted-fixture-admin-token",
        ),
        (
            "uppercase-digest",
            registry_json(&serde_json::json!({
                "schema_version": 1,
                "next_sequence": 1,
                "devices": [{"device_id": "dev-0001", "token_sha256": "AA".repeat(32)}]
            })),
            "dev-0001",
            "persisted-fixture-admin-token",
        ),
    ];
    for (name, bytes, absent_device, admin_token) in cases {
        let temp = tempfile::tempdir().expect("registry test temp dir");
        let config_root = temp.path().join("configs");
        std::fs::create_dir_all(&config_root).expect("create config root");
        std::fs::write(config_root.join("device-identities.json"), bytes)
            .expect("write broken registry");

        let state = ServerState::new(
            admin_token.to_owned(),
            server::firmware::FirmwareCatalog::in_memory(),
            config_root,
        );
        assert_eq!(state.registry().authenticate("any-token"), None, "{name}");
        assert!(!state.registry().contains_device(absent_device), "{name}");
        assert!(state.registry().store_load_failed(), "{name}");
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
    // registry path is a directory, so it cannot be atomically replaced.
    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    let state = ServerState::new(
        "failed-mint-admin-token".to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root.clone(),
    );
    std::fs::create_dir(config_root.join("device-identities.json"))
        .expect("block registry replacement");

    assert!(state.registry().mint().is_err());
    assert!(!state.registry().contains_device("dev-0001"));
}

#[tokio::test]
async fn failed_mint_http_response_keeps_the_store_error_contract() {
    const ADMIN_TOKEN: &str = "failed-mint-http-admin-token";

    let temp = tempfile::tempdir().expect("registry test temp dir");
    let config_root = temp.path().join("configs");
    let state = ServerState::new(
        ADMIN_TOKEN.to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root.clone(),
    );
    super::support::owner_account(&state);
    std::fs::create_dir(config_root.join("device-identities.json"))
        .expect("block registry replacement");
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
            .is_some_and(|message| message.starts_with("sync and replace device identity store:")),
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

fn registry_json(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("encode registry fixture")
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
