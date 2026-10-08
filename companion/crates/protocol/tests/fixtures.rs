use protocol::{
    Deframer, FrameError, MAX_PAYLOAD_SIZE, MessageError, decode_message, decode_wire_frame,
    encode_message,
};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../protocol/fixtures/v2");

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{ROOT}/{name}")).unwrap()
}

#[test]
fn valid_golden_frames_round_trip_byte_exactly() {
    for name in [
        "status_request.bin",
        "status_response.bin",
        "time_sync.bin",
        "ack_time.bin",
        "push_timer.bin",
        "ack_push.bin",
        "heartbeat.bin",
        "heartbeat_ack.bin",
        "error.bin",
        "apply_config_min.bin",
        "ack_config.bin",
        "apply_config_max.bin",
        "activate_card.bin",
        "ack_activate.bin",
        "trigger_interrupt.bin",
        "ack_interrupt.bin",
        "device_event_tap.bin",
        "device_event_previous.bin",
        "device_event_next.bin",
        "device_event_dismissed.bin",
        "error_unknown_card.bin",
        "error_config_too_large.bin",
        "push_timer_paused.bin",
        "network_config.bin",
        "factory_reset.bin",
        "status_response_networked.bin",
        "status_response_ota_failed.bin",
        "asset_begin.bin",
        "asset_begin_rle.bin",
        "ack_asset_begin.bin",
        "asset_chunk.bin",
        "asset_commit.bin",
        "asset_release.bin",
        "push_scene.bin",
        "push_scene_min.bin",
        "ack_scene.bin",
    ] {
        let bytes = fixture(name);
        let frame = decode_wire_frame(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        let message = decode_message(&frame).unwrap_or_else(|error| panic!("{name}: {error}"));
        let wire = encode_message(frame.request_id, &message)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            wire, bytes,
            "{name}: re-encode differs from the golden frame"
        );
    }
}

#[test]
fn ota_failure_fixture_is_additive_to_old_status_frames() {
    let old = decode_wire_frame(&fixture("status_response_networked.bin")).unwrap();
    let protocol::Message::StatusResponse(old) = decode_message(&old).unwrap() else {
        panic!("old fixture should remain a status response");
    };
    assert_eq!(old.last_ota_error, None);

    let current = decode_wire_frame(&fixture("status_response_ota_failed.bin")).unwrap();
    let protocol::Message::StatusResponse(current) = decode_message(&current).unwrap() else {
        panic!("OTA fixture should be a status response");
    };
    assert_eq!(current.ota_state, protocol::OtaState::Failed);
    assert_eq!(
        current.last_ota_error.as_deref(),
        Some("download: ESP_ERR_NO_MEM")
    );
}

#[test]
fn invalid_golden_inputs_have_stable_classes() {
    assert_eq!(
        decode_wire_frame(&fixture("bad_crc.bin")),
        Err(FrameError::Checksum)
    );
    let version = decode_wire_frame(&fixture("unsupported_version.bin")).unwrap();
    assert_eq!(decode_message(&version), Err(MessageError::Version(1)));
    let kind = decode_wire_frame(&fixture("unsupported_type.bin")).unwrap();
    assert_eq!(
        decode_message(&kind),
        Err(MessageError::UnsupportedType(99))
    );
    let invalid = decode_wire_frame(&fixture("invalid_cbor.bin")).unwrap();
    assert!(matches!(
        decode_message(&invalid),
        Err(MessageError::Cbor(_))
    ));
    let duplicate = decode_wire_frame(&fixture("duplicate_keys.bin")).unwrap();
    assert_eq!(
        decode_message(&duplicate),
        Err(MessageError::DuplicateOrUnsortedKey)
    );
    let duplicate_cards = decode_wire_frame(&fixture("duplicate_card_ids.bin")).unwrap();
    assert_eq!(
        decode_message(&duplicate_cards),
        Err(MessageError::DuplicateOrUnsortedKey)
    );
    // A card's tap action is the only enumerated value left on the wire, so it
    // is the only one that can be unsupported. Templates and size classes went
    // with protocol v2.
    let action = decode_wire_frame(&fixture("unsupported_tap_action_config.bin")).unwrap();
    assert_eq!(
        decode_message(&action),
        Err(MessageError::InvalidValue("tap action"))
    );
    let excessive = decode_wire_frame(&fixture("config_too_many_cards.bin")).unwrap();
    assert_eq!(
        decode_message(&excessive),
        Err(MessageError::ConfigTooLarge)
    );
    let utf8 = decode_wire_frame(&fixture("invalid_config_utf8.bin")).unwrap();
    assert!(matches!(decode_message(&utf8), Err(MessageError::Cbor(_))));
    for name in ["zero_request_config.bin", "nonzero_request_event.bin"] {
        let frame = decode_wire_frame(&fixture(name)).unwrap();
        assert_eq!(
            decode_message(&frame),
            Err(MessageError::InvalidRequestId),
            "{name}"
        );
    }
    let results = Deframer::new().push(&fixture("overlong.bin"));
    assert_eq!(results, vec![Err(FrameError::Overlong)]);
    assert!(Deframer::new().push(&fixture("garbage.bin"))[0].is_err());
}

#[test]
fn maximum_config_fixture_stays_inside_one_frame() {
    let frame = decode_wire_frame(&fixture("apply_config_max.bin")).unwrap();
    assert_eq!(frame.payload.len(), 317);
    assert!(frame.payload.len() <= MAX_PAYLOAD_SIZE);
    decode_message(&frame).unwrap();
}

#[test]
fn current_capabilities_advertise_implemented_features() {
    assert_eq!(protocol::CAPABILITY_PIPELINED_ASSET_CHUNKS, 4096);
    assert_eq!(protocol::ASSET_CHUNK_WINDOW, 8);
    assert_eq!(protocol::CAPABILITY_SCENE_RENDER, 256);
    assert_eq!(protocol::CAPABILITY_VOLATILE_ASSETS, 512);
    assert_eq!(protocol::CAPABILITY_DURABLE_ASSET_ENCODING, 1024);
    assert_eq!(
        protocol::CURRENT_CAPABILITIES,
        protocol::CAPABILITY_ASSET_TRANSFER
            | protocol::CAPABILITY_FIRMWARE_UPDATE
            | protocol::CAPABILITY_NETWORKING
            | protocol::CAPABILITY_SCENE_RENDER
            | protocol::CAPABILITY_VOLATILE_ASSETS
            | protocol::CAPABILITY_DURABLE_ASSET_ENCODING
            | protocol::CAPABILITY_DISPLAY_BRIGHTNESS
            | protocol::CAPABILITY_PIPELINED_ASSET_CHUNKS,
        "implemented asset transfer, firmware update, networking, scene \
         rendering, volatile-asset, and durable-asset encoding features must be advertised"
    );
    // Protocol v2 re-based the word: bits 0-4 described a device that rendered
    // templates itself and are retired, never re-issued.
    assert_eq!(protocol::CURRENT_CAPABILITIES, 8160);
}

fn assert_rot_rect_clip_is_pinned(nodes: &[protocol::SceneNode]) {
    let rot_rect = nodes
        .iter()
        .find_map(|node| match node {
            protocol::SceneNode::RotRect(rect) => Some(rect),
            _ => None,
        })
        .expect("the rich fixture carries a rotated rect");
    assert_eq!(
        rot_rect.clip,
        Some(protocol::SceneClipRect {
            x: 64,
            y: 24,
            w: 320,
            h: 320,
        })
    );
}

fn assert_running_colors_are_pinned(nodes: &[protocol::SceneNode]) {
    let arc = nodes
        .iter()
        .find_map(|node| match node {
            protocol::SceneNode::Arc(arc) => Some(arc),
            _ => None,
        })
        .expect("the rich fixture carries an arc");
    assert_eq!(arc.running_color, Some(0x0064_D2FF));
    let text = nodes
        .iter()
        .find_map(|node| match node {
            protocol::SceneNode::Text(text) if text.running_color.is_some() => Some(text),
            _ => None,
        })
        .expect("the rich fixture carries a text running color");
    assert_eq!(text.running_color, Some(0x00FF_9F0A));
}

fn assert_rich_rect_is_pinned(nodes: &[protocol::SceneNode]) {
    let rect = nodes
        .iter()
        .find_map(|node| match node {
            protocol::SceneNode::Rect(rect) => Some(rect),
            _ => None,
        })
        .expect("the rich fixture carries a rect");
    assert_eq!(rect.opacity, 0xC8);
    assert_eq!(
        rect.clip,
        Some(protocol::SceneClipRect {
            x: 16,
            y: 16,
            w: 416,
            h: 336,
        })
    );
}

fn assert_rich_arc_is_pinned(nodes: &[protocol::SceneNode]) {
    let arc = nodes
        .iter()
        .find_map(|node| match node {
            protocol::SceneNode::Arc(arc) => Some(arc),
            _ => None,
        })
        .expect("the rich fixture carries an arc");
    assert_eq!(arc.opacity, 0x33);
}

fn assert_bound_line_is_pinned(nodes: &[protocol::SceneNode]) {
    let line = nodes
        .iter()
        .find_map(|node| match node {
            protocol::SceneNode::Line(line) if !line.angle_binding.is_empty() => Some(line),
            _ => None,
        })
        .expect("the rich fixture carries a bound line");
    assert!(line.xs.is_empty());
    assert!(line.ys.is_empty());
    assert_eq!(line.pivot_x, 224);
    assert_eq!(line.pivot_y, 184);
    assert_eq!(line.length, 80);
    assert_eq!(line.angle_binding, "time:angle:hour");
}

fn assert_date_binding_is_pinned(nodes: &[protocol::SceneNode]) {
    assert!(nodes.iter().any(|node| {
        matches!(
            node,
            protocol::SceneNode::Text(protocol::SceneText {
                value: protocol::SceneValue::Binding(binding),
                ..
            }) if binding == "date"
        )
    }));
}

fn assert_time_binding_is_pinned(nodes: &[protocol::SceneNode]) {
    assert!(nodes.iter().any(|node| {
        matches!(
            node,
            protocol::SceneNode::Text(protocol::SceneText {
                value: protocol::SceneValue::Binding(binding),
                ..
            }) if binding == "time:HH:mm"
        )
    }));
}

fn assert_label_anchor_default_is_pinned(nodes: &[protocol::SceneNode]) {
    let label = nodes
        .iter()
        .find_map(|node| match node {
            protocol::SceneNode::Label(label) => Some(label),
            _ => None,
        })
        .expect("the rich fixture carries a label");
    assert_eq!(
        label.horizontal_anchor,
        protocol::SceneLabelAnchor::Left,
        "the pre-anchor fixture must keep decoding an omitted key as Left"
    );
}

fn assert_every_node_kind_is_pinned(nodes: &[protocol::SceneNode]) {
    let kinds: Vec<std::mem::Discriminant<protocol::SceneNode>> =
        nodes.iter().map(std::mem::discriminant).collect();
    for probe in [
        protocol::SceneNode::Rect(protocol::SceneRect::default()),
        protocol::SceneNode::Arc(protocol::SceneArc::default()),
        protocol::SceneNode::Line(protocol::SceneLine::default()),
        protocol::SceneNode::Text(protocol::SceneText::default()),
        protocol::SceneNode::Image(protocol::SceneImage::default()),
        protocol::SceneNode::Glyph(protocol::SceneGlyph::default()),
        protocol::SceneNode::Scale(protocol::SceneScale::default()),
        protocol::SceneNode::Label(protocol::SceneLabel::default()),
        protocol::SceneNode::RotRect(protocol::SceneRotRect::default()),
    ] {
        assert!(
            kinds.contains(&std::mem::discriminant(&probe)),
            "the rich fixture must cover every node kind"
        );
    }
}

fn assert_minimal_scene_defaults_are_pinned() {
    // The minimal fixture is the other half of the canonical emission rule:
    // every optional key at its default, so the file proves both encoders
    // omit them rather than spelling them out.
    let minimal = decode_wire_frame(&fixture("push_scene_min.bin")).unwrap();
    let protocol::Message::PushScene(minimal) = decode_message(&minimal).unwrap() else {
        panic!("push_scene_min.bin should be a PushScene");
    };
    assert_eq!(minimal.scene.nodes.len(), 1);
    let protocol::SceneNode::Rect(rect) = &minimal.scene.nodes[0] else {
        panic!("the minimal fixture's only node is a rect");
    };
    // An omitted opacity means opaque, which is the one default that is not
    // the zero value.
    assert_eq!(rect.opacity, u8::MAX);
    assert_eq!(rect.clip, None);
    assert_eq!(
        *rect,
        protocol::SceneRect {
            x: 4,
            y: 5,
            w: 10,
            h: 11,
            ..protocol::SceneRect::default()
        }
    );
}

#[test]
fn push_scene_fixtures_carry_what_they_are_meant_to() {
    let frame = decode_wire_frame(&fixture("push_scene.bin")).unwrap();
    let protocol::Message::PushScene(push) = decode_message(&frame).unwrap() else {
        panic!("push_scene.bin should be a PushScene");
    };
    assert_eq!(push.card_id, "clock");
    assert_eq!(push.revision, 12);
    // Every node kind, both Line forms, both Text value forms, and both live
    // wall-clock text bindings: an
    // all-minimal fixture would pass even if the encoders disagreed about the
    // kinds or additive fields it left out.
    assert_eq!(push.scene.nodes.len(), 12);
    assert_rich_rect_is_pinned(&push.scene.nodes);
    assert_rich_arc_is_pinned(&push.scene.nodes);
    assert_rot_rect_clip_is_pinned(&push.scene.nodes);
    assert_running_colors_are_pinned(&push.scene.nodes);
    assert_bound_line_is_pinned(&push.scene.nodes);
    assert_time_binding_is_pinned(&push.scene.nodes);
    assert_date_binding_is_pinned(&push.scene.nodes);
    assert_label_anchor_default_is_pinned(&push.scene.nodes);
    assert_every_node_kind_is_pinned(&push.scene.nodes);
    assert_minimal_scene_defaults_are_pinned();
}

#[test]
fn brightness_fixtures_pin_presence_floor_and_narrowing() {
    for (name, expected) in [
        ("apply_config_min.bin", None),
        ("apply_config_brightness_min.bin", Some(26)),
        ("apply_config_brightness_default.bin", Some(200)),
        ("apply_config_brightness_max.bin", Some(255)),
    ] {
        let wire = fixture(name);
        let frame = decode_wire_frame(&wire).unwrap();
        let message = decode_message(&frame).unwrap();
        let protocol::Message::ApplyConfig(ref config) = message else {
            panic!("config");
        };
        assert_eq!(config.brightness, expected);
        assert_eq!(
            protocol::encode_message(frame.request_id, &message).unwrap(),
            wire
        );
    }
    for name in [
        "brightness_zero.bin",
        "brightness_below_floor.bin",
        "brightness_overflow.bin",
        "brightness_negative.bin",
        "brightness_boolean.bin",
    ] {
        let frame = decode_wire_frame(&fixture(name)).unwrap();
        assert!(decode_message(&frame).is_err(), "{name}");
    }
}
