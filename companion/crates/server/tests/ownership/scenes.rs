use super::{connect_device, spawn, support};

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
async fn invalid_scenes_are_typed_bad_requests() {
    let valid = serde_json::json!({
        "card_id": "clock-1",
        "revision": 17,
        "template": "digital_clock",
        "show_seconds": true,
        "local_now": "2026-08-25T14:37:42",
    });
    for (label, field, invalid, message) in [
        (
            "unknown template",
            "template",
            serde_json::json!("analogue_clock"),
            "template",
        ),
        (
            "zero revision",
            "revision",
            serde_json::json!(0),
            "scene revision",
        ),
        (
            "overlong card id",
            "card_id",
            serde_json::json!("clock-id-that-is-more-than-32-bytes"),
            "card id",
        ),
        (
            "malformed local instant",
            "local_now",
            serde_json::json!("2026-08-25 14:37:42"),
            "local_now",
        ),
    ] {
        let (host, identity, admin_token) = spawn().await;
        let mut payload = valid.clone();
        payload[field] = invalid;
        let response = reqwest::Client::new()
            .post(format!(
                "http://{host}/v1/devices/{}/scene",
                identity.device_id
            ))
            .bearer_auth(admin_token)
            .header("Content-Type", "application/json")
            .body(payload.to_string())
            .send()
            .await
            .expect("scene request");
        assert_eq!(response.status(), 400, "{label}");
        let error: serde_json::Value =
            serde_json::from_str(&response.text().await.expect("typed error body"))
                .expect("typed error JSON");
        assert_eq!(error["kind"], "invalid-scene", "{label}");
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|actual| actual.contains(message)),
            "{label}: {error}"
        );
    }
}
