//! Stage 4 Task 6 evidence rows. These are neither additions to the retired
//! C-template parity headline nor aliases for the curated-v1 plugin matrix.

use std::collections::{BTreeSet, HashMap};

use lvgl_sim::{LOGICAL_WIDTH, Simulator, cases};
use protocol::{SceneNode, SceneValue};

fn evidence_cases() -> Vec<(String, lvgl_sim::scene::SceneRenderRequest)> {
    cases::timer_producer_scene_cases()
        .into_iter()
        .chain(cases::date_truncation_scene_cases())
        .collect()
}

fn assert_timer_producer_meaning(request: &lvgl_sim::scene::SceneRenderRequest) {
    let timer = request.timer.expect("the v2 timer row has a snapshot");
    let remaining_pct = u64::from(timer.remaining_ms) * 100 / u64::from(timer.total_ms);
    let elapsed_pct = 100 - remaining_pct;
    assert_eq!(remaining_pct, 35, "the producer fact is remaining percent");
    assert_eq!(elapsed_pct, 65, "the inverse must be visibly distinct");
    assert_ne!(remaining_pct, elapsed_pct);
}

#[test]
fn committed_v2_manifest_emits_timer_bindings_from_a_producer_shaped_snapshot() {
    let cases = cases::timer_producer_scene_cases();
    assert_eq!(cases.len(), 2, "one row at both orientations");
    for (name, request) in cases {
        assert_timer_producer_meaning(&request);
        let arc_bindings = request
            .scene
            .nodes
            .iter()
            .filter_map(|node| match node {
                SceneNode::Arc(arc) if !arc.end_binding.is_empty() => {
                    Some(arc.end_binding.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let text_bindings = request
            .scene
            .nodes
            .iter()
            .filter_map(|node| match node {
                SceneNode::Text(text) => match &text.value {
                    SceneValue::Binding(binding) => Some(binding.as_str()),
                    SceneValue::Literal(_) => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(arc_bindings, ["timer.permille"], "{name}");
        assert_eq!(text_bindings, ["timer.remaining:mm:ss"], "{name}");
    }
}

fn generated_array<'a>(source: &'a str, declaration: &str) -> &'a str {
    let after = source
        .split_once(declaration)
        .unwrap_or_else(|| panic!("missing generated-font declaration {declaration}"))
        .1;
    after
        .split_once("\n};")
        .expect("generated-font array terminator")
        .0
}

/// Measures ASCII exactly as the shipped LVGL generated BODY font does:
/// ASCII's format-0 cmap gives `glyph_id = codepoint - 32 + 1`, pair kerning
/// is applied to the current advance, and `fmt_txt` rounds 8.4 advances with
/// `(advance + 8) >> 4`.
fn shipped_body_width(text: &str) -> i32 {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../firmware/main/ui/fonts/deskmate_font_28.c"),
    )
    .expect("read the shipped BODY generated font");
    let advances = generated_array(
        &source,
        "static const lv_font_fmt_txt_glyph_dsc_t glyph_dsc[] = {",
    )
    .lines()
    .filter_map(|line| {
        line.split_once(".adv_w = ")
            .and_then(|(_, rest)| rest.split_once(','))
            .map(|(value, _)| value.parse::<i32>().expect("numeric glyph advance"))
    })
    .collect::<Vec<_>>();
    let pair_ids = generated_array(&source, "static const uint8_t kern_pair_glyph_ids[] =")
        .split(|character: char| !character.is_ascii_digit())
        .filter(|token| !token.is_empty())
        .map(|token| token.parse::<usize>().expect("numeric kerning glyph id"))
        .collect::<Vec<_>>();
    let pair_values = generated_array(&source, "static const int8_t kern_pair_values[] =")
        .split(|character: char| !(character.is_ascii_digit() || character == '-'))
        .filter(|token| !token.is_empty() && *token != "-")
        .map(|token| token.parse::<i32>().expect("numeric kerning value"))
        .collect::<Vec<_>>();
    assert_eq!(pair_ids.len(), pair_values.len() * 2);
    let kerning: HashMap<(usize, usize), i32> = pair_ids
        .as_chunks::<2>()
        .0
        .iter()
        .zip(pair_values)
        .map(|(pair, value)| ((pair[0], pair[1]), value))
        .collect();
    let bytes = text.as_bytes();
    assert!(bytes.iter().all(u8::is_ascii));
    bytes
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            let glyph_id = usize::from(*byte - b' ' + 1);
            let next_id = bytes
                .get(index + 1)
                .map_or(0, |next| usize::from(*next - b' ' + 1));
            let advance = advances[glyph_id] + kerning.get(&(glyph_id, next_id)).unwrap_or(&0);
            (advance + 8) >> 4
        })
        .sum()
}

#[test]
fn produced_nonzero_offset_date_is_wider_than_the_native_content_box() {
    assert_ne!(cases::DATE_OVERFLOW_UTC_OFFSET_MINUTES, 0);
    let produced = cases::date_overflow_text();
    let width = shipped_body_width(&produced);
    assert_eq!(width, cases::DATE_OVERFLOW_BODY_WIDTH);
    assert!(width > cases::DATE_CONTENT_WIDTH);
}

#[test]
fn native_date_binding_ellipsizes_the_produced_overflow() {
    let (_, request) = cases::date_truncation_scene_cases()
        .into_iter()
        .find(|(name, _)| name.ends_with("--landscape"))
        .expect("landscape date-overflow row");
    let mut clipped_request = request.clone();
    let date = clipped_request
        .scene
        .nodes
        .iter_mut()
        .find_map(|node| match node {
            SceneNode::Text(text) if text.value == SceneValue::Binding("date".to_string()) => {
                Some(text)
            }
            _ => None,
        })
        .expect("the real DigitalClock builder emits its date binding");
    assert_eq!(date.w, cases::DATE_CONTENT_WIDTH);
    assert!(date.ellipsize);
    date.ellipsize = false;

    let mut simulator = Simulator::new().expect("simulator");
    let ellipsized = simulator.render_scene(&request).expect("ellipsized scene");
    let clipped = simulator
        .render_scene(&clipped_request)
        .expect("one-line clipped scene");
    let differing = ellipsized
        .iter()
        .zip(&clipped)
        .enumerate()
        .filter_map(|(index, (left, right))| (left != right).then_some(index))
        .collect::<Vec<_>>();
    assert!(
        !differing.is_empty(),
        "the 178px produced date must exercise LV_LABEL_LONG_DOT, not merely set its flag"
    );
    assert!(differing.iter().all(|index| {
        let x = i32::try_from(index % usize::try_from(LOGICAL_WIDTH).unwrap()).unwrap();
        let y = i32::try_from(index / usize::try_from(LOGICAL_WIDTH).unwrap()).unwrap();
        (48..224).contains(&x) && (226..262).contains(&y)
    }));
}

#[test]
fn producer_and_date_scene_goldens_match() {
    let bless = std::env::var_os("BLESS").is_some();
    let golden_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/evidence-scene");
    std::fs::create_dir_all(&golden_dir).expect("golden dir");
    let mut simulator = Simulator::new().expect("simulator");
    let mut failures = Vec::new();
    let mut expected_names = BTreeSet::new();

    for (name, request) in evidence_cases() {
        expected_names.insert(name.clone());
        // This semantic assertion deliberately precedes all rendering and
        // byte comparison: an inverted producer that happened to draw its
        // supplied value correctly must fail before pixels can pass it.
        if request.timer.is_some() {
            assert_timer_producer_meaning(&request);
        }
        let png = simulator.render_scene_png(&request).expect(&name);
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
        let path = entry.expect("golden entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("png") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        if expected_names.contains(stem) {
            continue;
        }
        if bless {
            std::fs::remove_file(&path).expect("remove orphan golden");
        } else {
            failures.push(format!(
                "{stem}: orphan golden (run with BLESS=1 to delete)"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "evidence scene golden mismatches:\n{}",
        failures.join("\n")
    );
}
