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
