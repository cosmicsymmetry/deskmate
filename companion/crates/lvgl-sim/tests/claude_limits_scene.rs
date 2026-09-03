//! Pins the claude-limits curated plugin -- the Claude Code subscription
//! limits face -- at four data states and both orientations, to committed
//! PNGs under `tests/golden/claude-limits/`. Same harness shape as
//! `plugin_scene.rs`, in its own golden subdirectory for the same orphan-check
//! reason. Golden-only by design: see `cases::claude_limits_scene_cases`'s
//! doc for why these rows do not join the hardware framebuffer matrix.

use lvgl_sim::{Simulator, cases};

#[test]
fn claude_limits_goldens_match() {
    let bless = std::env::var("BLESS").is_ok();
    let golden_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/claude-limits");
    std::fs::create_dir_all(&golden_dir).expect("golden dir");
    let mut sim = Simulator::new().expect("simulator");
    let mut failures = Vec::new();
    let mut expected_names = std::collections::BTreeSet::new();

    for (name, request) in cases::claude_limits_scene_cases() {
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
        if !expected_names.contains(&stem) {
            if bless {
                std::fs::remove_file(&path).expect("remove orphan golden");
            } else {
                failures.push(format!(
                    "{stem}: orphan golden with no matching case (run with BLESS=1 to delete)"
                ));
            }
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
