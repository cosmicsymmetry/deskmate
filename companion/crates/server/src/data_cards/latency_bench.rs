//! Opt-in, real Bun + RSS + HTTP/WebSocket bench. No board or external fetch.
//! From companion: `cargo test -p server tap_latency_bench -- --ignored --nocapture`
//! Set `RUST_LOG=server::tap_latency=info` to retain segment timings as well.

use super::*;
use crate::image_ingest::{CanonicalFrame, canonical_frame_from_png};
use futures_util::{SinkExt as _, StreamExt as _};
use protocol::{Ack, DeviceEvent, EventAction, EventKind, Message};
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

struct Peer {
    taps: mpsc::UnboundedSender<tokio::sync::oneshot::Sender<Instant>>,
    scenes: mpsc::UnboundedReceiver<([u8; 32], Instant, usize, usize)>,
    task: tokio::task::JoinHandle<()>,
}

impl Peer {
    #[allow(clippy::too_many_lines)] // Complete simulated protocol peer.
    async fn start(state: &ServerState, account: &crate::identity::AccountId) -> Self {
        let identity = state.registry().mint().unwrap();
        state
            .identity()
            .assign_device(
                &identity.device_id,
                account,
                crate::identity::DeviceState::Active,
                chrono::Utc::now(),
            )
            .unwrap();
        let space = state.account_space(account);
        let config = space.root.join("bench-config.json");
        std::fs::rename(
            config,
            space
                .root
                .join("devices")
                .join(format!("{}.json", identity.device_id)),
        )
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = listener.local_addr().unwrap();
        let app = crate::app(state.clone());
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let request = http::Request::builder()
            .uri(format!("ws://{host}/v1/device/link"))
            .header("Authorization", format!("Bearer {}", identity.token))
            .header("Host", host.to_string())
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tokio_tungstenite::tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .unwrap();
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let (taps, mut requests) =
            mpsc::unbounded_channel::<tokio::sync::oneshot::Sender<Instant>>();
        let (scenes, scene_receiver) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut sequence = 0;
            let mut resident = HashSet::new();
            let mut assets = 0;
            let mut begins = 0;
            loop {
                tokio::select! {
                    request = requests.recv() => {
                        let Some(reply) = request else { break };
                        sequence += 1;
                        let tap = Message::DeviceEvent(DeviceEvent {
                            sequence, kind: EventKind::Tap, card_id: "bench".into(),
                            action: EventAction::StartPause, interrupt_token: None,
                        });
                        let bytes = protocol::encode_message(0, &tap).unwrap();
                        let started = Instant::now();
                        socket.send(WsMessage::Binary(bytes.into())).await.unwrap();
                        let _ = reply.send(started);
                    }
                    incoming = socket.next() => {
                        let Some(Ok(incoming)) = incoming else { break };
                        let WsMessage::Binary(bytes) = incoming else { continue };
                        let received = Instant::now();
                        let frame = protocol::decode_wire_frame(&bytes).unwrap();
                        let message = protocol::decode_message(&frame).unwrap();
                        let mut ack = Ack { acknowledged_type: frame.message_type, revision: None, already_present: None };
                        let response = match &message {
                            Message::StatusRequest => {
                                let mut status = protocol::test_support::sample_status_response();
                                status.capabilities = protocol::CURRENT_CAPABILITIES;
                                status.tier = protocol::Tier::Networked;
                                status.firmware_version = "1.0.0".into();
                                Message::StatusResponse(status)
                            }
                            Message::Heartbeat => Message::HeartbeatAck(protocol::HeartbeatAck { uptime_ms: 1234 }),
                            other => {
                                match other {
                                    Message::ApplyConfig(config) => ack.revision = Some(config.revision),
                                    Message::PushTimer(push) => ack.revision = Some(push.revision),
                                    Message::PushScene(push) => {
                                        ack.revision = Some(push.revision);
                                        for node in &push.scene.nodes {
                                            if let protocol::SceneNode::Image(image) = node {
                                                assert!(resident.contains(&image.digest), "scene references an absent asset");
                                                scenes.send((image.digest, received, assets, begins)).unwrap();
                                                assets = 0;
                                                begins = 0;
                                            }
                                        }
                                    }
                                    Message::AssetBegin(begin) => {
                                        begins += 1;
                                        ack.already_present = Some(resident.contains(&begin.digest));
                                    }
                                    Message::AssetChunk(_) => { assets += 1; }
                                    Message::AssetCommit(commit) => { resident.insert(commit.digest); }
                                    Message::AssetRelease(release) => resident.retain(|digest| release.digests.contains(digest)),
                                    _ => {}
                                }
                                Message::Ack(ack)
                            }
                        };
                        socket.send(WsMessage::Binary(protocol::encode_message(frame.request_id, &response).unwrap().into())).await.unwrap();
                    }
                }
            }
        });
        Self {
            taps,
            scenes: scene_receiver,
            task,
        }
    }

    async fn until(&mut self, digest: [u8; 32]) -> (Instant, usize, usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let (seen, at, assets, begins) = self.scenes.recv().await.expect("device stopped");
                if seen == digest {
                    return (at, assets, begins);
                }
            }
        })
        .await
        .expect("the expected scene did not arrive")
    }
}

fn rss_state(page: usize) -> serde_json::Value {
    let stories: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../faces/test/hn-front-page.captured.json"
    ))
    .unwrap();
    let entries: Vec<_> = stories
        .into_iter()
        .take(16)
        .map(|story| serde_json::json!({"title": story["title"], "age": "1h"}))
        .collect();
    assert_eq!(entries.len(), 16);
    serde_json::json!({"feedTitle": "Bench", "entries": entries, "page": page, "tappedAt": null})
}

fn draw(command: &FaceCommand, space: &AccountSpace, view: &str) -> CanonicalFrame {
    let rendered = faces_package::render(
        command,
        faces_package::RenderRequest {
            kind: "rss",
            settings: &BTreeMap::new(),
            state: Some(&rss_state(0)),
            taps: 0,
            view: Some(view),
        },
        &space.root,
    )
    .unwrap();
    canonical_frame_from_png(&rendered.png).unwrap()
}

fn write_config(space: &AccountSpace, source_id: &str) {
    let config = serde_json::json!({
        "schema_version": 10,
        "preferences": {"timezone":"UTC","autostart":false,"paused":false,"orientation":"landscape"},
        "cards":[{"kind":"picture","id":"bench","title":"Bench","source_id":source_id,
            "tap_action":{"kind":"none"},"refresh":{"kind":"manual"},"alert":{"kind":"none"},"dwell_seconds":null}],
        "image_sources":[{"id":source_id,"name":"Bench"}],"assets":[],
        "advance":{"kind":"manual"},"updater":{"channel":"stable","checks":"disabled"}
    });
    std::fs::create_dir_all(space.root.join("devices")).unwrap();
    std::fs::write(space.root.join("bench-config.json"), config.to_string()).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_socket_tap_wakes_an_idle_runtime_before_its_next_tick() {
    let state = ServerState::in_memory_with_runtime_options(app_core::RuntimeOptions {
        loop_maximum_wait: Duration::from_secs(3),
        pomodoro_interval: Duration::from_secs(30),
        status_interval: Duration::from_secs(30),
        time_sync_interval: Duration::from_secs(30),
        ..app_core::RuntimeOptions::default()
    });
    set_faces(&state, faces_package::fake());
    let account = state
        .identity()
        .create_account("wake@example.com", true, true, chrono::Utc::now())
        .unwrap();
    let space = state.account_space(&account.id);
    let source = space.image_sources.mint("Wake").unwrap();
    let frame = |name| {
        canonical_frame_from_png(
            &std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/support")
                    .join(name),
            )
            .unwrap(),
        )
        .unwrap()
    };
    let base = frame("fake-face.png");
    let next = frame("fake-face-alt.png");
    space
        .image_sources
        .accept_server_rendered(&source.id, base.clone(), chrono::Utc::now())
        .unwrap();
    space
        .image_sources
        .accept_staged_view(&source.id, "page-1", next.clone(), chrono::Utc::now())
        .unwrap();
    space.data_cards.lock().unwrap().specs.push(DataCardSpec {
        source_id: source.id.clone(),
        refresh_seconds: 900,
        face: FaceSpec {
            kind: "headlines".into(),
            settings: BTreeMap::new(),
        },
    });
    write_config(&space, &source.id);
    let mut peer = Peer::start(&state, &account.id).await;
    peer.until(base.digest).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (reply, started) = tokio::sync::oneshot::channel();
    peer.taps.send(reply).unwrap();
    started.await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(1), peer.until(next.digest)).await;
    state.shutdown();
    peer.task.abort();
    assert!(result.is_ok(), "tap waited for the three-second idle poll");
    let (_, chunks, begins) = result.unwrap();
    assert_eq!(
        (begins, chunks),
        (0, 0),
        "resident selection must push only the scene"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "latency bench requires Bun and installed companion/faces dependencies"]
#[allow(clippy::too_many_lines)] // Setup and isolated sampling are intentionally together.
async fn tap_latency_bench() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_ansi(false)
        .try_init();
    let bun = std::process::Command::new("which")
        .arg("bun")
        .output()
        .unwrap();
    assert!(bun.status.success(), "install Bun before running the bench");
    let command = FaceCommand::bun(
        PathBuf::from(String::from_utf8(bun.stdout).unwrap().trim()),
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../faces"),
    );
    for staged in [true, false] {
        let state =
            ServerState::in_memory_with_runtime_options(app_core::RuntimeOptions::default());
        set_faces(&state, command.clone());
        let account = state
            .identity()
            .create_account("bench@example.com", true, true, chrono::Utc::now())
            .unwrap();
        let space = state.account_space(&account.id);
        let source = space.image_sources.mint("Bench").unwrap();
        let initial_page = if staged { 0 } else { 2 };
        let next_view = if staged { "page-2" } else { "page-4" };
        let base = draw(&command, &space, if staged { "" } else { "page-3" });
        let next = draw(&command, &space, next_view);
        space
            .image_sources
            .accept_server_rendered(&source.id, base.clone(), chrono::Utc::now())
            .unwrap();
        if staged {
            space
                .image_sources
                .accept_staged_view(&source.id, next_view, next.clone(), chrono::Utc::now())
                .unwrap();
        }
        let face_state = {
            let mut cards = space.data_cards.lock().unwrap();
            cards.specs = vec![DataCardSpec {
                source_id: source.id.clone(),
                refresh_seconds: 900,
                face: FaceSpec {
                    kind: "rss".into(),
                    settings: BTreeMap::new(),
                },
            }];
            let taps = Arc::new(TapSignal::default());
            let task = worker::spawn_refresher(
                &tokio::runtime::Handle::current(),
                state.clone(),
                space.clone(),
                command.clone(),
                cards.face_state.clone(),
                taps.clone(),
                cards.specs[0].clone(),
            );
            cards
                .tasks
                .insert(source.id.clone(), Refresher::new(task, taps));
            cards.face_state.clone()
        };
        // The initial scheduled fetch has no URL and fails locally. Tap renders
        // use cached headlines; no public service or fetch mock is timed.
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if space
                    .data_cards
                    .lock()
                    .unwrap()
                    .outcomes
                    .contains_key(&source.id)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        face_state.put(&source.id, Some(rss_state(initial_page)));
        write_config(&space, &source.id);
        let mut peer = Peer::start(&state, &account.id).await;
        peer.until(base.digest).await;
        let mut samples = Vec::new();
        for trial in 0..35 {
            // Let post-render staging finish before the next isolated sample.
            // Vary the idle phase so a 25ms polling loop is not phase-locked.
            tokio::time::sleep(Duration::from_millis(30 + trial * 7 % 29)).await;
            if trial > 0 {
                space
                    .image_sources
                    .accept_server_rendered(&source.id, base.clone(), chrono::Utc::now())
                    .unwrap();
                space.image_sources.select_view(&source.id, "").unwrap();
                face_state.put(&source.id, Some(rss_state(initial_page)));
                state.notify_image_source_changed(
                    &account.id,
                    source.id.clone(),
                    base.digest,
                    ImageNotificationOrigin::ServerRenderedRefresh,
                );
                peer.until(base.digest).await;
                tokio::time::sleep(Duration::from_millis(3 + trial * 7 % 29)).await;
            }
            let prior_outcome = space.data_cards.lock().unwrap().outcomes[&source.id].1;
            let (reply, started) = tokio::sync::oneshot::channel();
            peer.taps.send(reply).unwrap();
            let started = started.await.unwrap();
            let (received, assets, _) = peer.until(next.digest).await;
            if staged {
                assert_eq!(assets, 0, "staged tap transferred an asset");
            }
            let elapsed = received.duration_since(started).as_secs_f64() * 1000.0;
            println!(
                "BENCH sample mode={} trial={trial} ms={elapsed:.3}",
                if staged { "staged" } else { "fallback" }
            );
            samples.push(elapsed);
            if !staged {
                // `record_outcome` runs AFTER staging. Wait for that exact
                // completion, rather than racing the next tap against a sleep.
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if space.data_cards.lock().unwrap().outcomes[&source.id].1 > prior_outcome {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .unwrap();
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "BENCH summary mode={} n={} median_ms={:.3} p95_ms={:.3}",
            if staged { "staged" } else { "fallback" },
            samples.len(),
            samples[samples.len() / 2],
            samples[(samples.len() * 95).div_ceil(100) - 1]
        );
        state.shutdown();
        peer.task.abort();
    }
}
