use super::*;

#[test]
fn reboot_restores_residency_before_scenes_and_staged_updates() {
    for picture_is_active in [true, false] {
        let control = MockDeviceControl::default();
        control.state.lock().unwrap().enforce_residency = true;
        let host = FakeImageSourceHostControl::default();
        let digest = [0x41; 32];
        host.stage_picture_frame("camera", digest, PICTURE_BLOB, false);
        let runtime = start_picture_runtime(
            picture_config(picture_is_active),
            &control,
            Some(Box::new(host.host())),
        );
        wait_for(Duration::from_secs(2), || {
            control.state.lock().unwrap().resident.contains(&digest)
        });
        control.force_disconnect(true);
        wait_for(Duration::from_secs(2), || {
            control.state.lock().unwrap().connection_count >= 2
        });
        if picture_is_active {
            // The older automatic scene path must recover even without another tap.
            wait_for(Duration::from_secs(2), || {
                control.state.lock().unwrap().resident.contains(&digest)
            });
        }
        let update = runtime.image_source_updated("camera", digest);
        runtime.shutdown().unwrap();
        let state = control.state.lock().unwrap();
        assert!(update.is_ok());
        assert_eq!(state.missing_scenes, 0);
        assert!(
            state.resident.contains(&digest),
            "staged update must restore the asset after reboot"
        );
    }
}

#[test]
fn fifteen_desired_frames_allow_staged_and_resting_replacements() {
    for count in [14u8, 15] {
        let control = MockDeviceControl::default();
        control.state.lock().unwrap().enforce_residency = true;
        let host = FakeImageSourceHostControl::default();
        let mk = |n: u8| DesiredAsset {
            digest: [n; 32],
            kind: protocol::AssetKind::Image,
            bytes: Arc::from(PICTURE_BLOB),
        };
        let mut desired: Vec<_> = (1..count).map(mk).collect();
        host.set_desired_assets(desired.clone());
        host.stage_picture_frame("camera", [count; 32], PICTURE_BLOB, false);
        let mut st = status(1);
        st.capabilities = protocol::CURRENT_CAPABILITIES;
        st.volatile_assets = Some(protocol::VolatileAssetStats {
            committed_count: u32::from(count),
            slot_capacity: 16,
            used_bytes: 0,
            psram_free_bytes: 8_000_000,
            psram_low_water_bytes: 8_000_000,
        });
        control.set_status(st);
        let runtime =
            start_picture_runtime(picture_config(true), &control, Some(Box::new(host.host())));
        wait_for(Duration::from_secs(2), || {
            control.state.lock().unwrap().resident.len() == usize::from(count)
        });
        desired[0] = mk(60);
        host.set_desired_assets(desired);
        host.stage_picture_frame("camera", [61; 32], PICTURE_BLOB, false);
        let first = runtime.image_source_updated("camera", [61; 32]);
        let held = control.state.lock().unwrap().resident.len();
        host.stage_picture_frame("camera", [60; 32], PICTURE_BLOB, false);
        let next = runtime.image_source_updated("camera", [60; 32]);
        eprintln!(
            "desired before updates={count}, initial update={first:?}, resident after reclaim={held}, staged selection={next:?}"
        );
        runtime.shutdown().unwrap();
        let state = control.state.lock().unwrap();
        assert_eq!(state.busy_begins, 0, "must make room before AssetBegin");
        assert_eq!(state.evicted_live, 0, "release must retain the live digest");
        assert!(first.is_ok());
        assert!(
            next.is_ok(),
            "a desired staged replacement must remain installable"
        );
    }
}

#[test]
fn dropping_handle_with_full_wake_queue_stops_runtime() {
    let control = MockDeviceControl::default();
    let gate = control.block_next_scene();
    let mut opts = options();
    opts.command_capacity = 1;
    let runtime = start_runtime_with_options(AppConfig::default(), &control, opts);
    gate.wait_until_entered();
    {
        let wake = control
            .state
            .lock()
            .unwrap()
            .event_wake
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        wake();
    }
    drop(runtime);
    gate.open();
    wait_for(Duration::from_secs(1), || {
        control.state.lock().unwrap().device_dropped
    });
    let stopped = control.state.lock().unwrap().device_dropped;
    eprintln!("device dropped after runtime handle dropped and IO unblocked={stopped}");
    assert!(
        stopped,
        "event waker must not keep the orphan runtime alive"
    );
}

struct RingHost {
    selected: Arc<Mutex<[u8; 32]>>,
    wrap: bool,
}

impl app_core::ImageSourceHost for RingHost {
    fn desired_assets(&mut self) -> Vec<DesiredAsset> {
        [1_u8, 2]
            .into_iter()
            .map(|id| DesiredAsset {
                digest: [id; 32],
                kind: protocol::AssetKind::Image,
                bytes: Arc::from(PICTURE_BLOB),
            })
            .collect()
    }
    fn image_source_frame(&mut self, _: &str) -> Option<ImageSourceFrame> {
        Some(ImageSourceFrame {
            digest: *self.selected.lock().unwrap(),
            bytes: Arc::from(PICTURE_BLOB),
            stale: false,
        })
    }
    fn local_tap_ring(&mut self, _: &str) -> Option<app_core::ImageTapRing> {
        Some(app_core::ImageTapRing {
            generation: 1,
            steps: [1_u8, 2]
                .into_iter()
                .map(|id| app_core::ImageTapStep {
                    selector_view: id.to_string(),
                    view: id.to_string(),
                    digest: [id; 32],
                    state: None,
                })
                .collect(),
            wrap: self.wrap,
            plugin: false,
        })
    }
    fn local_tap_selected(&mut self, _: &str, digest: [u8; 32]) -> bool {
        *self.selected.lock().unwrap() = digest;
        true
    }
}
#[derive(Default)]
struct LocalTapSink(Mutex<Vec<Option<u8>>>);
impl CardTapSink for LocalTapSink {
    fn tapped(&self, _: &str, _: &str) {
        self.0.lock().unwrap().push(None);
    }
    fn tapped_local(&self, _: &str, _: &str, _: app_core::ImageTapRing, index: u8) {
        self.0.lock().unwrap().push(Some(index));
    }
}

#[test]
fn local_taps_are_gated_resident_and_do_not_repush_on_selection_or_wrap() {
    for (capable, wrap) in [(false, true), (true, true), (true, false)] {
        let control = MockDeviceControl::default();
        let mut device_status = status(1);
        if !capable {
            device_status.capabilities &= !protocol::CAPABILITY_LOCAL_TAP_VIEWS;
        }
        control.set_status(device_status);
        let selected = Arc::new(Mutex::new([1; 32]));
        let sink = Arc::new(LocalTapSink::default());
        let runtime = RuntimeHandle::start_with_ports(
            picture_config(true),
            Box::new(MockDevice::new(control.clone())),
            options(),
            Some(Box::new(RingHost { selected, wrap })),
            Some(sink.clone()),
        )
        .unwrap();
        wait_for(Duration::from_secs(2), || {
            latest_picture_push(&control.operations()).is_some()
        });
        let operations = control.operations();
        let push = latest_picture_push(&operations).unwrap();
        assert_eq!(push.tap_views.len(), usize::from(capable));
        if capable {
            assert_eq!(push.tap_wrap, wrap);
            let scene_at = operations
                .iter()
                .position(|op| matches!(op, Operation::PushScene(_)))
                .unwrap();
            assert!(
                operations[..scene_at].contains(&Operation::AssetCommit([2; 32])),
                "tap asset must be resident before PushScene"
            );
        }
        let count = control.count_operations(|op| matches!(op, Operation::PushScene(_)));
        for (offset, index) in [1, u8::from(!wrap)].into_iter().enumerate() {
            let mut event = tap_event(u64::try_from(offset + 1).unwrap(), "picture-card");
            event.view_index = Some(index);
            control.push_event(event);
        }
        wait_for(Duration::from_secs(1), || sink.0.lock().unwrap().len() == 2);
        let expected = if !capable {
            vec![None, None]
        } else if wrap {
            vec![Some(1), Some(0)]
        } else {
            vec![Some(1), None]
        };
        assert_eq!(*sink.0.lock().unwrap(), expected);
        // Force a scheduler pass, which also polls the selected frame.
        runtime.snapshot().unwrap();
        assert_eq!(
            control.count_operations(|op| matches!(op, Operation::PushScene(_))),
            count
        );
        runtime.shutdown().unwrap();
    }
}
