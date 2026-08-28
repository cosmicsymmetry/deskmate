//! Task 8 (plugin-manifest stage): pins `lvgl_sim::cases::plugin_scene_cases()`
//! -- the two curated plugins (`aqi`, `agenda`), each at four data states
//! (fresh, stale, error, empty/missing-data) and both mount orientations --
//! to committed PNGs under `tests/golden/plugin-scene/`.
//!
//! A separate test and golden subdirectory from `tests/scene.rs`, for the
//! same reason that file's is separate from `tests/golden.rs`: these cases
//! render a `Scene` `plugin::compile_scene_with_assets` actually produced
//! from real on-disk manifests, not a hand-built one, and keeping the PNGs
//! in their own subdirectory means `tests/golden.rs`'s orphan check (which
//! lists only `tests/golden/*.png`, non-recursively) never sees them.

use lvgl_sim::{Simulator, cases};

#[test]
fn plugin_scene_goldens_match() {
    let bless = std::env::var("BLESS").is_ok();
    let golden_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/plugin-scene");
    std::fs::create_dir_all(&golden_dir).expect("golden dir");
    let mut sim = Simulator::new().expect("simulator");
    let mut failures = Vec::new();
    let mut expected_names = std::collections::BTreeSet::new();

    for (name, request) in cases::plugin_scene_cases() {
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
        "plugin scene golden mismatches:\n{}",
        failures.join("\n")
    );
}

/// Every plugin x state combination is covered, named explicitly (not just
/// counted) so a state silently dropped from `plugin_case`'s state table
/// fails here by name rather than only shrinking a total.
#[test]
fn every_plugin_state_has_a_case() {
    let names: Vec<String> = cases::plugin_scene_cases()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    for plugin_name in ["aqi", "agenda"] {
        for state in ["fresh", "stale", "error", "empty"] {
            for orientation in ["landscape", "flipped"] {
                let expected = format!("plugin-{plugin_name}--{state}--{orientation}");
                assert!(
                    names.contains(&expected),
                    "no plugin scene case named {expected}"
                );
            }
        }
    }
    assert_eq!(names.len(), 16, "2 plugins x 4 states x 2 orientations");
}

/// The 270° mount is a 180° rotation of the logical canvas -- see
/// `tests/scene.rs::every_case_flips_to_an_exact_reversal`'s doc for exactly
/// what this does and does not prove.
#[test]
fn every_plugin_case_flips_to_an_exact_reversal() {
    let mut sim = Simulator::new().expect("simulator");
    let cases = cases::plugin_scene_cases();
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
