// The panel's visual language, as the faces use it.
//
// These are not new colours. The neutrals and the chromatic roles are `DESIGN.md`'s
// dark scheme -- the same values `scene_build.rs` compiles into the scene-native
// faces -- so a rastered face and a clock face sit on the same ground with the same
// ink. The grid is `DESKMATE_GRID`.
//
// One rule from `DESIGN.md` governs everything below: colour is never the sole
// carrier of a state. A falling token prints a "-" and a downward triangle as well
// as turning red.

/** `DESKMATE_GRID`. */
export const GRID = 8;
/** `DESKMATE_MARGIN`. */
export const MARGIN = 3 * GRID;
/** `DESKMATE_RADIUS_MODULE`. */
export const RADIUS_MODULE = 3 * GRID;
/** Small enough to read as a control rather than as a module. */
export const RADIUS_CHIP = 1.5 * GRID;

export const CANVAS_WIDTH = 448;
export const CANVAS_HEIGHT = 368;
/** The width a face may paint, once both margins are taken. */
export const CONTENT_WIDTH = CANVAS_WIDTH - 2 * MARGIN;

// Neutrals: `DESIGN.md`'s dark column, which is the only column a panel has.

/** `--stage`. Black in both schemes, and an unlit AMOLED pixel emits nothing. */
export const GROUND = "#000000";
/** `--ink`. */
export const INK = "#f5f5f7";
/** `--ink-2`. */
export const INK_2 = "#a0a0a8";
/** `--ink-3`, the label role. */
export const INK_3 = "#84848d";
/** `DESKMATE_COLOR_SURFACE`: the module ground that lifts a panel off black. */
export const SURFACE = "#1a1a1f";
/** A hairline that separates without drawing attention. */
export const HAIRLINE = "#2a2a2f";

// The chromatic roles these faces claim, each with exactly one meaning. `--live`
// and `--warn` are deliberately absent: no face here is "on the panel right now" in
// a way it can know, and staleness is the server's job, not a face's.

/** `--good`: fresh, healthy, rising. */
export const GOOD = "#30d158";
/** `--bad`: fault, falling. */
export const BAD = "#ff453a";

// Type scale: four steps, each clearly a different voice. The hero steps are a
// candidate list because `fitSize` picks the largest that fits.

/** The label voice: uppercase, tracked out, `INK_3`. */
export const SIZE_EYEBROW = 15;
/** Tracking for the eyebrow. Small uppercase needs it to stay legible. */
export const TRACKING_EYEBROW = 1.4;
export const SIZE_CAPTION = 17;
export const SIZE_BODY = 21;
export const SIZE_SUBHEAD = 26;
export const SIZE_TITLE = 31;
/** Candidate sizes for a face's single dominant numeral. */
export const HERO_STEPS = [112, 96, 84, 72, 60] as const;

export const WEIGHT_REGULAR = 400;
export const WEIGHT_SEMIBOLD = 600;

/** Inter's cap height, as a fraction of the em. */
export const CAP_HEIGHT = 0.727;

/**
 * A heading positioned by its cap top rather than its baseline is what keeps the
 * optical margin above a hero numeral equal to the one beside it.
 */
export function baselineFromCapTop(capTop: number, size: number): number {
  return capTop + size * CAP_HEIGHT;
}

/** The vertical centre of a line of type, for aligning small runs against a chip. */
export function baselineFromCenter(center: number, size: number): number {
  return center + (size * CAP_HEIGHT) / 2;
}
