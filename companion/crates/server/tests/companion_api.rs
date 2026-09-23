//! HTTP contract for the browser companion's routes.
//!
//! These exercise the real router over a real loopback socket, the way
//! `image_routes.rs` does, because the things most likely to break here are not
//! reachable from a unit test: which gate a route is behind, what a route
//! answers when the board is absent, and whether the SPA fallback can shadow the
//! API. The traversal test intentionally drives the router directly so its
//! request target is not normalized by an HTTP client first.

use axum::body::{Body, to_bytes};
use axum::http::Request;
use reqwest::{Client, Method, StatusCode};
use serde::Deserialize;
use server::firmware::FirmwareCatalog;
use server::{ServerState, app, app_with_web};
use tower::ServiceExt;

mod support;

use support::{HttpTestServer as TestServer, json_body, replace_file_with_directory};

const ADMIN_TOKEN: &str = "in-memory-admin-token";

async fn spawn_with(state: ServerState, web_root: Option<std::path::PathBuf>) -> TestServer {
    support::owner_account(&state);
    support::spawn_http(app_with_web(support::with_fake_faces(state), web_root)).await
}

fn mint_test_device(state: &ServerState) -> server::registry::DeviceIdentity {
    let owner = support::owner_account(state);
    support::mint_owned_device(state, &owner)
}

fn device_config_path(state: &ServerState, device_id: &str) -> std::path::PathBuf {
    let owner = support::owner_account(state);
    state
        .account_space(&owner.id)
        .root
        .join("devices")
        .join(format!("{device_id}.json"))
}

fn account_root(config_root: &std::path::Path) -> std::path::PathBuf {
    std::fs::read_dir(config_root.join("accounts"))
        .expect("accounts directory")
        .next()
        .expect("owner account")
        .expect("account entry")
        .path()
}

async fn spawn() -> (TestServer, ServerState) {
    let state = ServerState::in_memory();
    // Minting a device needs an owner to give it to.
    support::owner_account(&state);
    let server = spawn_with(state.clone(), None).await;
    (server, state)
}

#[derive(Debug, Deserialize)]
struct MintedDevice {
    device_id: String,
}

async fn mint_device(client: &Client, server: &TestServer) -> MintedDevice {
    let response = client
        .post(format!("{}/v1/devices", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("mint device");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("mint response body");
    serde_json::from_str(&body).expect("mint response JSON")
}

/// Decodes a JSON body without pulling reqwest's `json` feature into this
/// workspace: the production client's feature set is deliberately minimal
/// because it is the one the SSRF egress guard hands to the provider layer.
async fn snapshot(client: &Client, server: &TestServer, device_id: &str) -> serde_json::Value {
    let response = client
        .get(format!("{}/v1/app/{device_id}/snapshot", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("snapshot");
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}

async fn direct_snapshot(state: &ServerState, device_id: &str) -> serde_json::Value {
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/v1/app/{device_id}/snapshot"))
                .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
                .body(Body::empty())
                .expect("snapshot request"),
        )
        .await
        .expect("snapshot response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("snapshot body");
    serde_json::from_slice(&body).expect("snapshot JSON")
}

async fn direct_device_list(state: &ServerState) -> serde_json::Value {
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/v1/app/devices")
                .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
                .body(Body::empty())
                .expect("device-list request"),
        )
        .await
        .expect("device-list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("device-list body");
    serde_json::from_slice(&body).expect("device-list JSON")
}

#[tokio::test]
async fn event_stream_ends_when_shutdown_begins_without_a_socket() {
    use futures_util::{FutureExt as _, StreamExt as _};
    use std::time::Duration;
    use tokio::time::timeout;

    let state = ServerState::in_memory();
    let device = mint_test_device(&state);
    let response = app(state.clone())
        .oneshot(event_request(&device.device_id))
        .await
        .expect("event response");
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    let first = timeout(Duration::from_secs(3), body.next())
        .await
        .expect("initial event deadline")
        .expect("initial event")
        .expect("event bytes");
    assert_app_state_event(&first);

    let next = body.next();
    tokio::pin!(next);
    assert!(
        next.as_mut().now_or_never().is_none(),
        "stream is waiting for another event"
    );
    state.begin_shutdown();
    assert!(
        timeout(Duration::from_millis(250), next)
            .await
            .expect("SSE body must end promptly on shutdown")
            .is_none()
    );
}

#[tokio::test]
async fn event_stream_opened_after_shutdown_ends_immediately() {
    use futures_util::StreamExt as _;
    use std::time::Duration;

    let state = ServerState::in_memory();
    let device = mint_test_device(&state);
    state.begin_shutdown();
    let response = app(state)
        .oneshot(event_request(&device.device_id))
        .await
        .expect("late event response");
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    assert!(
        tokio::time::timeout(Duration::from_millis(250), body.next())
            .await
            .expect("late SSE body must end immediately")
            .is_none()
    );
}

fn event_request(device_id: &str) -> Request<Body> {
    Request::builder()
        .uri(format!("/v1/app/{device_id}/events"))
        .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
        .body(Body::empty())
        .expect("event request")
}

fn assert_app_state_event(bytes: &[u8]) -> serde_json::Value {
    let text = std::str::from_utf8(bytes).expect("SSE text");
    assert!(text.lines().any(|line| line == "event: app-state"));
    let data = text
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .expect("snapshot data");
    let snapshot: serde_json::Value = serde_json::from_str(data).expect("snapshot JSON");
    assert_eq!(snapshot["device"]["connection"]["kind"], "disconnected");
    assert_eq!(snapshot["runtime"]["kind"], "running");
    assert_eq!(snapshot["has_saved_config"], false);
    assert_eq!(
        snapshot["host_protocol_version"],
        protocol::PROTOCOL_VERSION
    );
    assert!(snapshot["config"]["schema_version"].is_number());
    snapshot
}

#[tokio::test]
async fn open_event_stream_allows_graceful_server_shutdown() {
    use std::time::Duration;
    use tokio::time::timeout;

    let state = ServerState::in_memory();
    support::owner_account(&state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind shutdown test server");
    let server = TestServer {
        base_url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let shutdown_state = state.clone();
    let serving = tokio::spawn(async move {
        axum::serve(listener, app(state))
            .with_graceful_shutdown(async move {
                stopped.await.expect("shutdown requested");
                shutdown_state.begin_shutdown();
            })
            .await
            .expect("serve until drained");
    });
    let client = Client::new();
    let device = mint_device(&client, &server).await;
    let cookie = session_cookie(&client, &server).await;
    let expected = snapshot(&client, &server, &device.device_id).await;
    let mut response = timeout(
        Duration::from_secs(3),
        client
            .get(format!(
                "{}/v1/app/{}/events",
                server.base_url, device.device_id
            ))
            .header("cookie", cookie)
            .send(),
    )
    .await
    .expect("event response deadline")
    .expect("event response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut initial = Vec::new();
    timeout(Duration::from_secs(3), async {
        while !initial.windows(2).any(|bytes| bytes == b"\n\n") {
            initial.extend(
                response
                    .chunk()
                    .await
                    .expect("body chunk")
                    .expect("initial event"),
            );
        }
    })
    .await
    .expect("initial event deadline");
    assert_eq!(assert_app_state_event(&initial), expected);

    stop.send(()).expect("request shutdown");
    timeout(Duration::from_secs(3), async {
        while response
            .chunk()
            .await
            .expect("body remains readable")
            .is_some()
        {}
    })
    .await
    .expect("SSE body EOF during graceful shutdown");
    timeout(Duration::from_secs(3), serving)
        .await
        .expect("Axum must finish draining")
        .expect("server task");
}

#[tokio::test]
async fn every_companion_route_refuses_an_anonymous_caller() {
    let (server, _state) = spawn().await;
    let client = Client::new();

    // Asserted as a set rather than one route, because the failure this guards
    // against is a route added later without the extractor -- which would look
    // exactly like a working route in every other test.
    for (method, path, content_type, body) in [
        (Method::GET, "/v1/app/devices", None, None),
        (Method::GET, "/v1/app/desk-1/snapshot", None, None),
        (Method::GET, "/v1/app/desk-1/events", None, None),
        (
            Method::POST,
            "/v1/app/desk-1/config/validate",
            Some("application/json"),
            Some(r#"{"json":"{}"}"#),
        ),
        (
            Method::PUT,
            "/v1/app/desk-1/config",
            Some("application/json"),
            Some(r#"{"json":"{}"}"#),
        ),
        (
            Method::POST,
            "/v1/app/desk-1/preview",
            Some("application/json"),
            Some(r#"{"card_id":"clock-1"}"#),
        ),
        (
            Method::POST,
            "/v1/app/desk-1/pomodoro",
            Some("application/json"),
            Some(r#"{"card_id":"pomodoro","action":"start"}"#),
        ),
    ] {
        let mut request = client.request(method.clone(), format!("{}{path}", server.base_url));
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request.send().await.expect("anonymous request");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path} answered an anonymous caller"
        );
        assert!(
            response.bytes().await.expect("rejection body").is_empty(),
            "{method} {path} returned content to an anonymous caller"
        );
    }
}

#[tokio::test]
async fn the_admin_token_buys_a_session_cookie_and_a_wrong_one_buys_nothing() {
    let (server, _state) = spawn().await;
    let client = Client::new();

    let refused = client
        .post(format!("{}/v1/app/session", server.base_url))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "token": "not-the-admin-token" }).to_string())
        .send()
        .await
        .expect("login attempt");
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert!(
        refused.headers().get("set-cookie").is_none(),
        "a refused login must not mint a session"
    );

    let accepted = client
        .post(format!("{}/v1/app/session", server.base_url))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "token": ADMIN_TOKEN }).to_string())
        .send()
        .await
        .expect("login");
    assert_eq!(accepted.status(), StatusCode::NO_CONTENT);
    let cookie = accepted
        .headers()
        .get("set-cookie")
        .expect("session cookie")
        .to_str()
        .expect("cookie is text");
    // `__Host-` and its attributes are load-bearing, not decoration: without
    // them a sibling host under the registrable domain could set a same-named
    // cookie that the header scan would happily pick up first.
    assert!(cookie.starts_with("__Host-deskmate_session="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("Secure"));
    assert!(cookie.contains("Path=/"));
    assert!(cookie.ends_with("; Max-Age=2592000"));
    assert!(!cookie.to_ascii_lowercase().contains("domain="));
}

#[tokio::test]
async fn an_unlinked_device_reports_its_stored_configuration_and_says_it_is_disconnected() {
    // The board is normally powered off, so this is the ordinary path, not an
    // edge case: the page must render fully with nothing plugged in.
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;

    let body = snapshot(&client, &server, &device.device_id).await;

    assert_eq!(body["device"]["connection"]["kind"], "disconnected");
    assert_eq!(body["runtime"]["kind"], "running");
    assert_eq!(body["has_saved_config"], false);
    assert!(
        body["config"]["schema_version"].is_number(),
        "a snapshot without a board still carries the configuration to edit"
    );
    // Nothing is claimed about a device that has not spoken.
    assert!(body["device"]["firmware_version"].is_null());
    assert!(body["device"]["protocol_version"].is_null());
}

#[tokio::test]
async fn an_unlinked_device_reports_malformed_saved_settings_without_rewriting_them() {
    let root = tempfile::tempdir().expect("config root");
    let state = ServerState::new(
        ADMIN_TOKEN.to_owned(),
        FirmwareCatalog::in_memory(),
        root.path().to_path_buf(),
    );
    let device = mint_test_device(&state);
    let path = device_config_path(&state, &device.device_id);
    std::fs::create_dir_all(path.parent().unwrap()).expect("create devices directory");
    let saved = b"{ definitely not json";
    std::fs::write(&path, saved).expect("write malformed saved settings");

    let body = direct_snapshot(&state, &device.device_id).await;

    assert_eq!(
        body["config"],
        serde_json::to_value(app_core::AppConfig::default()).expect("default config JSON")
    );
    assert_eq!(body["persistence"]["kind"], "recoverable-error");
    assert_eq!(
        body["persistence"]["message"],
        "invalid config JSON: key must be a string at line 1 column 3"
    );
    assert_eq!(body["has_saved_config"], true);
    assert_eq!(
        std::fs::read(path).expect("saved settings after GET"),
        saved
    );
}

#[tokio::test]
async fn an_unlinked_device_reports_invalid_saved_settings_without_rewriting_them() {
    let root = tempfile::tempdir().expect("config root");
    let state = ServerState::new(
        ADMIN_TOKEN.to_owned(),
        FirmwareCatalog::in_memory(),
        root.path().to_path_buf(),
    );
    let device = mint_test_device(&state);
    let path = device_config_path(&state, &device.device_id);
    std::fs::create_dir_all(path.parent().unwrap()).expect("create devices directory");
    let mut invalid =
        serde_json::to_value(app_core::AppConfig::default()).expect("default config JSON");
    invalid["cards"][0]["dwell_seconds"] = serde_json::json!(app_core::MIN_DWELL_SECONDS - 1);
    let parsed: app_core::AppConfig =
        serde_json::from_value(invalid.clone()).expect("deserializable v10 settings");
    let expected_issues = parsed.validate().expect_err("invalid settings").issues;
    let saved = serde_json::to_vec_pretty(&invalid).expect("encode invalid settings");
    std::fs::write(&path, &saved).expect("write invalid saved settings");

    let body = direct_snapshot(&state, &device.device_id).await;

    assert_eq!(
        body["config"],
        serde_json::to_value(app_core::AppConfig::default()).expect("default config JSON")
    );
    assert_eq!(body["persistence"]["kind"], "validation-failed");
    assert_eq!(
        body["persistence"]["message"],
        app_core::SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE
    );
    assert_eq!(
        body["persistence"]["issues"],
        serde_json::to_value(expected_issues).expect("validation issues JSON")
    );
    assert_eq!(body["has_saved_config"], true);
    assert_eq!(
        std::fs::read(path).expect("saved settings after GET"),
        saved
    );
}

#[tokio::test]
async fn unsupported_device_config_does_not_block_startup_listing_or_other_devices() {
    let root = tempfile::tempdir().expect("config root");
    let initial = ServerState::new(
        ADMIN_TOKEN.to_owned(),
        FirmwareCatalog::in_memory(),
        root.path().to_path_buf(),
    );
    let valid = mint_test_device(&initial);
    let unsupported = mint_test_device(&initial);
    let malformed = mint_test_device(&initial);
    let unreadable = mint_test_device(&initial);

    let mut valid_config = app_core::AppConfig::default();
    valid_config.preferences.timezone = "Asia/Tbilisi".into();
    let valid_bytes = serde_json::to_vec_pretty(&valid_config).expect("encode valid config");
    let valid_path = device_config_path(&initial, &valid.device_id);
    std::fs::create_dir_all(valid_path.parent().unwrap()).expect("create devices directory");
    std::fs::write(&valid_path, &valid_bytes).expect("write valid config");

    let unsupported_bytes = br#"{"schema_version":11,"future_body":true}"#;
    let unsupported_path = device_config_path(&initial, &unsupported.device_id);
    std::fs::write(&unsupported_path, unsupported_bytes).expect("write unsupported config");

    let malformed_bytes = b"{ definitely not json";
    let malformed_path = device_config_path(&initial, &malformed.device_id);
    std::fs::write(&malformed_path, malformed_bytes).expect("write malformed config");

    let unreadable_path = device_config_path(&initial, &unreadable.device_id);
    std::fs::create_dir(&unreadable_path).expect("create unreadable stand-in");
    drop(initial);

    let state = ServerState::new(
        ADMIN_TOKEN.to_owned(),
        FirmwareCatalog::in_memory(),
        root.path().to_path_buf(),
    );
    let rows = direct_device_list(&state).await;
    assert_eq!(rows.as_array().map(Vec::len), Some(4));
    for id in [
        &valid.device_id,
        &unsupported.device_id,
        &malformed.device_id,
        &unreadable.device_id,
    ] {
        let row = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == *id)
            .unwrap_or_else(|| panic!("{id} missing from device list"));
        assert_eq!(row["has_saved_config"], true, "{id}");
    }

    let valid_snapshot = direct_snapshot(&state, &valid.device_id).await;
    assert_eq!(valid_snapshot["persistence"]["kind"], "clean");
    assert_eq!(
        valid_snapshot["config"]["preferences"]["timezone"],
        "Asia/Tbilisi"
    );

    let unsupported_snapshot = direct_snapshot(&state, &unsupported.device_id).await;
    assert_eq!(
        unsupported_snapshot["persistence"],
        serde_json::json!({
            "kind": "recoverable-error",
            "message": "config schema version 11 is unsupported; expected 10",
        })
    );
    assert_eq!(
        unsupported_snapshot["config"],
        serde_json::to_value(app_core::AppConfig::default()).unwrap()
    );

    let malformed_snapshot = direct_snapshot(&state, &malformed.device_id).await;
    assert_eq!(
        malformed_snapshot["persistence"]["kind"],
        "recoverable-error"
    );
    let unreadable_snapshot = direct_snapshot(&state, &unreadable.device_id).await;
    assert_eq!(
        unreadable_snapshot["persistence"]["kind"],
        "recoverable-error"
    );

    assert_eq!(std::fs::read(&valid_path).unwrap(), valid_bytes);
    assert_eq!(std::fs::read(&unsupported_path).unwrap(), unsupported_bytes);
    assert_eq!(std::fs::read(&malformed_path).unwrap(), malformed_bytes);
    assert!(unreadable_path.is_dir());
}

#[tokio::test]
async fn an_unknown_device_is_a_typed_not_found_rather_than_an_empty_snapshot() {
    let (server, _state) = spawn().await;
    let client = Client::new();

    let response = client
        .get(format!("{}/v1/app/never-minted/snapshot", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("snapshot");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json_body(response).await;
    // The category is what the frontend switches on; a bare status would leave
    // it with nothing to say.
    assert_eq!(body["category"], "not-found");
}

#[tokio::test]
async fn saving_a_configuration_persists_it_and_the_next_snapshot_shows_it() {
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;

    let before = snapshot(&client, &server, &device.device_id).await;
    let mut config = before["config"].clone();
    config["preferences"]["timezone"] = serde_json::json!("Europe/Berlin");

    let saved = save_config(&client, &server, &device.device_id, &config).await;
    assert_eq!(saved.status(), StatusCode::OK);

    let after = snapshot(&client, &server, &device.device_id).await;
    assert_eq!(after["config"]["preferences"]["timezone"], "Europe/Berlin");
    // The save is what makes a document exist on disk, and the page uses this
    // to tell a first run from an edited one.
    assert_eq!(after["has_saved_config"], true);
}

#[tokio::test]
async fn a_malformed_draft_is_an_invalid_payload() {
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;

    let malformed = serde_json::json!({ "schema_version": 10, "cards": [] });
    let response = save_config(&client, &server, &device.device_id, &malformed).await;
    // A shape that does not deserialize is the caller's payload, not a
    // validation verdict about a configuration -- the two are different repairs.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["category"], "invalid-payload");
}

#[tokio::test]
async fn semantic_validation_reports_issues_and_preserves_the_last_good_document() {
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;
    let before = snapshot(&client, &server, &device.device_id).await;
    let mut config = before["config"].clone();
    config["cards"][0]["dwell_seconds"] = serde_json::json!(app_core::MIN_DWELL_SECONDS - 1);
    let envelope = serde_json::json!({ "json": config.to_string() }).to_string();

    let preflight = client
        .post(format!(
            "{}/v1/app/{}/config/validate",
            server.base_url, device.device_id
        ))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(envelope.clone())
        .send()
        .await
        .expect("validate config");
    assert_eq!(preflight.status(), StatusCode::OK);
    let preflight = json_body(preflight).await;
    assert_eq!(preflight["valid"], false);
    let issues = preflight["issues"]
        .as_array()
        .filter(|issues| !issues.is_empty())
        .expect("semantic validation issues");
    assert!(issues.iter().any(|issue| {
        issue["path"] == "cards[0].dwell_seconds"
            && issue["code"] == "out-of-range"
            && issue["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty())
    }));
    let expected_issues = preflight["issues"].clone();

    let saved = put_config_body(&client, &server, &device.device_id, envelope).await;
    assert_eq!(saved.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let saved = json_body(saved).await;
    assert_eq!(saved["category"], "validation");
    assert_eq!(saved["issues"], expected_issues);

    let after = snapshot(&client, &server, &device.device_id).await;
    assert_eq!(after["config"], before["config"]);
    assert_eq!(after["has_saved_config"], before["has_saved_config"]);
}

#[tokio::test]
async fn a_draft_without_a_wire_lowering_is_refused_and_remains_unsaved() {
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;
    let before = snapshot(&client, &server, &device.device_id).await;
    let mut config = before["config"].clone();
    config["cards"][0]["tap_action"] = serde_json::json!({
        "kind": "open-url",
        "url": "https://example.test/action",
    });
    let parsed: app_core::AppConfig =
        serde_json::from_value(config.clone()).expect("deserializable configuration");
    parsed
        .validate()
        .expect("host action passes ordinary validation");

    let response = save_config(&client, &server, &device.device_id, &config).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(response).await;
    assert_eq!(body["category"], "validation");
    assert!(body["issues"].as_array().is_some_and(|issues| {
        issues
            .iter()
            .any(|issue| issue["path"] == "cards[0]" && issue["code"] == "requires-capability")
    }));

    let after = snapshot(&client, &server, &device.device_id).await;
    assert_eq!(after["has_saved_config"], false);
    assert_eq!(after["config"], before["config"]);
}

#[tokio::test]
async fn a_draft_over_the_size_limit_is_refused_before_it_is_parsed() {
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;

    // Deliberately not valid JSON: if the size check did not come first, this
    // would be reported as a parse failure and the test would catch it.
    let oversized = "x".repeat(64 * 1024 + 1);
    let response = client
        .post(format!(
            "{}/v1/app/{}/config/validate",
            server.base_url, device.device_id
        ))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "json": oversized }).to_string())
        .send()
        .await
        .expect("validate draft");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = json_body(response).await;
    assert_eq!(body["category"], "payload-too-large");
    assert_eq!(body["maximum_bytes"], 64 * 1024);
}

#[tokio::test]
async fn a_pomodoro_command_with_no_board_says_so_instead_of_silently_succeeding() {
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;

    let response = client
        .post(format!(
            "{}/v1/app/{}/pomodoro",
            server.base_url, device.device_id
        ))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "card_id": "pomodoro", "action": "start" }).to_string())
        .send()
        .await
        .expect("pomodoro");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json_body(response).await;
    assert_eq!(body["category"], "runtime-unavailable");
}

#[tokio::test]
async fn without_a_web_root_the_ui_paths_are_simply_absent() {
    // An API-only deployment must leave UI paths unmounted rather than answering
    // them with the SPA shell.
    let (server, _state) = spawn().await;
    let response = Client::new()
        .get(&server.base_url)
        .send()
        .await
        .expect("root request");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_spa_fallback_serves_client_routes_but_never_shadows_the_api() {
    let root = tempfile::tempdir().expect("web root");
    std::fs::write(root.path().join("index.html"), "<!doctype html>shell").expect("write shell");
    std::fs::create_dir_all(root.path().join("assets")).expect("assets dir");
    std::fs::write(root.path().join("assets/app.js"), "console.log(1)").expect("write asset");

    let server = spawn_with(ServerState::in_memory(), Some(root.path().to_path_buf())).await;
    let client = Client::new();

    let shell = client
        .get(&server.base_url)
        .send()
        .await
        .expect("root request");
    assert_eq!(shell.status(), StatusCode::OK);
    assert_eq!(
        shell.text().await.expect("shell body"),
        "<!doctype html>shell"
    );

    // An unknown non-API path is a client route, so it gets the shell.
    let client_route = client
        .get(format!("{}/settings/cards", server.base_url))
        .send()
        .await
        .expect("client route");
    assert_eq!(client_route.status(), StatusCode::OK);
    assert!(client_route.text().await.expect("body").contains("shell"));

    let asset = client
        .get(format!("{}/assets/app.js", server.base_url))
        .send()
        .await
        .expect("asset");
    assert_eq!(asset.status(), StatusCode::OK);
    assert_eq!(
        asset
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/javascript; charset=utf-8")
    );

    // The one that matters: a `/v1` path the API does not define must stay a
    // 404, or a JSON client gets an HTML page and a confusing parse error.
    let unknown_api = client
        .get(format!("{}/v1/not-a-route", server.base_url))
        .send()
        .await
        .expect("unknown api route");
    assert_eq!(unknown_api.status(), StatusCode::NOT_FOUND);
    assert!(!unknown_api.text().await.expect("body").contains("shell"));

    // And a real API route still answers as itself rather than as the shell.
    let devices = client
        .get(format!("{}/v1/app/devices", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("devices");
    assert_eq!(devices.status(), StatusCode::OK);
    assert_eq!(devices.text().await.expect("body"), "[]");
}

#[tokio::test]
async fn the_spa_cache_policy_distinguishes_shell_from_fingerprinted_assets() {
    let root = tempfile::tempdir().expect("web root");
    let shell = b"<!doctype html>shell";
    let asset = b"console.log(1)";
    std::fs::write(root.path().join("index.html"), shell).expect("write shell");
    std::fs::create_dir_all(root.path().join("assets")).expect("assets dir");
    std::fs::write(root.path().join("assets/app-abc123.js"), asset).expect("write asset");
    let app = app_with_web(ServerState::in_memory(), Some(root.path().to_path_buf()));

    for path in ["/", "/index.html", "/settings/cards"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("shell request"),
            )
            .await
            .expect("shell response");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("no-cache"),
            "{path} must not cache the deploy-varying shell"
        );
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("shell body")
                .as_ref(),
            shell,
            "{path} shell body"
        );
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri("/assets/app-abc123.js")
                .body(Body::empty())
                .expect("asset request"),
        )
        .await
        .expect("asset response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("asset body")
            .as_ref(),
        asset
    );
}

#[tokio::test]
async fn a_traversal_out_of_the_web_root_is_refused_rather_than_served() {
    let root = tempfile::tempdir().expect("fixture root");
    let web = root.path().join("web");
    std::fs::create_dir(&web).expect("web root");
    let shell = b"<!doctype html>shell";
    let secret = b"do not serve me";
    std::fs::write(web.join("index.html"), shell).expect("write shell");
    std::fs::write(root.path().join("secret.txt"), secret).expect("write secret");

    let response = app_with_web(ServerState::in_memory(), Some(web))
        .oneshot(
            Request::builder()
                .uri("/../secret.txt")
                .body(Body::empty())
                .expect("traversal request"),
        )
        .await
        .expect("traversal response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(body.as_ref(), shell);
    assert!(
        !body.windows(secret.len()).any(|window| window == secret),
        "the web root must not be escapable"
    );
}

#[tokio::test]
async fn the_device_and_firmware_surfaces_are_untouched_by_the_companion_routes() {
    // Browser sessions must not change the board's bearer gate or the firmware
    // download's deliberately unauthenticated contract. No edge gate exists.
    let state = ServerState::in_memory();
    let server = spawn_with(state, None).await;
    let client = Client::new();

    let response = client
        .get(format!("{}/v1/device/firmware", server.base_url))
        .send()
        .await
        .expect("firmware check");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "the device surface still wants a device bearer, not a session"
    );

    // `app()` and `app_with_web(.., None)` must be the same router; a difference
    // here would mean the UI wiring had changed the device contract.
    let plain = spawn_with_plain().await;
    let plain_response = Client::new()
        .get(format!("{}/v1/device/firmware", plain.base_url))
        .send()
        .await
        .expect("firmware check");
    assert_eq!(plain_response.status(), StatusCode::UNAUTHORIZED);
}

async fn spawn_with_plain() -> TestServer {
    let state = ServerState::in_memory();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    tokio::spawn(async move {
        axum::serve(listener, app(state))
            .await
            .expect("serve plain routes");
    });
    TestServer {
        base_url: format!("http://127.0.0.1:{}", address.port()),
    }
}

#[tokio::test]
async fn the_image_and_face_routes_answer_a_browser_session_not_only_a_bearer() {
    // The browser reaches these with a cookie because it cannot put a
    // bearer on every request without keeping the admin token where script can
    // read it. Gating them on the bearer alone made the add menu, the source
    // list and the face settings all fail from the page while every
    // bearer-carrying test passed -- so this test carries a cookie on purpose.
    let (server, _state) = spawn().await;
    let client = Client::new();
    // The cookie is replayed by hand rather than with reqwest's cookie store,
    // which would need a feature this crate deliberately does not enable -- and
    // which would refuse a `Secure` cookie over the loopback http the harness
    // uses anyway. In deployment the origin is HTTPS, so `Secure` costs nothing.
    let cookie = session_cookie(&client, &server).await;

    for path in ["/v1/images", "/v1/faces"] {
        let response = client
            .get(format!("{}{path}", server.base_url))
            .header("cookie", &cookie)
            .send()
            .await
            .expect("session request");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{path} refused a signed-in browser"
        );
    }

    let minted = client
        .post(format!("{}/v1/images", server.base_url))
        .header("cookie", &cookie)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "name": "Weather", "face_kind": null }).to_string())
        .send()
        .await
        .expect("mint from the browser session");
    assert_eq!(minted.status(), StatusCode::OK);
}

/// Signs in and returns the `Cookie` header a browser would then send.
async fn session_cookie(client: &Client, server: &TestServer) -> String {
    let login = client
        .post(format!("{}/v1/app/session", server.base_url))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "token": ADMIN_TOKEN }).to_string())
        .send()
        .await
        .expect("login");
    assert_eq!(login.status(), StatusCode::NO_CONTENT);
    let set_cookie = login
        .headers()
        .get("set-cookie")
        .expect("session cookie")
        .to_str()
        .expect("cookie is text");
    set_cookie
        .split(';')
        .next()
        .expect("cookie name=value")
        .to_owned()
}

#[tokio::test]
async fn the_device_list_says_which_identities_have_ever_been_configured() {
    // The page opens on one display, and registry order is mint order. A
    // server that has minted spare identities over time lists several that were
    // never configured; without this flag the page would open on the first of
    // those and show an empty loop for a display nobody owns. Observed on the
    // live server, which lists dev-0001 first and keeps the real panel at
    // dev-0005.
    let (server, _state) = spawn().await;
    let client = Client::new();
    let first = mint_device(&client, &server).await;
    let second = mint_device(&client, &server).await;

    let configured = snapshot(&client, &server, &second.device_id).await;
    let saved = save_config(&client, &server, &second.device_id, &configured["config"]).await;
    assert_eq!(saved.status(), StatusCode::OK);

    let listed = client
        .get(format!("{}/v1/app/devices", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("devices");
    let rows = json_body(listed).await;
    let row = |id: &str| {
        rows.as_array()
            .expect("device rows")
            .iter()
            .find(|row| row["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} missing from the device list"))
    };
    assert_eq!(row(&first.device_id)["has_saved_config"], false);
    assert_eq!(row(&second.device_id)["has_saved_config"], true);
    // The recency stamp is the tie-breaker `has_saved_config` cannot provide:
    // the live server has three configured identities and one panel, so the
    // page has to prefer the one most recently written to.
    assert!(row(&first.device_id)["configured_at"].is_null());
    assert!(
        row(&second.device_id)["configured_at"].as_i64().is_some(),
        "a written configuration must carry when it was written"
    );
}

#[tokio::test]
async fn preview_stays_upright_for_both_physical_mountings() {
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/one-pomodoro-card.json"
    ))
    .expect("fixture");
    let mut config: serde_json::Value = serde_json::from_str(&fixture).expect("fixture JSON");
    let mut previews = Vec::new();

    for orientation in ["landscape", "landscape-flipped"] {
        config["preferences"]["orientation"] = serde_json::json!(orientation);
        let saved = save_config(&client, &server, &device.device_id, &config).await;
        assert_eq!(saved.status(), StatusCode::OK);

        let stored = snapshot(&client, &server, &device.device_id).await;
        assert_eq!(stored["config"]["preferences"]["orientation"], orientation);

        let response = client
            .post(format!(
                "{}/v1/app/{}/preview",
                server.base_url, device.device_id
            ))
            .bearer_auth(ADMIN_TOKEN)
            .header("content-type", "application/json")
            .body(serde_json::json!({ "card_id": "pomodoro" }).to_string())
            .send()
            .await
            .expect("preview");
        assert_eq!(response.status(), StatusCode::OK);
        let frame = json_body(response).await;
        assert!(
            frame["png_base64"]
                .as_str()
                .is_some_and(|png| !png.is_empty()),
            "preview returned no PNG: {frame}"
        );
        assert_eq!(frame["sample"], true);
        assert!(frame["state"].is_null());
        previews.push(frame["png_base64"].clone());
    }

    assert_eq!(
        previews[0], previews[1],
        "physical mounting changed the person's upright preview"
    );
}

#[tokio::test]
async fn saving_a_configuration_revokes_the_sources_it_no_longer_declares() {
    // The live server had accumulated "Weather 2" and "Weather 3": credentialed
    // image sources no card referenced, because minting happens when the owner
    // picks a face from the add menu -- a draft action against a server resource,
    // with nothing joining the two transactions. Abandoning the draft left the
    // source behind forever, and the add menu offered it back as reusable.
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;

    let kept = mint_source(&client, &server, "Weather").await;
    let abandoned = mint_source(&client, &server, "Weather 2").await;
    assert_eq!(listed_source_ids(&client, &server).await.len(), 2);

    // A configuration that declares only the first one, with a card using it.
    let mut config = snapshot(&client, &server, &device.device_id).await["config"].clone();
    config["image_sources"] = serde_json::json!([{ "id": kept, "name": "Weather" }]);
    config["cards"] = serde_json::json!([{
        "kind": "picture",
        "id": "weather",
        "title": "Weather",
        "source_id": kept,
        "tap_action": { "kind": "none" },
        "refresh": { "kind": "manual" },
        "alert": { "kind": "none" },
        "dwell_seconds": null,
    }]);
    let saved = save_config(&client, &server, &device.device_id, &config).await;
    assert_eq!(saved.status(), StatusCode::OK);

    let remaining = listed_source_ids(&client, &server).await;
    assert!(remaining.contains(&kept), "a declared source must survive");
    assert!(
        !remaining.contains(&abandoned),
        "a source the configuration stopped declaring must be revoked"
    );
}

#[tokio::test]
async fn saving_a_configuration_removes_an_abandoned_face_and_preserves_a_declared_one() {
    let root = tempfile::tempdir().expect("config root");
    let state = ServerState::new(
        ADMIN_TOKEN.to_owned(),
        FirmwareCatalog::in_memory(),
        root.path().to_path_buf(),
    );
    let server = spawn_with(state, None).await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;
    let kept = mint_face_source(&client, &server, "Weather", "weather").await;
    let abandoned = mint_face_source(&client, &server, "News", "rss").await;

    let mut config = snapshot(&client, &server, &device.device_id).await["config"].clone();
    config["image_sources"] = serde_json::json!([{ "id": kept, "name": "Weather" }]);
    config["cards"] = serde_json::json!([{
        "kind": "picture",
        "id": "weather",
        "title": "Weather",
        "source_id": kept,
        "tap_action": { "kind": "none" },
        "refresh": { "kind": "manual" },
        "alert": { "kind": "none" },
        "dwell_seconds": null,
    }]);
    let saved = save_config(&client, &server, &device.device_id, &config).await;

    assert_eq!(saved.status(), StatusCode::OK);
    assert!(
        !listed_source_ids(&client, &server)
            .await
            .contains(&abandoned)
    );
    let bytes = std::fs::read(account_root(root.path()).join("data-cards.json"))
        .expect("persisted face specs");
    let persisted: serde_json::Value =
        serde_json::from_slice(&bytes).expect("persisted face specs parse");
    let specs = persisted.as_array().expect("persisted face spec list");
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0]["source_id"].as_str(), Some(kept.as_str()));
}

#[tokio::test]
async fn config_save_succeeds_when_source_reconciliation_persistence_fails() {
    let root = tempfile::tempdir().expect("config root");
    let state = ServerState::new(
        ADMIN_TOKEN.to_owned(),
        FirmwareCatalog::in_memory(),
        root.path().to_path_buf(),
    );
    let server = spawn_with(state, None).await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;
    let source = mint_face_source(&client, &server, "Weather", "weather").await;
    let config = snapshot(&client, &server, &device.device_id).await["config"].clone();
    let spec_path = account_root(root.path()).join("data-cards.json");
    let before = std::fs::read(&spec_path).expect("face specs");
    replace_file_with_directory(&account_root(root.path()).join("image-sources.json"));

    let saved = save_config(&client, &server, &device.device_id, &config).await;

    assert_eq!(saved.status(), StatusCode::OK);
    assert!(listed_source_ids(&client, &server).await.contains(&source));
    assert_eq!(std::fs::read(spec_path).expect("unchanged specs"), before);
}

#[tokio::test]
async fn config_save_succeeds_when_face_reconciliation_persistence_fails() {
    let root = tempfile::tempdir().expect("config root");
    let state = ServerState::new(
        ADMIN_TOKEN.to_owned(),
        FirmwareCatalog::in_memory(),
        root.path().to_path_buf(),
    );
    let server = spawn_with(state, None).await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;
    let source = mint_face_source(&client, &server, "Weather", "weather").await;
    let config = snapshot(&client, &server, &device.device_id).await["config"].clone();
    let spec_path = account_root(root.path()).join("data-cards.json");
    replace_file_with_directory(&spec_path);

    let saved = save_config(&client, &server, &device.device_id, &config).await;

    assert_eq!(saved.status(), StatusCode::OK);
    assert!(!listed_source_ids(&client, &server).await.contains(&source));
    assert!(spec_path.is_dir(), "the failing face target was replaced");
}

#[tokio::test]
async fn a_configuration_that_still_declares_everything_revokes_nothing() {
    // The guard against the shape that has bitten this project before: a
    // KEEP-set read as a delete-list. `AssetRelease.digests` once meant "wipe
    // every asset you hold" when the desired set arrived empty.
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;

    let first = mint_source(&client, &server, "Weather").await;
    let second = mint_source(&client, &server, "Hacker News").await;

    let mut config = snapshot(&client, &server, &device.device_id).await["config"].clone();
    config["image_sources"] = serde_json::json!([
        { "id": first, "name": "Weather" },
        { "id": second, "name": "Hacker News" },
    ]);
    let saved = save_config(&client, &server, &device.device_id, &config).await;
    assert_eq!(saved.status(), StatusCode::OK);

    let remaining = listed_source_ids(&client, &server).await;
    assert!(remaining.contains(&first));
    assert!(remaining.contains(&second));
}

async fn mint_source(client: &Client, server: &TestServer, name: &str) -> String {
    mint_source_with_face(client, server, name, None).await
}

async fn mint_face_source(
    client: &Client,
    server: &TestServer,
    name: &str,
    face_kind: &str,
) -> String {
    mint_source_with_face(client, server, name, Some(face_kind)).await
}

async fn mint_source_with_face(
    client: &Client,
    server: &TestServer,
    name: &str,
    face_kind: Option<&str>,
) -> String {
    let mut body = serde_json::json!({ "name": name });
    if let Some(face_kind) = face_kind {
        body["face_kind"] = face_kind.into();
    }
    let response = client
        .post(format!("{}/v1/images", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("mint image source");
    assert_eq!(response.status(), StatusCode::OK);
    // The route answers `id`, not `source_id`. Naming it wrong here is the same
    // mistake that crashed the page: a stub or helper that believes a shape the
    // server does not send agrees with the code and disagrees with reality.
    json_body(response).await["id"]
        .as_str()
        .expect("source id")
        .to_owned()
}

async fn save_config(
    client: &Client,
    server: &TestServer,
    device_id: &str,
    config: &serde_json::Value,
) -> reqwest::Response {
    put_config_body(
        client,
        server,
        device_id,
        serde_json::json!({ "json": config.to_string() }).to_string(),
    )
    .await
}

async fn put_config_body(
    client: &Client,
    server: &TestServer,
    device_id: &str,
    body: String,
) -> reqwest::Response {
    client
        .put(format!("{}/v1/app/{device_id}/config", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("save config")
}

async fn listed_source_ids(client: &Client, server: &TestServer) -> Vec<String> {
    let response = client
        .get(format!("{}/v1/images", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("list image sources");
    json_body(response)
        .await
        .as_array()
        .expect("source rows")
        .iter()
        .map(|row| row["id"].as_str().expect("source id").to_owned())
        .collect()
}

async fn preview_of(
    client: &Client,
    server: &TestServer,
    device_id: &str,
    card_id: &str,
) -> serde_json::Value {
    let response = client
        .post(format!("{}/v1/app/{device_id}/preview", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "card_id": card_id }).to_string())
        .send()
        .await
        .expect("preview");
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}

#[tokio::test]
async fn a_picture_cards_preview_is_the_frame_its_source_last_drew() {
    use base64::Engine as _;
    use std::time::Duration;

    // Until 2026-09-20 every picture card previewed as a black rectangle with a
    // sentence in it, so a weather or token card the owner had just created looked
    // unfinished in the one place they could look at it.
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;
    // `headlines` is complete at birth, so the fake package draws it straight away;
    // a plain source has no producer in this test and never gets a frame.
    let drawn = mint_face_source(&client, &server, "Headlines", "headlines").await;
    let silent = mint_source(&client, &server, "Camera").await;

    let mut config = snapshot(&client, &server, &device.device_id).await["config"].clone();
    config["image_sources"] = serde_json::json!([
        { "id": drawn, "name": "Headlines" },
        { "id": silent, "name": "Camera" },
    ]);
    let picture = |id: &str, source: &str| {
        serde_json::json!({
            "kind": "picture", "id": id, "title": id, "source_id": source,
            "tap_action": { "kind": "none" }, "refresh": { "kind": "manual" },
            "alert": { "kind": "none" }, "dwell_seconds": null,
        })
    };
    config["cards"] = serde_json::json!([picture("drawn", &drawn), picture("silent", &silent)]);
    let saved = save_config(&client, &server, &device.device_id, &config).await;
    assert_eq!(saved.status(), StatusCode::OK);

    let waiting = preview_of(&client, &server, &device.device_id, "silent").await;
    assert!(waiting["png_base64"].is_null());
    assert!(
        waiting["state"]
            .as_str()
            .is_some_and(|state| state.starts_with("No frame yet")),
        "a source that has drawn nothing says so: {waiting}"
    );

    let mut frame = serde_json::Value::Null;
    for _ in 0..100 {
        frame = preview_of(&client, &server, &device.device_id, "drawn").await;
        if frame["png_base64"].is_string() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let png = base64::engine::general_purpose::STANDARD
        .decode(
            frame["png_base64"]
                .as_str()
                .expect("the face was drawn within five seconds"),
        )
        .expect("base64");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let dimension = |offset: usize| u32::from_be_bytes(png[offset..offset + 4].try_into().unwrap());
    assert_eq!(
        (dimension(16), dimension(20)),
        (448, 368),
        "one whole canvas"
    );
    assert_eq!(
        frame["sample"], false,
        "these are real pixels, not defaults"
    );
    assert!(frame["state"].is_null());
}
