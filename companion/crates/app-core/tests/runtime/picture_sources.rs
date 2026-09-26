use super::*;

#[test]
fn a_picture_card_builds_a_single_full_canvas_image_scene() {
    let control = MockDeviceControl::default();
    let host = FakeImageSourceHostControl::default();
    let digest = [0x41; protocol::ASSET_DIGEST_LEN];
    host.stage_picture_frame("camera", digest, PICTURE_BLOB, false);
    let runtime =
        start_picture_runtime(picture_config(true), &control, Some(Box::new(host.host())));

    wait_for(Duration::from_secs(1), || {
        latest_picture_push(&control.operations()).is_some_and(|push| {
            matches!(push.scene.nodes.first(), Some(SceneNode::Image(image)) if image.digest == digest)
        })
    });
    let push = latest_picture_push(&control.operations()).expect("the picture scene was pushed");
    assert_eq!(push.scene.nodes.len(), 1);
    let SceneNode::Image(image) = &push.scene.nodes[0] else {
        panic!("a picture card's face is one image node");
    };
    assert_eq!(image.x, 0);
    assert_eq!(image.y, 0);
    assert_eq!(image.w, protocol::SCENE_CANVAS_WIDTH);
    assert_eq!(image.h, protocol::SCENE_CANVAS_HEIGHT);
    assert_eq!(image.digest, digest);
    assert!(!image.recolor);
    runtime.shutdown().unwrap();
}

#[test]
fn a_stale_picture_card_adds_the_shared_footer_and_nothing_else() {
    let control = MockDeviceControl::default();
    let host = FakeImageSourceHostControl::default();
    host.stage_picture_frame(
        "camera",
        [0x42; protocol::ASSET_DIGEST_LEN],
        PICTURE_BLOB,
        true,
    );
    let runtime =
        start_picture_runtime(picture_config(true), &control, Some(Box::new(host.host())));

    wait_for(Duration::from_secs(1), || {
        latest_picture_push(&control.operations()).is_some_and(|push| push.scene.nodes.len() == 2)
    });
    let push = latest_picture_push(&control.operations()).expect("the stale picture was pushed");
    assert_eq!(push.scene.nodes.len(), 2);
    assert!(matches!(push.scene.nodes[0], SceneNode::Image(_)));
    assert!(matches!(push.scene.nodes[1], SceneNode::Text(_)));
    runtime.shutdown().unwrap();
}

#[test]
fn a_picture_source_with_no_frame_yet_says_so_in_words() {
    let control = MockDeviceControl::default();
    let host = FakeImageSourceHostControl::default();
    let runtime =
        start_picture_runtime(picture_config(true), &control, Some(Box::new(host.host())));

    wait_for(Duration::from_secs(1), || {
        latest_picture_push(&control.operations()).is_some()
    });
    let push = latest_picture_push(&control.operations()).expect("the waiting face was pushed");
    assert_eq!(push.scene.nodes.len(), 1);
    let SceneNode::Text(text) = &push.scene.nodes[0] else {
        panic!("a source with no frame draws one state word");
    };
    assert_eq!(
        text.value,
        SceneValue::Literal("Waiting for the first picture".into())
    );
    assert!(
        !push
            .scene
            .nodes
            .iter()
            .any(|node| matches!(node, SceneNode::Image(_)))
    );
    runtime.shutdown().unwrap();
}

#[test]
fn a_picture_card_with_no_host_refuses_distinguishably() {
    let control = MockDeviceControl::default();
    let runtime = start_picture_runtime(picture_config(true), &control, None);

    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.card_errors.iter().any(|error| {
            error.card_id == "picture-card" && error.message.contains("no image source host")
        })
    });
    let refusal = snapshot
        .card_errors
        .iter()
        .find(|error| error.card_id == "picture-card")
        .expect("the picture refusal is visible");
    assert!(refusal.message.contains("no image source host"));
    runtime.shutdown().unwrap();
}

#[test]
fn a_picture_card_negotiates_native_once_its_frame_is_installable() {
    let control = MockDeviceControl::default();
    let host = FakeImageSourceHostControl::default();
    let digest = [0x43; protocol::ASSET_DIGEST_LEN];
    host.stage_picture_frame("camera", digest, PICTURE_BLOB, false);
    let runtime =
        start_picture_runtime(picture_config(true), &control, Some(Box::new(host.host())));

    wait_for(Duration::from_secs(1), || {
        latest_picture_push(&control.operations()).is_some()
    });
    let push = latest_picture_push(&control.operations()).expect("the picture scene was pushed");
    let requirements = analyze_scene(&push.scene);
    let profile = DeviceRenderProfile {
        capabilities: protocol::CURRENT_CAPABILITIES,
        confirmed_assets: BTreeSet::new(),
        installable_assets: BTreeSet::from([digest]),
    };
    validate_native_scene(&requirements, &profile).unwrap();
    runtime.shutdown().unwrap();
}

#[test]
fn a_refused_picture_scene_is_retried_only_for_a_new_digest() {
    let control = MockDeviceControl::default();
    control.refuse_scenes_for("picture-card");
    let host = FakeImageSourceHostControl::default();
    let initial_digest = [0x44; protocol::ASSET_DIGEST_LEN];
    let updated_digest = [0x45; protocol::ASSET_DIGEST_LEN];
    host.stage_picture_frame("camera", initial_digest, PICTURE_BLOB, false);
    let runtime =
        start_picture_runtime(picture_config(true), &control, Some(Box::new(host.host())));

    wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.card_errors.iter().any(|error| {
            error.kind == CardErrorKind::SceneRefused && error.card_id == "picture-card"
        })
    });
    let picture_attempts = || {
        control.count_operations(|operation| {
            matches!(operation, Operation::PushScene(push) if push.card_id == "picture-card")
        })
    };
    assert_eq!(picture_attempts(), 1);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        picture_attempts(),
        1,
        "an unchanged refused picture candidate must stay terminal"
    );

    host.stage_picture_frame("camera", updated_digest, PICTURE_BLOB, false);
    runtime
        .image_source_updated("camera", updated_digest)
        .unwrap();
    wait_for(Duration::from_secs(1), || picture_attempts() == 2);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        picture_attempts(),
        2,
        "the new digest is evaluated once and then stays terminal"
    );
    assert!(runtime.snapshot().unwrap().card_errors.iter().any(|error| {
        error.kind == CardErrorKind::SceneRefused && error.card_id == "picture-card"
    }));
    runtime.shutdown().unwrap();
}

#[test]
fn an_image_source_update_for_a_card_that_is_not_on_screen_pushes_no_scene() {
    let existing_digest = [0x50; protocol::ASSET_DIGEST_LEN];
    let picture_digest = [0x51; protocol::ASSET_DIGEST_LEN];
    let operations = run_image_source_update_case(false, existing_digest, picture_digest);
    assert_eq!(
        operations.len(),
        7,
        "unexpected update transcript: {operations:?}"
    );
    assert_image_update_asset_prefix(&operations, existing_digest, picture_digest);
    assert!(
        operations
            .iter()
            .all(|operation| !matches!(operation, Operation::PushScene(_))),
        "an off-screen frame becomes resident without rebuilding the visible face"
    );
}

#[test]
fn an_image_source_update_for_the_visible_card_reaches_the_glass_before_any_flash_work() {
    // The interactive contract, and the reason the volatile tier exists: the
    // frame on the glass is drawn from PSRAM first. Durable housekeeping may
    // follow and may cost flash, but it may not stand in front of the picture.
    // Measured on `dev-0005` 2026-09-26, the durable-first order cost 13.3 s
    // from tap to pixels, 9.4 s of it compaction with the clock on screen.
    let existing_digest = [0x52; protocol::ASSET_DIGEST_LEN];
    let picture_digest = [0x53; protocol::ASSET_DIGEST_LEN];
    let operations = run_image_source_update_case(true, existing_digest, picture_digest);

    let volatile_at = operations
        .iter()
        .position(|operation| {
            matches!(operation, Operation::VolatileAssetBegin(digest) if digest == &picture_digest)
        })
        .unwrap_or_else(|| {
            panic!("the visible card's frame was not sent to PSRAM: {operations:?}")
        });
    let scene_at = operations
        .iter()
        .position(|operation| {
            matches!(operation, Operation::PushScene(push) if push.scene.nodes.iter().any(
                |node| matches!(node, SceneNode::Image(image) if image.digest == picture_digest)
            ))
        })
        .unwrap_or_else(|| panic!("the new frame never reached a scene: {operations:?}"));
    assert!(
        volatile_at < scene_at,
        "the scene must name a digest the device already holds: {operations:?}"
    );

    let durable_at = operations.iter().position(
        |operation| matches!(operation, Operation::AssetBegin(digest) if digest == &picture_digest),
    );
    let release_at = operations
        .iter()
        .position(|operation| matches!(operation, Operation::AssetRelease(_)));
    for (label, flash_at) in [
        ("the durable write", durable_at),
        ("the release", release_at),
    ] {
        if let Some(flash_at) = flash_at {
            assert!(
                scene_at < flash_at,
                "{label} ran before the picture was on screen: {operations:?}"
            );
        }
    }
}

#[test]
fn a_failed_asset_transfer_pushes_no_scene_and_releases_nothing() {
    let control = MockDeviceControl::default();
    let host = FakeImageSourceHostControl::default();
    let previous_digest = [0x54; protocol::ASSET_DIGEST_LEN];
    let failed_digest = [0x55; protocol::ASSET_DIGEST_LEN];
    host.stage_picture_frame("camera", previous_digest, PICTURE_BLOB, false);
    let runtime =
        start_picture_runtime(picture_config(true), &control, Some(Box::new(host.host())));
    wait_for(Duration::from_secs(1), || {
        latest_picture_push(&control.operations()).is_some_and(|push| {
            push.scene.nodes.iter().any(
                |node| matches!(node, SceneNode::Image(image) if image.digest == previous_digest),
            )
        })
    });
    let before = control.operations().len();

    control.refuse_asset_chunks_for(failed_digest);
    host.stage_picture_frame("camera", failed_digest, PICTURE_BLOB, false);
    runtime
        .image_source_updated("camera", failed_digest)
        .expect_err("the failed durable transfer reaches the command caller");
    wait_for(Duration::from_secs(1), || {
        control.operations()[before..].iter().any(
            |operation| matches!(operation, Operation::AssetChunk(digest, _) if digest == &failed_digest),
        )
    });
    thread::sleep(Duration::from_millis(30));

    let operations = control.operations();
    let after = &operations[before..];
    assert!(
        after
            .iter()
            .all(|operation| !matches!(operation, Operation::AssetRelease(_))),
        "an incomplete desired-set pass must release nothing: {after:?}"
    );
    assert!(
        after
            .iter()
            .all(|operation| !matches!(operation, Operation::PushScene(_))),
        "a failed transfer must leave the last good face active: {after:?}"
    );
    let last_good = latest_picture_push(&operations).expect("the old face remains recorded");
    assert!(
        last_good
            .scene
            .nodes
            .iter()
            .any(|node| matches!(node, SceneNode::Image(image) if image.digest == previous_digest))
    );
    runtime.shutdown().unwrap();
}

#[test]
fn a_staleness_flip_on_the_visible_card_rebuilds_its_scene() {
    let control = MockDeviceControl::default();
    let host = FakeImageSourceHostControl::default();
    let digest = [0x56; protocol::ASSET_DIGEST_LEN];
    host.stage_picture_frame("camera", digest, PICTURE_BLOB, false);
    let runtime =
        start_picture_runtime(picture_config(true), &control, Some(Box::new(host.host())));
    wait_for(Duration::from_secs(1), || {
        latest_picture_push(&control.operations()).is_some_and(|push| push.scene.nodes.len() == 1)
    });
    let pushes_before = control.count_operations(|operation| {
        matches!(operation, Operation::PushScene(push) if push.card_id == "picture-card")
    });

    host.set_picture_stale("camera", true);
    wait_for(Duration::from_secs(1), || {
        control.count_operations(|operation| {
            matches!(operation, Operation::PushScene(push) if push.card_id == "picture-card")
        })
            > pushes_before
    });
    let push = latest_picture_push(&control.operations()).expect("the stale face was rebuilt");
    assert_eq!(push.scene.nodes.len(), 2);
    assert!(matches!(push.scene.nodes[0], SceneNode::Image(_)));
    assert!(matches!(push.scene.nodes[1], SceneNode::Text(_)));
    runtime.shutdown().unwrap();
}
