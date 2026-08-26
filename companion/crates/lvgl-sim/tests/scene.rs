//! Task 8 (stage 2a): pins `lvgl_sim::cases::scene_cases()` — one scene per
//! `scene_node_kind_t`, at both mount orientations — to committed PNGs under
//! `tests/golden/scene/`.
//!
//! A separate test and golden subdirectory from `tests/golden.rs`, for the same
//! reason `tests/asset_font.rs` is: these cases are `SceneRenderRequest`s
//! rendered through `Simulator::render_scene_png` — the firmware's scene
//! decoder and `ui/scene_view.c` interpreter — not `RenderRequest`s through
//! `template_view_show`. Keeping the PNGs in a subdirectory means
//! `tests/golden.rs`'s orphan check, which lists only `tests/golden/*.png`
//! non-recursively, never sees them.

use lvgl_sim::{Simulator, cases};

#[test]
fn scene_goldens_match() {
    let bless = std::env::var("BLESS").is_ok();
    let golden_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/scene");
    std::fs::create_dir_all(&golden_dir).expect("golden dir");
    let mut sim = Simulator::new().expect("simulator");
    let mut failures = Vec::new();
    let mut expected_names = std::collections::BTreeSet::new();

    for (name, request) in cases::scene_cases() {
        expected_names.insert(name.clone());
        let png = sim.render_scene_png(&request).expect(&name);
        let path = golden_dir.join(format!("{name}.png"));
        if bless {
            std::fs::write(&path, &png).expect("write golden");
            continue;
        }
        match std::fs::read(&path) {
            Ok(expected) if expected == png => {}
            Ok(_) => failures.push(format!("{name}: pixels differ")),
            Err(_) => failures.push(format!("{name}: golden missing (run with BLESS=1)")),
        }
    }

    for entry in std::fs::read_dir(&golden_dir).expect("read golden dir") {
        let entry = entry.expect("golden dir entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        if expected_names.contains(&stem) {
            continue;
        }
        if bless {
            std::fs::remove_file(&path).expect("remove orphan golden");
        } else {
            failures.push(format!(
                "{stem}: orphan golden with no matching case (run with BLESS=1 to delete)"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "scene golden mismatches:\n{}",
        failures.join("\n")
    );
}

/// Every `scene_node_kind_t` is covered. Written as an explicit roll-call
/// rather than a count so that adding another node kind to the model fails
/// here, loudly, instead of leaving the new kind with no golden at all.
#[test]
fn every_node_kind_has_a_case() {
    let names: Vec<String> = cases::scene_cases()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    for kind in [
        "rect", "arc", "line", "text", "image", "glyph", "scale", "label", "rot-rect",
    ] {
        for orientation in ["landscape", "flipped"] {
            let expected = format!("scene-{kind}--{orientation}");
            assert!(
                names.contains(&expected),
                "no scene case named {expected}; every scene_node_kind_t needs \
                 one at both orientations"
            );
        }
    }
    assert_eq!(names.len(), 18, "9 node kinds x 2 orientations");
}

/// The 270° mount is a 180° rotation of the logical canvas, so every case's
/// two goldens must be exact reversals of each other.
///
/// This does *not* exercise per-node-kind orientation handling: `sim_shim.c`'s
/// `copy_frame_out` reaches "flipped" by reversing the frame buffer
/// index-by-index after the landscape render, without LVGL ever being told to
/// rotate, so no node kind's geometry is re-evaluated here. What this test
/// actually pins is `copy_frame_out`'s reversal and render determinism — plus
/// it is why the "flipped" goldens have a real job: they are the 270°
/// reference Tasks 9 and 10 compare the *device* against, where the rotation
/// is real and per-node-kind geometry genuinely is exercised.
#[test]
fn every_case_flips_to_an_exact_reversal() {
    let mut sim = Simulator::new().expect("simulator");
    let cases = cases::scene_cases();
    for pair in cases.as_chunks::<2>().0 {
        let (landscape_name, landscape) = &pair[0];
        let (flipped_name, flipped) = &pair[1];
        assert!(
            landscape_name.ends_with("--landscape") && flipped_name.ends_with("--flipped"),
            "cases are expected in landscape/flipped pairs, got \
             {landscape_name} and {flipped_name}"
        );
        let plain = sim.render_scene(landscape).expect(landscape_name);
        let turned = sim.render_scene(flipped).expect(flipped_name);
        let reversed: Vec<u16> = plain.iter().rev().copied().collect();
        assert_eq!(turned, reversed, "{flipped_name} is not a 180° rotation");
    }
}
