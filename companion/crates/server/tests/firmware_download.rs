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
//!
//! Every negative case below *plants a real file at the location the
//! attack targets* and asserts the response is neither 200 nor that
//! file's planted bytes. A traversal test that only asserts 404 against a
//! target that doesn't exist proves nothing: a completely guard-free
//! implementation returns 404 there too (`File::open` on a nonexistent
//! path fails regardless of whether anything upstream tried to stop it),
//! so it pins *unreachability*, not the guard. Planting a real,
//! attacker-reachable-if-unguarded file is what makes a guard's removal
//! observable as a test failure instead of a silent no-op.

use std::path::Path;

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

/// A name derived from the catalog's own (randomly generated, per-test)
/// temp directory name, so files planted outside it by different tests
/// running concurrently can never collide.
fn unique_name(state: &ServerState, suffix: &str) -> String {
    let dir_name = state
        .firmware()
        .directory()
        .file_name()
        .expect("a tempdir has a file name")
        .to_string_lossy();
    format!("{dir_name}-{suffix}")
}

/// Writes `bytes` to `path`, creating parent directories as needed.
async fn plant(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.unwrap();
    }
    tokio::fs::write(path, bytes).await.unwrap();
}

/// Asserts `response` is not the guard-free outcome: not `200`, and its
/// body (if any) is not `planted`. Removes `cleanup_path` first (best
/// effort) so a failing assertion still leaves no stray file behind.
async fn assert_guard_held(response: reqwest::Response, planted: &[u8], cleanup_path: &Path) {
    let status = response.status();
    let body = response.bytes().await.ok();
    let _ = tokio::fs::remove_file(cleanup_path).await;
    assert_ne!(status, 200, "the guard-free outcome was reachable");
    if let Some(body) = body {
        assert_ne!(
            body.as_ref(),
            planted,
            "the planted bytes were served despite a non-200 status"
        );
    }
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
    let (host, state) = spawn().await;
    let planted = b"PLANTED-VIA-PARTIALLY-ENCODED-DOT-DOT";
    let stem = unique_name(&state, "partial-traversal-target");
    // One level above the catalog directory -- exactly where
    // `directory.join("../<stem>.bin")` would land if `image_path`'s
    // checks were absent.
    let target = state
        .firmware()
        .directory()
        .parent()
        .expect("a tempdir has a parent")
        .join(format!("{stem}.bin"));
    plant(&target, planted).await;

    // "..%2f" -- literal dot-dot, encoded slash. `url` only collapses
    // *literal* dot-segments at parse time, so this survives to the wire
    // undecoded and axum decodes it once, server-side, to "../<stem>.bin".
    let response = get_raw(&host, &format!("/v1/firmware/..%2f{stem}.bin")).await;

    assert_guard_held(response, planted, &target).await;
}

#[tokio::test]
async fn download_rejects_fully_encoded_dot_dot() {
    let (host, state) = spawn().await;
    let planted = b"PLANTED-VIA-FULLY-ENCODED-DOT-DOT";
    let stem = unique_name(&state, "full-traversal-target");
    let target = state
        .firmware()
        .directory()
        .parent()
        .expect("a tempdir has a parent")
        .join(format!("{stem}.bin"));
    plant(&target, planted).await;

    // "%2e%2e%2f" -- the classic fully-encoded traversal payload, decoding
    // to the same "../<stem>.bin".
    let response = get_raw(&host, &format!("/v1/firmware/%2e%2e%2f{stem}.bin")).await;

    assert_guard_held(response, planted, &target).await;
}

#[tokio::test]
async fn download_rejects_an_embedded_null_byte() {
    let (host, _state) = spawn().await;
    // Unlike the other cases here, this one cannot be made "reachable if
    // unguarded" by planting a file: a path containing a NUL byte cannot
    // be opened *or created* through `std::fs` on any platform this server
    // targets -- the NUL rejection happens converting the path to a C
    // string, underneath any guard this crate could add or remove. This
    // test therefore pins that OS-level behavior, not `image_path`'s
    // allowlist; the review that asked for this suite noted the same.
    let response = get_raw(&host, "/v1/firmware/1.0.0%00.bin").await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn download_rejects_an_encoded_slash_inside_the_name() {
    let (host, state) = spawn().await;
    let planted = b"PLANTED-UNDER-A-SUBDIRECTORY";
    // "%2f" alone -- an encoded slash smuggled into the middle of a
    // filename. If `image_path`'s allowlist were absent, `directory.join`
    // would treat "foo/bar.bin" as two path components, landing here.
    let target = state.firmware().directory().join("foo").join("bar.bin");
    plant(&target, planted).await;

    let response = get_raw(&host, "/v1/firmware/foo%2fbar.bin").await;

    assert_guard_held(response, planted, &target).await;
}

#[tokio::test]
async fn download_rejects_an_over_long_version() {
    let (host, state) = spawn().await;
    let planted = b"PLANTED-AT-AN-OVER-LONG-NAME";
    let long_name = "a".repeat(40);
    // Every character here is allowlist-safe and there is no traversal --
    // only the 32-character length cap stands between this and a 200. The
    // file is planted directly inside the catalog directory, not outside
    // it, so this test isolates that one check from the others.
    let target = state
        .firmware()
        .directory()
        .join(format!("{long_name}.bin"));
    plant(&target, planted).await;

    let response = get_raw(&host, &format!("/v1/firmware/{long_name}.bin")).await;

    assert_guard_held(response, planted, &target).await;
}

#[tokio::test]
async fn download_rejects_a_non_ascii_version() {
    let (host, state) = spawn().await;
    let planted = b"PLANTED-AT-A-NON-ASCII-NAME";
    // "é" as a literal filename component (planted with real filesystem
    // APIs, so this is exactly what the decoded request must also
    // resolve to for the two to collide).
    let filename = "1.0.0\u{e9}.bin";
    let target = state.firmware().directory().join(filename);
    plant(&target, planted).await;

    // Percent-encoded UTF-8 for the same "é" (%C3%A9) -- valid UTF-8 once
    // decoded, so it reaches the handler rather than being rejected by the
    // extractor, and must be rejected by the ASCII-only allowlist instead.
    let response = get_raw(&host, "/v1/firmware/1.0.0%C3%A9.bin").await;

    assert_guard_held(response, planted, &target).await;
}

#[tokio::test]
async fn download_rejects_a_name_without_the_bin_suffix() {
    let (host, _state) = spawn().await;
    let response = get_raw(&host, "/v1/firmware/1.0.0").await;
    assert_eq!(response.status(), 404);
}
