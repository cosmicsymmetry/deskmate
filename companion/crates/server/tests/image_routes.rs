//! HTTP contract for picture ingest and image-source lifecycle routes.

use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use server::{ServerState, app};

const ADMIN_TOKEN: &str = "in-memory-admin-token";
const ONE_MEBIBYTE: usize = 1024 * 1024;

struct TestServer {
    base_url: String,
}

/// A loopback HTTP harness for producer and admin image routes.
async fn spawn() -> TestServer {
    let state = ServerState::in_memory();
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
    let response = client
        .post(format!("{}/v1/images", server.base_url))
        .bearer_auth(ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "name": name }).to_string())
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
