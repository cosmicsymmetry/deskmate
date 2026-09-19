//! HTTP contract for picture ingest and image-source lifecycle routes.

use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use server::firmware::FirmwareCatalog;
use server::{ServerState, app};

const ADMIN_TOKEN: &str = "in-memory-admin-token";
const ONE_MEBIBYTE: usize = 1024 * 1024;

struct TestServer {
    base_url: String,
}

/// A loopback HTTP harness for producer and admin image routes.
async fn spawn() -> TestServer {
    let state = ServerState::in_memory();
    spawn_with(state).await
}

async fn spawn_with(state: ServerState) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    tokio::spawn(async move {
        axum::serve(listener, app(state))
            .await
            .expect("serve image routes");
    });
    TestServer {
        base_url: format!("http://127.0.0.1:{}", address.port()),
    }
}

#[derive(Debug, Deserialize)]
struct MintedSource {
    id: String,
    token: String,
}

async fn mint(client: &Client, server: &TestServer, name: &str) -> MintedSource {
    mint_with_face(client, server, name, None).await
}

async fn mint_with_face(
    client: &Client,
    server: &TestServer,
    name: &str,
    face_kind: Option<&str>,
) -> MintedSource {
    let response = client
        .post(format!("{}/v1/images", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "name": name, "face_kind": face_kind }).to_string())
        .send()
        .await
        .expect("mint image source");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("mint response body");
    serde_json::from_str(&body).expect("mint response JSON")
}

async fn push(
    client: &Client,
    server: &TestServer,
    path_token: &str,
    bearer_token: Option<&str>,
    content_type: &str,
    body: Vec<u8>,
) -> Response {
    let request = client
        .post(format!("{}/v1/images/{path_token}", server.base_url))
        .header("content-type", content_type)
        .body(body);
    let request = match bearer_token {
        Some(token) => request.bearer_auth(token),
        None => request,
    };
    request.send().await.expect("push image")
}

fn png_of(width: u32, height: u32) -> Vec<u8> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG header");
        writer
            .write_image_data(&vec![0x2a; (width * height * 3) as usize])
            .expect("PNG pixels");
    }
    output
}

fn exact_png() -> Vec<u8> {
    png_of(448, 368)
}

fn replace_file_with_directory(path: &std::path::Path) {
    std::fs::rename(path, path.with_extension("backup")).expect("preserve original file");
    std::fs::create_dir(path).expect("blocking directory");
}

#[tokio::test]
async fn an_exact_png_is_accepted() {
    let server = spawn().await;
    let client = Client::new();
    let source = mint(&client, &server, "Status panel").await;

    let response = push(
        &client,
        &server,
        &source.token,
        None,
        "image/png",
        exact_png(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.text().await.expect("accepted body").is_empty());
}

#[tokio::test]
async fn an_unknown_token_is_unauthorized() {
    let server = spawn().await;
    let client = Client::new();

    let response = push(
        &client,
        &server,
        &"00".repeat(32),
        None,
        "image/png",
        exact_png(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response.text().await.expect("unauthorized body");
    assert_eq!(body.lines().count(), 1, "producer errors stay on one line");
    for internal in [
        "ImageSourceError",
        "CanonicalFrame",
        "companion/",
        ".rs",
        "/Users/",
    ] {
        assert!(
            !body.contains(internal),
            "producer body exposed an internal marker {internal:?}: {body}"
        );
    }
    let body: serde_json::Value = serde_json::from_str(&body).expect("typed error JSON");
    assert_eq!(body["kind"], "unauthorized");
}

#[tokio::test]
async fn a_revoked_token_is_unauthorized() {
    let server = spawn().await;
    let client = Client::new();
    let source = mint(&client, &server, "Revoked panel").await;
    let revoked = client
        .delete(format!("{}/v1/images/{}", server.base_url, source.id))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("revoke image source");
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let response = push(
        &client,
        &server,
        &source.token,
        None,
        "image/png",
        exact_png(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn deleting_a_source_removes_its_face_preserves_others_and_revokes_its_token() {
    let root = tempfile::tempdir().expect("config root");
    let state = ServerState::new(
        ADMIN_TOKEN.to_owned(),
        FirmwareCatalog::in_memory(),
        root.path().to_path_buf(),
    );
    let server = spawn_with(state).await;
    let client = Client::new();
    let removed = mint_with_face(&client, &server, "Weather", Some("weather")).await;
    let retained = mint_with_face(&client, &server, "News", Some("rss")).await;

    let response = client
        .delete(format!("{}/v1/images/{}", server.base_url, removed.id))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("delete image source");

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let bytes = std::fs::read(root.path().join("data-cards.json")).expect("persisted face specs");
    let persisted: serde_json::Value =
        serde_json::from_slice(&bytes).expect("persisted face specs parse");
    let specs = persisted.as_array().expect("persisted face spec list");
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0]["source_id"].as_str(), Some(retained.id.as_str()));
    let rejected = push(
        &client,
        &server,
        &removed.token,
        None,
        "image/png",
        exact_png(),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn delete_source_persistence_failure_is_internal_and_leaves_the_face_untouched() {
    let root = tempfile::tempdir().expect("config root");
    let state = ServerState::new(
        ADMIN_TOKEN.to_owned(),
        FirmwareCatalog::in_memory(),
        root.path().to_path_buf(),
    );
    let server = spawn_with(state).await;
    let client = Client::new();
    let source = mint_with_face(&client, &server, "Weather", Some("weather")).await;
    let spec_path = root.path().join("data-cards.json");
    let before = std::fs::read(&spec_path).expect("face specs");
    replace_file_with_directory(&root.path().join("image-sources.json"));

    let response = client
        .delete(format!("{}/v1/images/{}", server.base_url, source.id))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("delete image source");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        json_body(response).await,
        serde_json::json!({ "kind": "internal" })
    );
    assert_eq!(std::fs::read(spec_path).expect("unchanged specs"), before);
    let listed = client
        .get(format!("{}/v1/images", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("list sources");
    assert!(
        json_body(listed)
            .await
            .as_array()
            .expect("source rows")
            .iter()
            .any(|row| row["id"] == source.id && row["face"]["kind"] == "weather")
    );
}

#[tokio::test]
async fn delete_face_persistence_failure_is_internal_after_the_token_is_revoked() {
    let root = tempfile::tempdir().expect("config root");
    let state = ServerState::new(
        ADMIN_TOKEN.to_owned(),
        FirmwareCatalog::in_memory(),
        root.path().to_path_buf(),
    );
    let server = spawn_with(state).await;
    let client = Client::new();
    let source = mint_with_face(&client, &server, "Weather", Some("weather")).await;
    let spec_path = root.path().join("data-cards.json");
    replace_file_with_directory(&spec_path);

    let response = client
        .delete(format!("{}/v1/images/{}", server.base_url, source.id))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("delete image source");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        json_body(response).await,
        serde_json::json!({ "kind": "internal" })
    );
    assert!(spec_path.is_dir(), "the failing face target was replaced");
    let rejected = push(
        &client,
        &server,
        &source.token,
        None,
        "image/png",
        exact_png(),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_token_is_accepted_as_a_bearer_header_too() {
    let server = spawn().await;
    let client = Client::new();
    let source = mint(&client, &server, "Header-auth panel").await;

    let response = push(
        &client,
        &server,
        "token-is-in-the-header",
        Some(&source.token),
        "image/png",
        exact_png(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_body_over_one_mebibyte_is_rejected() {
    let server = spawn().await;
    let client = Client::new();
    let source = mint(&client, &server, "Bounded panel").await;

    let response = push(
        &client,
        &server,
        &source.token,
        None,
        "image/png",
        vec![0; ONE_MEBIBYTE + 1],
    )
    .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn a_body_that_is_not_a_png_is_rejected() {
    let server = spawn().await;
    let client = Client::new();
    let source = mint(&client, &server, "PNG-only panel").await;

    // Deliberately SMALL. A one-mebibyte body would be rejected for its size as
    // well as its type, so a correct server answering 413 would fail this test --
    // and, because the server closes as soon as it decides while the client is
    // still writing a megabyte, the write itself failed with ConnectionReset
    // about one run in three. The size rule has its own test above.
    let response = push(
        &client,
        &server,
        &source.token,
        None,
        "application/octet-stream",
        vec![0; 32],
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn a_wrongly_sized_png_is_unprocessable() {
    let server = spawn().await;
    let client = Client::new();
    let source = mint(&client, &server, "Exact-size panel").await;

    let response = push(
        &client,
        &server,
        &source.token,
        None,
        "image/png",
        png_of(447, 368),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.text().await.expect("unprocessable body");
    assert!(body.contains("448x368"), "wrong-size guidance: {body}");
}

#[tokio::test]
async fn a_second_push_inside_five_seconds_is_rate_limited() {
    let server = spawn().await;
    let client = Client::new();
    let source = mint(&client, &server, "Rate-limited panel").await;
    let png = exact_png();

    let first = push(
        &client,
        &server,
        &source.token,
        None,
        "image/png",
        png.clone(),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = push(&client, &server, &source.token, None, "image/png", png).await;

    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn minting_returns_the_plaintext_exactly_once() {
    let server = spawn().await;
    let client = Client::new();
    let source = mint(&client, &server, "One-time token panel").await;
    assert_eq!(source.token.len(), 64);

    let list = client
        .get(format!("{}/v1/images", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("list image sources");
    assert_eq!(list.status(), StatusCode::OK);
    let body = list.text().await.expect("GET response body");
    assert!(!body.contains(&source.token));
    let listed: serde_json::Value = serde_json::from_str(&body).expect("source descriptor JSON");
    assert_eq!(listed[0]["id"], source.id);
    assert_eq!(listed[0]["name"], "One-time token panel");
    assert!(listed[0]["face"].is_null());

    let by_id = client
        .get(format!("{}/v1/images/{}", server.base_url, source.id))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("GET source route");
    assert_eq!(by_id.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        !by_id
            .text()
            .await
            .expect("GET source body")
            .contains(&source.token)
    );

    let response = client
        .delete(format!("{}/v1/images/{}", server.base_url, source.id))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .expect("delete image source");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(response.text().await.expect("delete body").is_empty());
}

#[tokio::test]
async fn minting_a_ninth_source_preserves_the_capacity_response() {
    let server = spawn().await;
    let client = Client::new();

    for index in 0..8 {
        mint(&client, &server, &format!("Source {index}")).await;
    }

    let response = client
        .post(format!("{}/v1/images", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "name": "One too many" }).to_string())
        .send()
        .await
        .expect("mint beyond capacity");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(response).await,
        serde_json::json!({
            "kind": "capacity",
            "message": "the image-source capacity has been reached",
        })
    );
}

async fn json_body(response: Response) -> serde_json::Value {
    let body = response.text().await.expect("response body");
    serde_json::from_str(&body).unwrap_or_else(|error| panic!("invalid JSON ({error}): {body}"))
}

#[tokio::test]
async fn the_producer_routes_need_no_admin_token_and_the_admin_routes_do() {
    let server = spawn().await;
    let client = Client::new();

    let unauthenticated_mint = client
        .post(format!("{}/v1/images", server.base_url))
        .header("content-type", "application/json")
        .body(r#"{"name":"Not authorized"}"#)
        .send()
        .await
        .expect("unauthenticated mint");
    assert_eq!(unauthenticated_mint.status(), StatusCode::UNAUTHORIZED);

    let source = mint(&client, &server, "Producer-auth panel").await;
    let unauthenticated_revoke = client
        .delete(format!("{}/v1/images/{}", server.base_url, source.id))
        .send()
        .await
        .expect("unauthenticated revoke");
    assert_eq!(unauthenticated_revoke.status(), StatusCode::UNAUTHORIZED);

    let unauthenticated_list = client
        .get(format!("{}/v1/images", server.base_url))
        .send()
        .await
        .expect("unauthenticated list");
    assert_eq!(unauthenticated_list.status(), StatusCode::UNAUTHORIZED);

    let producer_push = push(
        &client,
        &server,
        &source.token,
        None,
        "image/png",
        exact_png(),
    )
    .await;
    assert_eq!(producer_push.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_source_without_a_server_face_refuses_settings_updates() {
    let server = spawn().await;
    let client = Client::new();
    let source = mint(&client, &server, "External producer").await;

    let response = client
        .put(format!("{}/v1/images/{}/face", server.base_url, source.id))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(r#"{"fields":{"location":"Berlin"}}"#)
        .send()
        .await
        .expect("update external source");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: serde_json::Value =
        serde_json::from_str(&response.text().await.expect("typed error body"))
            .expect("typed error JSON");
    assert_eq!(body["kind"], "face-not-configurable");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("external producer"))
    );
}

async fn local_json_request(
    state: &ServerState,
    method: &str,
    path: &str,
    body: serde_json::Value,
) -> (axum::http::StatusCode, serde_json::Value) {
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt as _;

    let response = app(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {ADMIN_TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, body)
}

#[tokio::test]
async fn server_owned_faces_accept_and_persist_partial_settings() {
    let root = tempfile::tempdir().unwrap();
    let state = ServerState::new(
        ADMIN_TOKEN.into(),
        FirmwareCatalog::in_memory(),
        root.path().to_path_buf(),
    );
    let (status, defaults) =
        local_json_request(&state, "GET", "/v1/faces", serde_json::Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    for (kind, key, value) in [
        ("weather", "units", "imperial"),
        ("rss", "title", "News"),
        ("token", "currency", "eur"),
    ] {
        let (status, minted) = local_json_request(
            &state,
            "POST",
            "/v1/images",
            serde_json::json!({"name":kind, "face_kind":kind}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(minted.as_object().unwrap().len(), 2);
        assert_eq!(minted["token"].as_str().unwrap().len(), 64);
        let id = minted["id"].as_str().unwrap();
        let path = format!("/v1/images/{id}/face");
        let mut expected = defaults
            .as_array()
            .unwrap()
            .iter()
            .find(|face| face["kind"] == kind)
            .unwrap()
            .clone();
        assert_eq!(
            local_json_request(&state, "PUT", &path, serde_json::json!({"fields":{}})).await,
            (StatusCode::OK, expected.clone())
        );
        expected["fields"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|field| field["key"] == key)
            .unwrap()["value"] = value.into();
        assert_eq!(
            local_json_request(
                &state,
                "PUT",
                &path,
                serde_json::json!({"fields":{key:value}})
            )
            .await,
            (StatusCode::OK, expected.clone())
        );
        let (status, listed) =
            local_json_request(&state, "GET", "/v1/images", serde_json::Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            listed
                .as_array()
                .unwrap()
                .iter()
                .find(|source| source["id"] == id)
                .unwrap()["face"],
            expected
        );
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.path().join("data-cards.json")).unwrap())
                .unwrap();
        let spec = persisted
            .as_array()
            .unwrap()
            .iter()
            .find(|spec| spec["source_id"] == id)
            .unwrap();
        assert_eq!(spec["face"][key], value);
        assert_eq!(spec["refresh_seconds"], 900);
    }
    state.shutdown();
}

#[tokio::test]
async fn server_owned_face_updates_keep_existing_rejection_contracts() {
    let state = ServerState::in_memory();
    let (_, minted) = local_json_request(
        &state,
        "POST",
        "/v1/images",
        serde_json::json!({"name":"Weather", "face_kind":"weather"}),
    )
    .await;
    let path = format!("/v1/images/{}/face", minted["id"].as_str().unwrap());
    for (fields, message) in [
        (
            serde_json::json!({"location":" "}),
            "location: must not be empty",
        ),
        (
            serde_json::json!({"units":"kelvin"}),
            "units: has an unsupported value",
        ),
        (
            serde_json::json!({"unknown":"value"}),
            "unknown face field \"unknown\"",
        ),
    ] {
        assert_eq!(
            local_json_request(&state, "PUT", &path, serde_json::json!({"fields":fields})).await,
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                serde_json::json!({"kind":"invalid-face-fields", "message":message})
            )
        );
    }
    assert_eq!(
        local_json_request(
            &state,
            "PUT",
            "/v1/images/missing/face",
            serde_json::json!({"fields":{}})
        )
        .await,
        (StatusCode::NOT_FOUND, serde_json::Value::Null)
    );
    let (status, _) = local_json_request(
        &state,
        "POST",
        "/v1/images",
        serde_json::json!({"name":"Unknown", "face_kind":"unknown"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, listed) =
        local_json_request(&state, "GET", "/v1/images", serde_json::Value::Null).await;
    assert_eq!(
        listed.as_array().unwrap().len(),
        1,
        "failed attachments roll back their source"
    );
    state.shutdown();
}
