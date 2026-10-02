//! A device tap must stay inside the account that owns the device, even when
//! another account has a face with the same source id.

mod support;

use std::path::{Path, PathBuf};

use futures_util::SinkExt as _;
use protocol::{DeviceEvent, EventAction, EventKind, Message};
use server::{FaceCommand, ServerState, app, set_faces};
use tokio_tungstenite::tungstenite::Message as WsMessage;

const SHARED_SOURCE_ID: &str = "image-aaaaaaaaaaaaaaaaaaaaaaaa";

fn shell_word(value: &Path) -> String {
    format!("'{}'", value.display().to_string().replace('\'', "'\\''"))
}

fn fake_faces_failing_scheduled_renders(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let wrapper = root.join("fake-faces-fail-scheduled");
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake-faces.sh");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexport DESKMATE_FAKE_FACES_FAIL_SCHEDULED=1\nexec {} \"$@\"\n",
            shell_word(&fake)
        ),
    )
    .expect("write fake-faces wrapper");
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700))
        .expect("make fake-faces wrapper executable");
    wrapper
}

fn picture_config() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 11,
        "preferences": {
            "timezone": "UTC",
            "autostart": false,
            "paused": false,
            "orientation": "landscape"
        },
        "cards": [{
            "kind": "picture",
            "id": "headlines",
            "title": "Headlines",
            "source_id": SHARED_SOURCE_ID,
            "tap_action": { "kind": "none" },
            "refresh": { "kind": "manual" },
            "alert": { "kind": "none" },
            "dwell_seconds": null
        }],
        "image_sources": [{ "id": SHARED_SOURCE_ID, "name": "Headlines" }],
        "assets": [],
        "advance": { "kind": "manual" },
        "updater": { "channel": "stable", "checks": "notify" }
    })
}

fn write_account_layout(
    config_root: &Path,
    account: &support::TestAccount,
    token_digest: &str,
    device_id: Option<&str>,
    face_state: Option<&[u8]>,
) -> PathBuf {
    let root = config_root.join("accounts").join(&account.id.0);
    std::fs::create_dir_all(root.join("devices")).expect("create account layout");
    std::fs::write(
        root.join("image-sources.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "sources": [{
                "id": SHARED_SOURCE_ID,
                "name": "Headlines",
                "token_sha256": token_digest,
                "recent_push_times": []
            }]
        }))
        .unwrap(),
    )
    .expect("write image sources");
    std::fs::write(
        root.join("data-cards.json"),
        serde_json::to_vec_pretty(&serde_json::json!([{
            "source_id": SHARED_SOURCE_ID,
            "refresh_seconds": 900,
            "face": { "kind": "headlines", "list": "top" }
        }]))
        .unwrap(),
    )
    .expect("write data-card specs");
    if let Some(device_id) = device_id {
        std::fs::write(
            root.join("devices").join(format!("{device_id}.json")),
            serde_json::to_vec_pretty(&picture_config()).unwrap(),
        )
        .expect("write device config");
    }
    if let Some(face_state) = face_state {
        std::fs::write(root.join("face-state.json"), face_state).expect("write face state");
    }
    root
}

#[allow(clippy::result_large_err)]
async fn connect_device(
    server: &support::HttpTestServer,
    token: &str,
) -> Result<support::DeviceSocket, tokio_tungstenite::tungstenite::Error> {
    let host = server
        .base_url
        .strip_prefix("http://")
        .expect("test server uses HTTP");
    support::connect_device(host, token).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_tap_changes_only_its_own_accounts_face_state() {
    let parsed_config: app_core::AppConfig =
        serde_json::from_value(picture_config()).expect("picture config parses");
    parsed_config
        .compile(1)
        .expect("picture config compiles before the device links");

    let config = tempfile::tempdir().expect("config root");
    let state = ServerState::new(
        support::IN_MEMORY_ADMIN_TOKEN.into(),
        server::firmware::FirmwareCatalog::in_memory(),
        config.path().to_path_buf(),
    );
    set_faces(
        &state,
        FaceCommand::program(fake_faces_failing_scheduled_renders(config.path())),
    );

    // Create B first so a process-wide or first-match lookup cannot accidentally
    // make this test pass. A is the account whose device sends the tap.
    let account_b = support::owner_account(&state);
    let account_a = support::second_account(&state, "a@example.com");
    let device_a = support::mint_owned_device(&state, &account_a);
    let b_state = br#"{
  "version": 1,
  "sources": {
    "image-aaaaaaaaaaaaaaaaaaaaaaaa": { "page": 7 }
  }
}"#;
    let root_b = write_account_layout(
        config.path(),
        &account_b,
        &"b".repeat(64),
        None,
        Some(b_state),
    );
    let root_a = write_account_layout(
        config.path(),
        &account_a,
        &"a".repeat(64),
        Some(&device_a.device_id),
        None,
    );

    // Open both spaces after their fixtures exist. Their scheduled renders fail
    // deliberately, leaving B's sentinel state untouched and A with no state.
    state.account_space(&account_b.id);
    state.account_space(&account_a.id);

    let server = support::spawn_http(app(state.clone())).await;
    let mut socket = connect_device(&server, &device_a.token)
        .await
        .expect("A's device links");
    support::drive_until_config(&mut socket, "headlines").await;
    support::drive_until_scene(&mut socket).await;
    socket
        .send(WsMessage::Binary(
            protocol::encode_message(
                0,
                &Message::DeviceEvent(DeviceEvent {
                    sequence: 1,
                    kind: EventKind::Tap,
                    card_id: "headlines".into(),
                    action: EventAction::StartPause,
                    interrupt_token: None,
                }),
            )
            .expect("encode tap")
            .into(),
        ))
        .await
        .expect("send tap");

    let a_state_path = root_a.join("face-state.json");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(bytes) = std::fs::read(&a_state_path)
                && serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["sources"]
                    [SHARED_SOURCE_ID]["page"]
                    == 3
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("A's tapped face state was not stored");

    assert_eq!(
        std::fs::read(root_b.join("face-state.json")).unwrap(),
        b_state,
        "A's tap must not mutate B's state for the same source id"
    );
    assert!(
        !config.path().join("face-state.json").exists(),
        "face state must never be written at the instance root"
    );
    state.shutdown();
}
