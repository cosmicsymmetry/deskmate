//! The panel's visual language, as the faces use it.
//!
//! These are not new colours. The neutrals and the chromatic roles are
//! `DESIGN.md`'s dark scheme -- the same values `scene_build.rs` compiles into
//! the scene-native faces -- so a rastered face and a clock face sit on the
//! same ground with the same ink. The grid is `DESKMATE_GRID`.
//!
//! One rule from `DESIGN.md` governs everything below: **colour is never the
//! sole carrier of a state.** A falling token prints a "-" and a downward
//! triangle as well as turning red; a stale face prints the word.

/// `DESKMATE_GRID`.
pub(crate) const GRID: f64 = 8.0;
/// `DESKMATE_MARGIN`.
pub(crate) const MARGIN: f64 = 3.0 * GRID;
/// `DESKMATE_RADIUS_MODULE`.
pub(crate) const RADIUS_MODULE: f64 = 3.0 * GRID;
/// The radius a chip or pill gets, small enough to read as a control rather
/// than as a module.
pub(crate) const RADIUS_CHIP: f64 = 1.5 * GRID;

pub(crate) const CANVAS_WIDTH: f64 = 448.0;
pub(crate) const CANVAS_HEIGHT: f64 = 368.0;
/// The width a face may paint, once both margins are taken.
pub(crate) const CONTENT_WIDTH: f64 = CANVAS_WIDTH - 2.0 * MARGIN;

// ---------------------------------------------------------------------------
// Neutrals. `DESIGN.md`'s dark column, which is the only column a panel has.
// ---------------------------------------------------------------------------

/// `--stage`. Black in both schemes, and an unlit AMOLED pixel emits nothing.
pub(crate) const GROUND: &str = "#000000";
/// `--ink`.
pub(crate) const INK: &str = "#f5f5f7";
/// `--ink-2`.
pub(crate) const INK_2: &str = "#a0a0a8";
/// `--ink-3`, the label role.
pub(crate) const INK_3: &str = "#84848d";
/// `DESKMATE_COLOR_SURFACE`: the module ground that lifts a panel off black.
pub(crate) const SURFACE: &str = "#1a1a1f";
/// A hairline that separates without drawing attention.
pub(crate) const HAIRLINE: &str = "#2a2a2f";

// ---------------------------------------------------------------------------
// The chromatic roles these faces claim, each with exactly one meaning.
//
// `--live` and `--warn` are deliberately absent. No face here is "on the panel
// right now" in a way it can know, and staleness is the shared footer's job,
// not a face's -- so importing those two roles would be a claim the design
// does not make. `DESIGN.md` has the full set when a face needs one.
// ---------------------------------------------------------------------------

/// `--good`: fresh, healthy, rising.
pub(crate) const GOOD: &str = "#30d158";
/// `--bad`: fault, falling.
pub(crate) const BAD: &str = "#ff453a";

// ---------------------------------------------------------------------------
// Type scale.
//
// The sizes are the app's own scale converted to panel pixels, and they are
// deliberately coarse: four steps, each clearly a different voice. The hero
// steps are candidate lists rather than single values because `svg::fit_size`
// picks the largest that fits, so a two-digit temperature is set larger than a
// four-digit price without either being clipped.
// ---------------------------------------------------------------------------

/// The label voice: uppercase, tracked out, `INK_3`.
pub(crate) const SIZE_EYEBROW: f64 = 15.0;
/// Tracking for the eyebrow. Small uppercase needs it to stay legible.
pub(crate) const TRACKING_EYEBROW: f64 = 1.4;
pub(crate) const SIZE_CAPTION: f64 = 17.0;
pub(crate) const SIZE_BODY: f64 = 21.0;
pub(crate) const SIZE_SUBHEAD: f64 = 26.0;
pub(crate) const SIZE_TITLE: f64 = 31.0;
/// Candidate sizes for a face's single dominant numeral.
pub(crate) const HERO_STEPS: [f64; 5] = [112.0, 96.0, 84.0, 72.0, 60.0];

pub(crate) const WEIGHT_REGULAR: u16 = 400;
pub(crate) const WEIGHT_SEMIBOLD: u16 = 600;

/// Inter's cap height is about 0.727em and its ascender about 0.969em. A
/// heading positioned by its cap top rather than its baseline is what keeps
/// the optical margin above a hero numeral equal to the one beside it, so the
/// faces place large type this way.
pub(crate) fn baseline_from_cap_top(cap_top: f64, size: f64) -> f64 {
    cap_top + size * 0.727
}

/// The vertical centre of a line of type, for aligning small runs against an
/// icon or a chip.
pub(crate) fn baseline_from_center(center: f64, size: f64) -> f64 {
    center + size * 0.727 / 2.0
}
