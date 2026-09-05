use std::collections::BTreeSet;

use lvgl_sim::{SimError, Simulator};

/// Runs one golden suite while storing only landscape frames. Flipped cases
/// remain in the source tables for hardware use; the simulator's one
/// `copy_frame_out` reversal is pinned separately once per render wrapper.
pub fn assert_goldens<T, Cases, Before, Render>(
    subdirectory: &str,
    bless: bool,
    failure_prefix: Option<&str>,
    orphan_detail: &str,
    cases: Cases,
    mut before_render: Before,
    mut render: Render,
) where
    Cases: IntoIterator<Item = (String, T)>,
    Before: FnMut(&str, &T),
    Render: FnMut(&mut Simulator, &T) -> Result<Vec<u8>, SimError>,
{
    let golden_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let golden_dir = if subdirectory.is_empty() {
        golden_root
    } else {
        golden_root.join(subdirectory)
    };
    std::fs::create_dir_all(&golden_dir).expect("golden dir");

    let mut simulator = Simulator::new().expect("simulator");
    let mut failures = Vec::new();
    let mut expected_names = BTreeSet::new();

    for (name, request) in cases {
        if name.ends_with("--flipped") {
            continue;
        }
        expected_names.insert(name.clone());
        before_render(&name, &request);
        let png =
            render(&mut simulator, &request).unwrap_or_else(|error| panic!("{name}: {error}"));
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
        let path = entry.expect("golden dir entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("png") {
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
            failures.push(format!("{stem}: {orphan_detail}"));
        }
    }

    let failure_detail = failures.join("\n");
    let failure_message = match failure_prefix {
        Some(prefix) => format!("{prefix}:\n{failure_detail}"),
        None => failure_detail,
    };
    assert!(failures.is_empty(), "{failure_message}");
}
