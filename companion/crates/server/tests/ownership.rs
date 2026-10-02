//! Ownership, exercised with a fake device socket rather than a board.

use futures_util::SinkExt;
use protocol::{DeviceEvent, EventAction, EventKind, Message};
use server::{ServerState, app};
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[path = "ownership/registry.rs"]
mod registry;
#[path = "ownership/scenes.rs"]
mod scenes;
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
    let owner = support::owner_account(&state);
    let identity = support::mint_owned_device(&state, &owner);
    let admin_token = admin_token.to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (
        format!("127.0.0.1:{}", address.port()),
        identity,
        admin_token,
    )
}

use support::connect_device;

fn clock_config() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/one-clock-card.json"
    ))
    .expect("fixture")
}

async fn write_config(
    client: &reqwest::Client,
    host: &str,
    device_id: &str,
    admin_token: &str,
    body: impl Into<reqwest::Body>,
) -> reqwest::Response {
    client
        .put(format!("http://{host}/v1/devices/{device_id}/config"))
        .bearer_auth(admin_token)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .expect("config write")
}

#[tokio::test]
async fn config_written_by_admin_reaches_the_device() {
    let (host, identity, admin_token) = spawn().await;
    let mut socket = connect_device(&host, &identity.token)
        .await
        .expect("connect");

    let config = clock_config();
    let client = reqwest::Client::new();
    let admin_request = write_config(&client, &host, &identity.device_id, &admin_token, config);
    let (response, applied) = tokio::join!(
        admin_request,
        support::drive_until_config(&mut socket, "clock-1")
    );
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
    // Detect the closed link before reapplying config while detached. Production's
    // two-second status poll races the 1.5-second timer observation below.
    let (host, identity, admin_token) = spawn_state(
        ServerState::in_memory_with_runtime_options(app_core::RuntimeOptions {
            status_interval: std::time::Duration::from_millis(20),
            reconnect_interval: std::time::Duration::from_millis(10),
            ..app_core::RuntimeOptions::default()
        }),
        support::IN_MEMORY_ADMIN_TOKEN,
    )
    .await;
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
    let saved = write_config(client, host, device_id, admin_token, config).await;
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
            .expect("encode tap event")
            .into(),
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
    let config = clock_config();
    let response = write_config(
        &reqwest::Client::new(),
        &host,
        &identity.device_id,
        &admin_token,
        config,
    )
    .await;

    assert_eq!(response.status(), 200);
    let account_root = std::fs::read_dir(config_root.join("accounts"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(account_root.join("devices/dev-0001.json").is_file());
    assert!(!former_derived_root.join("dev-0001.json").exists());
}

#[tokio::test]
async fn admin_routes_refuse_a_bad_admin_token() {
    let (host, identity, _admin_token) = spawn().await;
    let client = reqwest::Client::new();
    let scene = serde_json::json!({
        "card_id": "clock-1",
        "revision": 17,
        "template": "digital_clock",
        "show_seconds": true,
        "local_now": "2026-08-25T14:37:42",
    })
    .to_string();
    for (name, method, path, body) in [
        (
            "config-write",
            reqwest::Method::PUT,
            format!("/v1/devices/{}/config", identity.device_id),
            Some("{}".to_owned()),
        ),
        (
            "scene-write",
            reqwest::Method::POST,
            format!("/v1/devices/{}/scene", identity.device_id),
            Some(scene),
        ),
        (
            "status-read",
            reqwest::Method::GET,
            format!("/v1/devices/{}", identity.device_id),
            None,
        ),
    ] {
        let mut request = client
            .request(method, format!("http://{host}{path}"))
            .bearer_auth("not-the-admin-token");
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body);
        }
        let response = request.send().await.expect("request");
        assert_eq!(response.status(), 401, "{name}");
    }

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
    let valid = clock_config();
    let saved = write_config(
        &client,
        &host,
        &identity.device_id,
        &admin_token,
        valid.clone(),
    )
    .await;
    assert_eq!(saved.status(), 200);

    let mut invalid: serde_json::Value = serde_json::from_str(&valid).unwrap();
    invalid["cards"][0]["id"] = "not-last-good".into();
    // A dwell below MIN_DWELL_SECONDS: since schema v10 dwell is a card field,
    // so this is the invalid document a playlist reference used to be.
    invalid["cards"][0]["dwell_seconds"] = 1.into();
    let rejected = write_config(
        &client,
        &host,
        &identity.device_id,
        &admin_token,
        serde_json::to_vec(&invalid).unwrap(),
    )
    .await;
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
    let valid = clock_config();
    let saved = write_config(
        &client,
        &host,
        &identity.device_id,
        &admin_token,
        valid.clone(),
    )
    .await;
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

    let rejected = write_config(
        &client,
        &host,
        &identity.device_id,
        &admin_token,
        serde_json::to_vec(&unsupported).unwrap(),
    )
    .await;
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
    let response = write_config(
        &reqwest::Client::new(),
        &host,
        &identity.device_id,
        &admin_token,
        vec![b' '; app_core::MAX_CONFIG_FILE_BYTES + 1],
    )
    .await;
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
    let owner = support::owner_account(&state);
    let identity = support::mint_owned_device(&state, &owner);
    let account_root = state.account_space(&owner.id).root.clone();
    std::fs::create_dir_all(account_root.join("devices")).expect("create account devices root");
    let admin_token = ADMIN_TOKEN.to_string();
    let mut invalid: serde_json::Value =
        serde_json::from_str(&clock_config()).expect("fixture JSON");
    invalid["cards"][0]["dwell_seconds"] = 1.into();
    std::fs::write(
        account_root
            .join("devices")
            .join(format!("{}.json", identity.device_id)),
        serde_json::to_vec(&invalid).expect("invalid config JSON"),
    )
    .expect("write invalid stored config");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
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
async fn linked_config_recovery_remains_visible_until_explicit_save() {
    const ADMIN_TOKEN: &str = "linked-recovery-admin-token";

    let temp = tempfile::tempdir().expect("config test temp dir");
    let config_root = temp.path().join("configs");
    std::fs::create_dir_all(&config_root).expect("create config root");
    let state = ServerState::new(
        ADMIN_TOKEN.to_string(),
        server::firmware::FirmwareCatalog::in_memory(),
        config_root.clone(),
    );
    let owner = support::owner_account(&state);
    let refused = support::mint_owned_device(&state, &owner);
    let healthy = support::mint_owned_device(&state, &owner);
    let devices_root = state.account_space(&owner.id).root.join("devices");
    std::fs::create_dir_all(&devices_root).expect("create account devices root");
    let refused_path = devices_root.join(format!("{}.json", refused.device_id));
    let refused_bytes = br#"{"schema_version":12,"future_body":true}"#;
    std::fs::write(&refused_path, refused_bytes).expect("write unsupported config");
    let healthy_path = devices_root.join(format!("{}.json", healthy.device_id));
    let valid = clock_config();
    std::fs::write(&healthy_path, &valid).expect("write healthy config");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let host = format!("127.0.0.1:{}", address.port());
    let client = reqwest::Client::new();

    let mut refused_socket = connect_device(&host, &refused.token)
        .await
        .expect("refused-config device still links on safe fallback");
    let refused_apply = support::drive_until_config(&mut refused_socket, "clock").await;
    assert_eq!(refused_apply.cards[0].card_id, "clock");

    let mut healthy_socket = connect_device(&host, &healthy.token)
        .await
        .expect("healthy device links alongside refused-config device");
    let healthy_apply = support::drive_until_config(&mut healthy_socket, "clock-1").await;
    assert_eq!(healthy_apply.cards[0].card_id, "clock-1");

    let refused_snapshot = companion_snapshot(&client, &host, &refused.device_id, &owner).await;
    assert_recoverable_unsupported(&refused_snapshot);
    let healthy_snapshot = companion_snapshot(&client, &host, &healthy.device_id, &owner).await;
    assert_eq!(healthy_snapshot["persistence"]["kind"], "clean");

    let status = admin_status(&client, &host, &refused.device_id, ADMIN_TOKEN).await;
    assert_eq!(status["config"]["origin"], "defaults");
    assert_eq!(status["config"]["using_fallback"], true);
    assert_eq!(status["config"]["fallback_reason"], "recovery");
    assert_recoverable_unsupported(&status["snapshot"]);

    let event = first_app_state_event(&client, &host, &refused.device_id, &owner).await;
    assert_recoverable_unsupported(&event);

    drop(refused_socket);
    let mut refused_socket = connect_after_release(&host, &refused.token).await;
    support::bootstrap_runtime(&mut refused_socket).await;
    let after_reconnect = companion_snapshot(&client, &host, &refused.device_id, &owner).await;
    assert_recoverable_unsupported(&after_reconnect);
    assert_eq!(std::fs::read(&refused_path).unwrap(), refused_bytes);
    assert_eq!(std::fs::read_to_string(&healthy_path).unwrap(), valid);

    let save = write_config(
        &client,
        &host,
        &refused.device_id,
        ADMIN_TOKEN,
        valid.clone(),
    );
    let (saved, applied) = tokio::join!(
        save,
        support::drive_until_config(&mut refused_socket, "clock-1")
    );
    assert_eq!(saved.status(), 200);
    assert_eq!(applied.cards[0].card_id, "clock-1");

    let cleared = companion_snapshot(&client, &host, &refused.device_id, &owner).await;
    assert_eq!(cleared["persistence"]["kind"], "clean");
    let status = admin_status(&client, &host, &refused.device_id, ADMIN_TOKEN).await;
    assert_eq!(status["config"]["origin"], "current");
    assert_eq!(status["config"]["using_fallback"], false);
    assert!(status["config"]["fallback_reason"].is_null());
    assert_eq!(status["snapshot"]["persistence"]["kind"], "clean");
}

async fn companion_snapshot(
    client: &reqwest::Client,
    host: &str,
    device_id: &str,
    account: &support::TestAccount,
) -> serde_json::Value {
    let response = client
        .get(format!("http://{host}/v1/app/{device_id}/snapshot"))
        .header("cookie", &account.cookie)
        .header("origin", "https://deskmate.test")
        .send()
        .await
        .expect("companion snapshot");
    assert_eq!(response.status(), 200);
    serde_json::from_str(&response.text().await.expect("companion snapshot body"))
        .expect("companion snapshot JSON")
}

async fn admin_status(
    client: &reqwest::Client,
    host: &str,
    device_id: &str,
    admin_token: &str,
) -> serde_json::Value {
    let response = client
        .get(format!("http://{host}/v1/devices/{device_id}"))
        .bearer_auth(admin_token)
        .send()
        .await
        .expect("admin status");
    assert_eq!(response.status(), 200);
    serde_json::from_str(&response.text().await.expect("admin status body"))
        .expect("admin status JSON")
}

fn assert_recoverable_unsupported(snapshot: &serde_json::Value) {
    assert_eq!(snapshot["persistence"]["kind"], "recoverable-error");
    assert_eq!(
        snapshot["persistence"]["message"],
        "config schema version 12 is unsupported; expected 11"
    );
}

async fn first_app_state_event(
    client: &reqwest::Client,
    host: &str,
    device_id: &str,
    account: &support::TestAccount,
) -> serde_json::Value {
    let mut response = client
        .get(format!("http://{host}/v1/app/{device_id}/events"))
        .header("cookie", &account.cookie)
        .header("origin", "https://deskmate.test")
        .send()
        .await
        .expect("event stream");
    assert_eq!(response.status(), 200);
    let mut bytes = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !bytes.windows(2).any(|window| window == b"\n\n") {
            bytes.extend(
                response
                    .chunk()
                    .await
                    .expect("event stream chunk")
                    .expect("first app-state event"),
            );
        }
    })
    .await
    .expect("first app-state event deadline");
    let text = std::str::from_utf8(&bytes).expect("event stream UTF-8");
    assert!(text.lines().any(|line| line == "event: app-state"));
    let data = text
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .expect("app-state data");
    serde_json::from_str(data).expect("app-state JSON")
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
