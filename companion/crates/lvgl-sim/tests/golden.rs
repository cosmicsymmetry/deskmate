use lvgl_sim::{Simulator, cases};

#[test]
fn golden_frames_match() {
    let bless = std::env::var("BLESS").is_ok();
    let golden_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    std::fs::create_dir_all(&golden_dir).expect("golden dir");
    let mut sim = Simulator::new().expect("simulator");
    let mut failures = Vec::new();
    for (name, request) in cases::golden_cases() {
        let png = sim.render_png(&request).expect(&name);
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
    assert!(
        failures.is_empty(),
        "golden mismatches:\n{}",
        failures.join("\n")
    );
}
