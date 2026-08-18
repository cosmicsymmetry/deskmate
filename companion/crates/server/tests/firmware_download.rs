//! End-to-end coverage for `GET /v1/firmware/{version}.bin`, exercised
//! through the real HTTP + percent-decoding stack rather than by calling
//! `FirmwareCatalog`'s private helpers directly -- percent-decoding is
//! exactly where path-traversal attacks live, and a unit test that hands a
//! pre-decoded string to the sanitizer never touches that layer.
//!
//! This route sits behind a public Cloudflare tunnel from Task 8 onward and
//! is deliberately unauthenticated (images are byte-identical across
//! devices and hold no per-device secret -- integrity comes from the
//! tunnel's TLS), so every case here reflects what a fully anonymous
//! internet client can throw at it.

use server::{ServerState, app};

async fn spawn() -> (String, ServerState) {
    let state = ServerState::in_memory();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let serve_state = state.clone();
    tokio::spawn(async move {
        axum::serve(listener, app(serve_state)).await.unwrap();
    });
    (format!("127.0.0.1:{}", address.port()), state)
}

/// Sends a GET whose request-target path is exactly `raw_path` (already
/// percent-encoded where the test wants it, and not re-normalized by
/// `reqwest`/`url`'s own dot-segment resolution the way a literal `..`
/// segment would be).
async fn get_raw(host: &str, raw_path: &str) -> reqwest::Response {
    let url = format!("http://{host}{raw_path}");
    reqwest::Client::new().get(url).send().await.unwrap()
}

#[tokio::test]
async fn download_serves_the_current_image() {
    let (host, state) = spawn().await;
    let image_path = state.firmware().directory().join("1.0.0.bin");
    tokio::fs::write(&image_path, b"firmware-image-bytes")
        .await
        .unwrap();

    let response = get_raw(&host, "/v1/firmware/1.0.0.bin").await;

    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/octet-stream"
    );
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        b"firmware-image-bytes"
    );
}

#[tokio::test]
async fn download_rejects_partially_encoded_dot_dot() {
    let (host, _state) = spawn().await;
    // "..%2f" -- literal dot-dot, encoded slash. `url` only collapses
    // *literal* dot-segments at parse time, so this survives to the wire
    // undecoded and axum decodes it once, server-side.
    let response = get_raw(&host, "/v1/firmware/..%2fCargo.toml.bin").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn download_rejects_fully_encoded_dot_dot() {
    let (host, _state) = spawn().await;
    // "%2e%2e%2f" -- the classic fully-encoded traversal payload.
    let response = get_raw(&host, "/v1/firmware/%2e%2e%2f%2e%2e%2fetc%2fpasswd.bin").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn download_rejects_an_embedded_null_byte() {
    let (host, _state) = spawn().await;
    let response = get_raw(&host, "/v1/firmware/1.0.0%00.bin").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn download_rejects_an_encoded_slash_inside_the_name() {
    let (host, _state) = spawn().await;
    // "%2f" alone -- an encoded slash smuggled into the middle of a
    // filename, not spelling a traversal but still not a bare version.
    let response = get_raw(&host, "/v1/firmware/foo%2fbar.bin").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn download_rejects_an_over_long_version() {
    let (host, _state) = spawn().await;
    let long_name = "a".repeat(40);
    let response = get_raw(&host, &format!("/v1/firmware/{long_name}.bin")).await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn download_rejects_a_non_ascii_version() {
    let (host, _state) = spawn().await;
    // "é" percent-encoded as UTF-8 (%C3%A9) -- valid UTF-8 once decoded, so
    // it reaches the handler rather than being rejected by the extractor,
    // and must be rejected by the character allowlist instead.
    let response = get_raw(&host, "/v1/firmware/1.0.0%C3%A9.bin").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn download_rejects_a_name_without_the_bin_suffix() {
    let (host, _state) = spawn().await;
    let response = get_raw(&host, "/v1/firmware/1.0.0").await;
    assert_eq!(response.status(), 404);
}
