//! Tabular-figure proof for the digital clock's hero time label.
//!
//! `tools/genfonts.sh` bakes Inter's tabular ("tnum") digit forms into the
//! DISPLAY and HERO tiers so a ticking clock does not shimmer: every digit
//! must carry the same advance, so an HH:MM label occupies the same box
//! whatever the time reads. Golden PNGs cannot state that on their own — they
//! pin two frames, not the relationship between them — so this test measures
//! the rendered frames and asserts the relationship.
//!
//! Note the measure. Ink extent is *not* the same as advance: Inter's `1` has
//! wider side bearings than `0`, so "11:35" inks 5px narrower than "00:00"
//! even though both labels are exactly as wide. The assertions below compare
//! quantities that are only equal when the advances are equal:
//!
//! 1. the colon's ink columns across the brief's `11:35` / `00:00` pair —
//!    equal only if the two leading digits advance identically;
//! 2. the total ink span across five times that all start with `1` and end
//!    with `0`, and between them contain all ten digits — the shared outer
//!    glyphs cancel the bearing difference, so any digit with an odd advance
//!    shifts the trailing `0` and changes the span.

#[path = "cases.rs"]
// Only the two pinned instants are used here; the case table itself belongs
// to `golden.rs`.
#[allow(dead_code)]
mod cases;

use lvgl_sim::{
    RenderRequest, SimField, SimFieldValue, SimOrientation, SimTemplate, Simulator, LOGICAL_WIDTH,
};

/// Rows covering the hero time label and nothing else: the title band ends at
/// y = 46 and the date band starts at y = 228 (see `digital_clock.c`).
const TIME_BAND: std::ops::Range<usize> = 120..216;

/// 2025-08-13, at times that all render `1X:Y0` and together use every digit:
/// 16:00, 19:50, 17:30, 18:40, 12:20.
const ALL_DIGIT_INSTANTS: [i64; 5] = [
    1_755_100_800,
    1_755_114_600,
    1_755_106_200,
    1_755_110_400,
    1_755_087_600,
];

fn request(now_unix_seconds: i64) -> RenderRequest {
    RenderRequest {
        template: SimTemplate::DigitalClock,
        fields: vec![
            SimField {
                name: "title".to_string(),
                value: SimFieldValue::Text("Desk".to_string()),
            },
            SimField {
                name: "show_seconds".to_string(),
                value: SimFieldValue::Boolean(false),
            },
        ],
        utc_offset_minutes: 0,
        now_unix_seconds,
        orientation: SimOrientation::Landscape,
    }
}

/// Inclusive column runs of lit pixels across [`TIME_BAND`] — one run per
/// glyph, since the tiers are set with the font's own inter-glyph gaps and no
/// two glyphs of a time touch.
fn ink_runs(pixels: &[u16]) -> Vec<(usize, usize)> {
    let width = LOGICAL_WIDTH as usize;
    let mut runs = Vec::new();
    let mut run_start: Option<usize> = None;
    for column in 0..width {
        let lit = TIME_BAND.clone().any(|row| pixels[row * width + column] != 0);
        match (lit, run_start) {
            (true, None) => run_start = Some(column),
            (false, Some(start)) => {
                runs.push((start, column - 1));
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        runs.push((start, width - 1));
    }
    runs
}

#[test]
fn hero_time_label_is_tabular() {
    let mut sim = Simulator::new().expect("simulator");

    let runs_1135 = ink_runs(&sim.render(&request(cases::TABULAR_1135)).expect("11:35"));
    let runs_0000 = ink_runs(&sim.render(&request(cases::TABULAR_0000)).expect("00:00"));
    assert_eq!(runs_1135.len(), 5, "11:35 did not render five glyphs");
    assert_eq!(runs_0000.len(), 5, "00:00 did not render five glyphs");
    assert_eq!(
        runs_1135[2], runs_0000[2],
        "the colon sits at {:?} in 11:35 but {:?} in 00:00 — the leading \
         digits do not share an advance",
        runs_1135[2], runs_0000[2]
    );

    let spans: Vec<(usize, usize)> = ALL_DIGIT_INSTANTS
        .iter()
        .map(|instant| {
            let runs = ink_runs(&sim.render(&request(*instant)).expect("all-digit instant"));
            (runs[0].0, runs[runs.len() - 1].1)
        })
        .collect();
    for span in &spans {
        assert_eq!(
            *span, spans[0],
            "times sharing their first and last glyph inked {:?} and {:?} — \
             some digit does not share the tabular advance",
            spans[0], span
        );
    }
}
