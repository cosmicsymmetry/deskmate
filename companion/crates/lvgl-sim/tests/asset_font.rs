//! Task 12: pins `lvgl_sim::cases::asset_font_cases()` — the runtime
//! font-asset parity golden — to committed PNGs under
//! `tests/golden/asset-font/`.
//!
//! Deliberately a separate test (and a separate golden subdirectory) from
//! `tests/golden.rs`/`tests/golden/`: `asset_font_cases()` returns
//! `cases::AssetFontCase`, not `RenderRequest`, and is rendered through
//! `Simulator::render_asset_font_png`, not `Simulator::render_png` — see
//! `cases.rs`'s doc comment on [`lvgl_sim::cases::AssetFontCase`] for why
//! it cannot simply join `golden_cases()`. Keeping its PNGs in a
//! subdirectory (rather than `tests/golden/` itself) means
//! `tests/golden.rs`'s orphan-PNG check — which lists only
//! `tests/golden/*.png`, non-recursively — never sees them and does not
//! need to know this case table exists.

use lvgl_sim::{Simulator, cases};

#[test]
fn asset_font_golden_matches() {
    let bless = std::env::var("BLESS").is_ok();
    let golden_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/asset-font");
    std::fs::create_dir_all(&golden_dir).expect("golden dir");
    let mut sim = Simulator::new().expect("simulator");
    let mut failures = Vec::new();
    let mut expected_names = std::collections::BTreeSet::new();

    for (name, case) in cases::asset_font_cases() {
        expected_names.insert(name.clone());
        let png = sim.render_asset_font_png(&case).expect(&name);
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
        "asset-font golden mismatches:\n{}",
        failures.join("\n")
    );
}
