//! The admin card-preview route, seen from the device's end of the link.
//!
//! `crates/server/src/admin.rs` covers what the route *returns* for each of
//! §4.2's outcomes, against the real `ServerPluginHost` and the committed
//! fixtures. This file covers the other half of the claim, which no in-crate
//! test can see: rendering a preview adds nothing to the transcript the
//! device receives.

use std::time::Duration;

use server::{ServerState, app};

mod support;

/// Boots a server on a loopback port and mints one device identity.
/// Mirrors `tests/ownership.rs`'s helper; kept separate so the two files can
/// diverge without one silently changing the other's fixture.
async fn spawn() -> (String, server::registry::DeviceIdentity, String) {
    let state = ServerState::in_memory();
    let identity = state.registry().mint().expect("mint identity");
    let admin_token = support::IN_MEMORY_ADMIN_TOKEN.to_string();
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

// The Err type is tungstenite's, so its size is not ours to reduce.
#[allow(clippy::result_large_err)]
async fn connect_device(
    host: &str,
    token: &str,
) -> Result<support::DeviceSocket, tokio_tungstenite::tungstenite::Error> {
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
        .map(|(socket, _)| socket)
}

async fn admin_json(client: &reqwest::Client, url: String, admin_token: &str) -> serde_json::Value {
    let response = client
        .get(url)
        .bearer_auth(admin_token)
        .send()
        .await
        .expect("admin GET");
    let body = response.text().await.expect("admin response body");
    serde_json::from_str(&body).expect("admin response JSON")
}

/// Waits for the plugin card's one scheduled provider refresh to be applied,
/// answering device traffic between polls. This server has no plugin
/// registry, so the refresh fails immediately and locally -- no egress, no
/// DNS -- and this settles in milliseconds rather than on a network timeout.
async fn settle_first_refresh(
    socket: &mut support::DeviceSocket,
    client: &reqwest::Client,
    host: &str,
    device_id: &str,
    admin_token: &str,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        support::flush_socket(socket).await;
        let status = admin_json(
            client,
            format!("http://{host}/v1/devices/{device_id}"),
            admin_token,
        )
        .await;
        let settled = status["snapshot"]["providers"]
            .as_array()
            .is_some_and(|providers| {
                providers.iter().any(|provider| {
                    provider["widget_id"] == "aqi-card" && provider["state"]["kind"] == "error"
                })
            });
        if settled {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the plugin card's provider never reported: {status}"
        );
    }
}

#[tokio::test]
async fn rendering_a_preview_adds_nothing_to_the_device_transcript() {
    let (host, identity, admin_token) = spawn().await;
    let mut socket = connect_device(&host, &identity.token)
        .await
        .expect("connect");
    let client = reqwest::Client::new();

    // This server's plugin registry is empty, so the card's provider fails
    // without any egress at all. That is deliberate: the curated manifests
    // point at `example.invalid`, so a live socket test could only ever wait
    // on a DNS failure, and what this test measures is the transcript, not
    // the frame. The four render outcomes are covered in `admin.rs` against
    // the real `ServerPluginHost` and a fixture refresher that always
    // supplies a value.
    //
    // Here the *real* `ServerProviderRefresher` runs, and "plugin not
    // loaded" is `plugin_error_result` (`value: None`, `error: Some(..)`).
    // `apply_provider_result` (`app-core/src/runtime.rs`) only inserts into
    // `state.plugin_snapshots` `if let Some(value) = result.value`, so this
    // card never gets a cached snapshot at all. `render_card_preview`
    // (`app-core/src/runtime.rs`) consults the card's own provider state
    // before falling into "Waiting for the first refresh": a provider that
    // already reported `Error` previews as `error` with that provider's own
    // message rather than promising a resolution that can never come, so the
    // preview below settles on `error`, the same outcome
    // `settle_first_refresh` waits for on the device-status side.
    let config = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/one-plugin-card.json"
    ))
    .expect("fixture");
    let put = client
        .put(format!(
            "http://{host}/v1/devices/{}/config",
            identity.device_id
        ))
        .bearer_auth(&admin_token)
        .header("Content-Type", "application/json")
        .body(config)
        .send();
    let (response, applied) =
        tokio::join!(put, support::drive_until_config(&mut socket, "aqi-card"));
    assert_eq!(response.expect("config PUT").status(), 200);
    assert_eq!(applied.widgets.len(), 1);

    // The card's one scheduled refresh must land before the windows below, or
    // its `PushData` would be attributed to the preview. Nothing else is due
    // afterwards: the cadence is 15 minutes, the playlist advances manually,
    // and a plugin card with no snapshot has its scene refused once and
    // recorded, never retried.
    settle_first_refresh(
        &mut socket,
        &client,
        &host,
        &identity.device_id,
        &admin_token,
    )
    .await;
    support::flush_socket(&mut socket).await;

    // Control. Without it the empty transcript below would pass just as well
    // against a collector that reads nothing at all.
    let push = client
        .post(format!(
            "http://{host}/v1/devices/{}/scene",
            identity.device_id
        ))
        .bearer_auth(&admin_token)
        .header("Content-Type", "application/json")
        .body(
            serde_json::json!({
                "card_id": "aqi-card",
                "revision": 17,
                "template": "digital_clock",
                "show_seconds": false,
                "local_now": "2026-08-25T14:37:42",
            })
            .to_string(),
        )
        .send();
    let (pushed, control) = tokio::join!(
        push,
        support::card_messages_during(&mut socket, Duration::from_millis(750))
    );
    assert_eq!(pushed.expect("scene push").status(), 200);
    assert_eq!(control, vec!["PushScene"]);

    // The measurement: five previews, and the device sees nothing.
    let previews = async {
        for _ in 0..5 {
            let body = admin_json(
                &client,
                format!(
                    "http://{host}/v1/devices/{}/cards/aqi-card/preview",
                    identity.device_id
                ),
                &admin_token,
            )
            .await;
            assert_eq!(body["state"], "error", "unexpected preview body: {body}");
            assert!(body["png_base64"].is_null());
        }
    };
    // This window is wider than the control window above on purpose: the
    // production runtime's `RuntimeOptions::default().status_interval` is 2
    // seconds, and `card_message_name` deliberately excludes `StatusRequest`
    // as link housekeeping rather than a card-bearing message. A window
    // shorter than that interval would never see one at all, leaving that
    // exclusion untested -- an accidentally-un-excluded `StatusRequest`
    // reaching the device during this window would then pass this assertion
    // for the wrong reason. 2.3 seconds gives one status tick a safe margin
    // to land inside the window.
    let ((), transcript) = tokio::join!(
        previews,
        support::card_messages_during(&mut socket, Duration::from_millis(2_300))
    );
    assert_eq!(transcript, Vec::<&'static str>::new());
}
