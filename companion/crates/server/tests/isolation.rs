//! Account B must not see account A's resources through any route that takes an id.

mod support;

use reqwest::StatusCode;
use server::{ServerState, app};
use support::*;

struct World {
    server: HttpTestServer,
    b: TestAccount,
    dev_a: String,
    source_a: String,
}

async fn world() -> World {
    let state = with_fake_faces(ServerState::in_memory());
    let a = owner_account(&state);
    let b = second_account(&state, "b@example.com");
    let dev_a = mint_owned_device(&state, &a).device_id;
    let server = spawn_http(app(state)).await;
    let source = cookie_request(
        &server,
        &a,
        "POST",
        "/v1/images",
        Some(serde_json::json!({"name": "Weather"})),
    )
    .await;
    assert_eq!(source.status(), StatusCode::OK);
    let source_a = json_body(source).await["id"].as_str().unwrap().to_owned();
    World {
        server,
        b,
        dev_a,
        source_a,
    }
}

#[tokio::test]
async fn account_b_gets_404_for_every_route_naming_account_a_resources() {
    let world = world().await;
    let device = &world.dev_a;
    let source = &world.source_a;
    let config = serde_json::json!({
        "json": serde_json::to_string(&app_core::AppConfig::default()).unwrap(),
    });
    let cases = vec![
        ("GET", format!("/v1/app/{device}/snapshot"), None),
        ("GET", format!("/v1/app/{device}/events"), None),
        (
            "PUT",
            format!("/v1/app/{device}/config"),
            Some(config.clone()),
        ),
        (
            "POST",
            format!("/v1/app/{device}/config/validate"),
            Some(config),
        ),
        (
            "POST",
            format!("/v1/app/{device}/preview"),
            Some(serde_json::json!({"card_id": "clock"})),
        ),
        (
            "POST",
            format!("/v1/app/{device}/pomodoro"),
            Some(serde_json::json!({"card_id": "p", "action": "start"})),
        ),
        ("DELETE", format!("/v1/app/devices/{device}"), None),
        ("DELETE", format!("/v1/images/{source}"), None),
        (
            "PUT",
            format!("/v1/images/{source}/face"),
            Some(serde_json::json!({"fields": {}})),
        ),
    ];

    for (method, path, body) in cases {
        let status = cookie_request(&world.server, &world.b, method, &path, body)
            .await
            .status();
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path} leaked");
    }
}

#[tokio::test]
async fn lists_are_scoped_to_the_signed_in_account() {
    let world = world().await;
    let devices =
        json_body(cookie_request(&world.server, &world.b, "GET", "/v1/app/devices", None).await)
            .await;
    assert_eq!(devices, serde_json::json!([]));

    let sources =
        json_body(cookie_request(&world.server, &world.b, "GET", "/v1/images", None).await).await;
    assert!(
        sources
            .as_array()
            .unwrap()
            .iter()
            .all(|source| source["id"] != world.source_a)
    );
}

#[tokio::test]
async fn owner_only_surfaces_are_404_for_other_accounts() {
    let world = world().await;
    for (method, path, body) in [
        ("GET", "/v1/manage", None),
        ("POST", "/v1/integrations/google", None),
        (
            "PUT",
            "/v1/app/instance/signups",
            Some(serde_json::json!({"open": true})),
        ),
    ] {
        assert_eq!(
            cookie_request(&world.server, &world.b, method, path, body)
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{method} {path}"
        );
    }
}
