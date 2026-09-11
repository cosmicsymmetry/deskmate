//! Native date-overflow evidence rows.

use std::collections::HashMap;

mod common;

use lvgl_sim::{LOGICAL_WIDTH, Simulator, cases};
use protocol::{SceneNode, SceneValue};

fn evidence_cases() -> Vec<(String, lvgl_sim::scene::SceneRenderRequest)> {
    cases::date_truncation_scene_cases()
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
fn date_scene_goldens_match() {
    common::assert_goldens(
        "evidence-scene",
        std::env::var_os("BLESS").is_some(),
        Some("evidence scene golden mismatches"),
        "orphan golden (run with BLESS=1 to delete)",
        evidence_cases(),
        |_, _| {},
        lvgl_sim::Simulator::render_scene_png,
    );
}
