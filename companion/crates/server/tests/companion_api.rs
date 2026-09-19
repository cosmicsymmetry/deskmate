//! HTTP contract for the browser companion's routes.
//!
//! These exercise the real router over a real loopback socket, the way
//! `image_routes.rs` does, because the things most likely to break here are not
//! reachable from a unit test: which gate a route is behind, what a route
//! answers when the board is absent, and whether the SPA fallback can shadow the
//! API.

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use server::firmware::FirmwareCatalog;
use server::{ServerState, app, app_with_web};

const ADMIN_TOKEN: &str = "in-memory-admin-token";

struct TestServer {
    base_url: String,
}

async fn spawn_with(state: ServerState, web_root: Option<std::path::PathBuf>) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    tokio::spawn(async move {
        axum::serve(listener, app_with_web(state, web_root))
            .await
            .expect("serve companion routes");
    });
    TestServer {
        base_url: format!("http://127.0.0.1:{}", address.port()),
    }
}

async fn spawn() -> (TestServer, ServerState) {
    let state = ServerState::in_memory();
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

async fn json_body(response: reqwest::Response) -> serde_json::Value {
    let text = response.text().await.expect("response body");
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("body is not JSON ({error}): {text}"))
}

#[tokio::test]
async fn every_companion_route_refuses_an_anonymous_caller() {
    let (server, _state) = spawn().await;
    let client = Client::new();

    // Asserted as a set rather than one route, because the failure this guards
    // against is a route added later without the extractor -- which would look
    // exactly like a working route in every other test.
    for path in [
        "/v1/app/devices",
        "/v1/app/desk-1/snapshot",
        "/v1/app/desk-1/events",
    ] {
        let response = client
            .get(format!("{}{path}", server.base_url))
            .send()
            .await
            .expect("anonymous request");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{path} answered an anonymous caller"
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

    let saved = client
        .put(format!(
            "{}/v1/app/{}/config",
            server.base_url, device.device_id
        ))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "json": config.to_string() }).to_string())
        .send()
        .await
        .expect("save config");
    assert_eq!(saved.status(), StatusCode::OK);

    let after = snapshot(&client, &server, &device.device_id).await;
    assert_eq!(after["config"]["preferences"]["timezone"], "Europe/Berlin");
    // The save is what makes a document exist on disk, and the page uses this
    // to tell a first run from an edited one.
    assert_eq!(after["has_saved_config"], true);
}

#[tokio::test]
async fn an_invalid_draft_is_refused_with_the_issues_that_explain_it() {
    let (server, _state) = spawn().await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;

    let response = client
        .put(format!(
            "{}/v1/app/{}/config",
            server.base_url, device.device_id
        ))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(
            serde_json::json!({
                "json": serde_json::json!({ "schema_version": 10, "cards": [] }).to_string()
            })
            .to_string(),
        )
        .send()
        .await
        .expect("save config");
    // A shape that does not deserialize is the caller's payload, not a
    // validation verdict about a configuration -- the two are different repairs.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["category"], "invalid-payload");
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
async fn a_traversal_out_of_the_web_root_is_refused_rather_than_served() {
    let root = tempfile::tempdir().expect("web root");
    std::fs::write(root.path().join("index.html"), "<!doctype html>shell").expect("write shell");
    let secret = root.path().parent().expect("parent").join("secret.txt");
    std::fs::write(&secret, "do not serve me").expect("write secret");

    let server = spawn_with(ServerState::in_memory(), Some(root.path().to_path_buf())).await;
    let response = Client::new()
        .get(format!("{}/../secret.txt", server.base_url))
        .send()
        .await
        .expect("traversal attempt");
    let body = response.text().await.expect("body");
    assert!(
        !body.contains("do not serve me"),
        "the web root must not be escapable"
    );
    let _ = std::fs::remove_file(secret);
}

#[tokio::test]
async fn the_device_and_firmware_surfaces_are_untouched_by_the_companion_routes() {
    // The board and the OTA path must keep their own auth, because the edge gate
    // that fronts the UI deliberately does not cover them.
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
    let saved = client
        .put(format!(
            "{}/v1/app/{}/config",
            server.base_url, second.device_id
        ))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "json": configured["config"].to_string() }).to_string())
        .send()
        .await
        .expect("save config");
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
    let saved = client
        .put(format!(
            "{}/v1/app/{}/config",
            server.base_url, device.device_id
        ))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "json": config.to_string() }).to_string())
        .send()
        .await
        .expect("save config");
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
    let saved = client
        .put(format!(
            "{}/v1/app/{}/config",
            server.base_url, device.device_id
        ))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "json": config.to_string() }).to_string())
        .send()
        .await
        .expect("save config");

    assert_eq!(saved.status(), StatusCode::OK);
    assert!(
        !listed_source_ids(&client, &server)
            .await
            .contains(&abandoned)
    );
    let bytes = std::fs::read(root.path().join("data-cards.json")).expect("persisted face specs");
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
    let spec_path = root.path().join("data-cards.json");
    let before = std::fs::read(&spec_path).expect("face specs");
    replace_file_with_directory(&root.path().join("image-sources.json"));

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
    let spec_path = root.path().join("data-cards.json");
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
    let saved = client
        .put(format!(
            "{}/v1/app/{}/config",
            server.base_url, device.device_id
        ))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "json": config.to_string() }).to_string())
        .send()
        .await
        .expect("save config");
    assert_eq!(saved.status(), StatusCode::OK);

    let remaining = listed_source_ids(&client, &server).await;
    assert!(remaining.contains(&first));
    assert!(remaining.contains(&second));
}

async fn mint_source(client: &Client, server: &TestServer, name: &str) -> String {
    let response = client
        .post(format!("{}/v1/images", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "name": name }).to_string())
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

async fn mint_face_source(
    client: &Client,
    server: &TestServer,
    name: &str,
    face_kind: &str,
) -> String {
    let response = client
        .post(format!("{}/v1/images", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "name": name, "face_kind": face_kind }).to_string())
        .send()
        .await
        .expect("mint server face");
    assert_eq!(response.status(), StatusCode::OK);
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
    client
        .put(format!("{}/v1/app/{device_id}/config", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "json": config.to_string() }).to_string())
        .send()
        .await
        .expect("save config")
}

fn replace_file_with_directory(path: &std::path::Path) {
    std::fs::rename(path, path.with_extension("backup")).expect("preserve original file");
    std::fs::create_dir(path).expect("blocking directory");
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
