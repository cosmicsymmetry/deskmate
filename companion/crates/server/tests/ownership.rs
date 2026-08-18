//! Ownership, exercised with a fake device socket rather than a board.

use server::{ServerState, app};

mod support;

/// Boots a server on a loopback port and mints one device identity.
/// Mirrors `tests/device_link.rs`'s helper; kept separate so the two files
/// can diverge without one silently changing the other's fixture.
async fn spawn() -> (String, server::registry::DeviceIdentity, String) {
    let state = ServerState::in_memory();
    let identity = state.registry().mint();
    let admin_token = state.admin_token().to_string();
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

    assert_eq!(applied.widgets.len(), 1);
    assert_eq!(applied.widgets[0].widget_id, "clock-1");
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
    assert!(
        connect_device(&host, &identity.token).await.is_err(),
        "a second link for a device that already has one was accepted"
    );
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
    // Catches the V1 regression where a validation failure was mislabeled as
    // last-good and allowed to displace the genuinely working config.
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
    invalid["playlists"][0]["entries"][0]["card_id"] = "not-last-good".into();
    invalid["active_playlist_id"] = "missing-playlist".into();
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
            .any(|issue| issue["path"] == "active_playlist_id")
    }));

    let mut socket = connect_device(&host, &identity.token)
        .await
        .expect("connect after invalid write");
    let applied = support::drive_until_config(&mut socket, "clock-1").await;
    assert_eq!(applied.widgets[0].widget_id, "clock-1");
    assert!(
        applied
            .widgets
            .iter()
            .all(|widget| widget.widget_id != "not-last-good"),
        "the invalid config displaced the genuine last-good config"
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
