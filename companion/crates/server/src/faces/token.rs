//! The token face: one price, its direction, and where it has been.
//!
//! # The two-size price
//!
//! The price is set with its currency mark and its fractional part at about
//! 42% of the integer part's size, all on one baseline. That is the
//! conventional ticker treatment and it is not decoration: it lets the digits
//! that actually change size be as large as the canvas allows, while keeping
//! the cents legible rather than dropping them.
//!
//! # Why the sparkline is drawn at full resolution
//!
//! This is the clearest thing rastering buys. A scene-native sparkline would
//! be capped at `protocol::MAX_SCENE_LINE_POINTS` -- 8 points per `SceneLine`
//! node -- so a day of prices would arrive as seven straight segments. Here
//! the whole series is drawn.

use super::svg::{Anchor, Canvas, Text, count};
use std::fmt::Write as _;

use super::theme::{
    BAD, CANVAS_HEIGHT, CANVAS_WIDTH, CONTENT_WIDTH, GOOD, GRID, GROUND, HAIRLINE, HERO_STEPS, INK,
    INK_2, INK_3, MARGIN, RADIUS_CHIP, SIZE_CAPTION, SIZE_EYEBROW, SIZE_SUBHEAD, TRACKING_EYEBROW,
    WEIGHT_REGULAR, WEIGHT_SEMIBOLD, baseline_from_cap_top, baseline_from_center,
};

/// The fractional part and currency mark, relative to the integer part.
const FRACTION_RATIO: f64 = 0.42;
const SPARKLINE_HEIGHT: f64 = 96.0;
/// Headroom above and below the series so a flat line is not drawn on the
/// boundary and a peak is not clipped by the stroke's own width.
const SPARKLINE_PADDING: f64 = 8.0;
const SPARKLINE_STROKE: f64 = 2.5;
const AREA_OPACITY: f64 = 0.16;
const CHIP_HEIGHT: f64 = 32.0;

/// Everything the token face draws. The face does no arithmetic on prices
/// beyond layout: the direction and the percentage arrive decided.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TokenFace {
    /// The ticker, e.g. "SOL". Set as the identity.
    pub symbol: String,
    /// The long name, e.g. "Solana".
    pub name: String,
    /// The quote currency's ISO code, e.g. "USD".
    pub currency: String,
    /// The currency's mark, e.g. "$". Empty falls back to the code.
    pub currency_mark: String,
    pub price: f64,
    /// Change over the window, in percent. Sign carries the direction.
    pub change_percent: f64,
    pub low: f64,
    pub high: f64,
    /// Oldest to newest. Fewer than two points draws no sparkline.
    pub series: Vec<f64>,
}

impl TokenFace {
    fn rising(&self) -> bool {
        self.change_percent >= 0.0
    }

    fn direction_color(&self) -> &'static str {
        if self.rising() { GOOD } else { BAD }
    }
}

pub(crate) fn render(face: &TokenFace) -> String {
    let mut canvas = Canvas::new(CANVAS_WIDTH, CANVAS_HEIGHT);
    canvas.rect(0.0, 0.0, CANVAS_WIDTH, CANVAS_HEIGHT, GROUND);

    draw_identity(&mut canvas, face);
    let price_bottom = draw_price(&mut canvas, face);
    draw_change_chip(&mut canvas, face, price_bottom + 1.75 * GRID);
    draw_sparkline(&mut canvas, face);
    draw_range(&mut canvas, face);

    canvas.finish()
}

fn draw_identity(canvas: &mut Canvas, face: &TokenFace) {
    let baseline = baseline_from_cap_top(MARGIN, SIZE_SUBHEAD);
    let symbol = face.symbol.to_uppercase();
    canvas.text(&Text::new(MARGIN, baseline, &symbol, SIZE_SUBHEAD, INK).weight(WEIGHT_SEMIBOLD));

    let symbol_width = crate::face_render::text_width(&symbol, SIZE_SUBHEAD, WEIGHT_SEMIBOLD);
    let name_x = MARGIN + symbol_width + 1.5 * GRID;
    let currency = face.currency.to_uppercase();
    // Both of these runs are tracked, so both are measured with tracking.
    let currency_width =
        super::svg::tracked_width(&currency, SIZE_EYEBROW, WEIGHT_SEMIBOLD, TRACKING_EYEBROW);
    let name_room = CANVAS_WIDTH - MARGIN - currency_width - 2.0 * GRID - name_x;

    let name = super::svg::normalize_whitespace(&face.name).to_uppercase();
    if !name.is_empty() && name_room > 0.0 {
        let fitted = super::svg::fit_tracked(
            &name,
            SIZE_EYEBROW,
            WEIGHT_SEMIBOLD,
            TRACKING_EYEBROW,
            name_room,
        );
        canvas.text(
            &Text::new(
                name_x,
                // Aligned on the symbol's own baseline, not its box, so the two
                // runs sit on one line despite the size difference.
                baseline,
                &fitted,
                SIZE_EYEBROW,
                INK_3,
            )
            .weight(WEIGHT_SEMIBOLD)
            .tracking(TRACKING_EYEBROW),
        );
    }

    canvas.text(
        &Text::new(
            CANVAS_WIDTH - MARGIN,
            baseline,
            &currency,
            SIZE_EYEBROW,
            INK_3,
        )
        .weight(WEIGHT_SEMIBOLD)
        .tracking(TRACKING_EYEBROW)
        .anchor(Anchor::End),
    );
}

/// Draws the price and returns the y of its cap bottom.
fn draw_price(canvas: &mut Canvas, face: &TokenFace) -> f64 {
    let (integer, fraction) = split_price(face.price);
    let mark = face.currency_mark.clone();
    // Below a whole unit, the integer part is the single character "0" and
    // every digit that matters lives in the fraction. Shrinking the fraction
    // there inverts the emphasis: it sets the one meaningless glyph large and
    // the significant digits small. So a sub-unit price is set at one size.
    let uniform = face.price.abs() < 1.0 && !fraction.is_empty();

    // Fit the whole assembly, not just the integer part: the mark and the
    // fraction are what push a four-digit price over the edge.
    let size = fit_price_size(&mark, &integer, &fraction, uniform);
    let fraction_size = if uniform { size } else { size * FRACTION_RATIO };
    let mark_size = size * FRACTION_RATIO;
    let cap_top = 62.0;
    let baseline = baseline_from_cap_top(cap_top, size);

    // The decimal separator is set at the INTEGER's size, not the fraction's.
    //
    // A period is a few pixels of ink. Set at 42% beside a 112px numeral it
    // disappears, and "$101" followed by a small "96" reads as ten thousand
    // one hundred and ninety-six. Keeping the separator large is what makes
    // the magnitude unambiguous; the digits after it can still be small.
    let (integer_run, fraction_digits) = if uniform {
        (integer.clone(), fraction.clone())
    } else {
        match fraction.strip_prefix('.') {
            Some(digits) => (format!("{integer}."), digits.to_owned()),
            None => (integer.clone(), fraction.clone()),
        }
    };

    let mut x = MARGIN;
    if !mark.is_empty() {
        // Cap-top aligned, not baseline aligned. A currency mark sitting on
        // the baseline of a 112px numeral hangs off the bottom-left corner
        // looking detached; the superior position is what every ticker uses.
        canvas.text(
            &Text::new(
                x,
                baseline_from_cap_top(cap_top, mark_size),
                &mark,
                mark_size,
                INK_2,
            )
            .weight(WEIGHT_SEMIBOLD),
        );
        x += crate::face_render::text_width(&mark, mark_size, WEIGHT_SEMIBOLD) + 2.0;
    }
    canvas.text(&Text::new(x, baseline, &integer_run, size, INK).weight(WEIGHT_SEMIBOLD));
    x += crate::face_render::text_width(&integer_run, size, WEIGHT_SEMIBOLD);
    if !fraction_digits.is_empty() {
        // The fraction stays on the main baseline whichever mode we are in:
        // raised cents would read as a footnote marker.
        let fill = if uniform { INK } else { INK_2 };
        canvas.text(
            &Text::new(x, baseline, &fraction_digits, fraction_size, fill).weight(WEIGHT_SEMIBOLD),
        );
    }

    cap_top + size * 0.727
}

/// The largest step at which the whole assembly fits.
///
/// It measures exactly what [`draw_price`] will draw, separator included and
/// at the size it will be drawn -- otherwise the fit is computed for a
/// different string than the one that appears, which is how a price ends up
/// one glyph past the margin.
fn fit_price_size(mark: &str, integer: &str, fraction: &str, uniform: bool) -> f64 {
    let (integer_run, fraction_digits) = if uniform {
        (integer.to_owned(), fraction.to_owned())
    } else {
        match fraction.strip_prefix('.') {
            Some(digits) => (format!("{integer}."), digits.to_owned()),
            None => (integer.to_owned(), fraction.to_owned()),
        }
    };
    for &candidate in &HERO_STEPS {
        let fraction_size = if uniform {
            candidate
        } else {
            candidate * FRACTION_RATIO
        };
        let mark_size = candidate * FRACTION_RATIO;
        let width = crate::face_render::text_width(mark, mark_size, WEIGHT_SEMIBOLD)
            + crate::face_render::text_width(&integer_run, candidate, WEIGHT_SEMIBOLD)
            + crate::face_render::text_width(&fraction_digits, fraction_size, WEIGHT_SEMIBOLD);
        if width <= CONTENT_WIDTH {
            return candidate;
        }
    }
    HERO_STEPS[HERO_STEPS.len() - 1]
}

fn draw_change_chip(canvas: &mut Canvas, face: &TokenFace, top: f64) {
    let color = face.direction_color();
    // The sign is printed as well as coloured: `DESIGN.md` requires that a
    // state survive a greyscale screenshot, and a red number with no minus is
    // indistinguishable from a green one in that screenshot.
    let label = format!(
        "{}{:.2}%",
        if face.rising() { '+' } else { '-' },
        face.change_percent.abs()
    );
    let text_width = crate::face_render::text_width(&label, SIZE_CAPTION, WEIGHT_SEMIBOLD);
    let triangle_width = 9.0;
    let chip_width = triangle_width + 1.0 * GRID + text_width + 3.0 * GRID;
    let center = top + CHIP_HEIGHT / 2.0;

    canvas.rect_opacity(
        MARGIN,
        top,
        chip_width,
        CHIP_HEIGHT,
        RADIUS_CHIP,
        color,
        AREA_OPACITY,
    );

    let triangle_x = MARGIN + 1.5 * GRID;
    canvas.path(
        &triangle(triangle_x, center, triangle_width, face.rising()),
        color,
    );

    canvas.text(
        &Text::new(
            triangle_x + triangle_width + 1.0 * GRID,
            baseline_from_center(center, SIZE_CAPTION),
            &label,
            SIZE_CAPTION,
            color,
        )
        .weight(WEIGHT_SEMIBOLD),
    );

    canvas.text(
        &Text::new(
            MARGIN + chip_width + 1.5 * GRID,
            baseline_from_center(center, SIZE_EYEBROW),
            "24H",
            SIZE_EYEBROW,
            INK_3,
        )
        .weight(WEIGHT_SEMIBOLD)
        .tracking(TRACKING_EYEBROW),
    );
}

/// An equilateral-ish triangle pointing up or down, centred on `(x, cy)`.
fn triangle(x: f64, cy: f64, width: f64, up: bool) -> String {
    let half = width / 2.0;
    let height = width * 0.86;
    if up {
        format!(
            "M{:.2} {:.2}L{:.2} {:.2}L{:.2} {:.2}Z",
            x,
            cy + height / 2.0,
            x + half,
            cy - height / 2.0,
            x + width,
            cy + height / 2.0,
        )
    } else {
        format!(
            "M{:.2} {:.2}L{:.2} {:.2}L{:.2} {:.2}Z",
            x,
            cy - height / 2.0,
            x + half,
            cy + height / 2.0,
            x + width,
            cy - height / 2.0,
        )
    }
}

const SPARKLINE_TOP: f64 = 190.0;

fn draw_sparkline(canvas: &mut Canvas, face: &TokenFace) {
    if face.series.len() < 2 {
        // Not an error: a freshly configured card has no history yet. A
        // hairline reads as "no series" without pretending to be a flat price.
        canvas.rect(
            MARGIN,
            SPARKLINE_TOP + SPARKLINE_HEIGHT - 1.0,
            CONTENT_WIDTH,
            1.0,
            HAIRLINE,
        );
        return;
    }

    let (minimum, maximum) = face
        .series
        .iter()
        .fold((f64::MAX, f64::MIN), |(low, high), &value| {
            (low.min(value), high.max(value))
        });
    let span = (maximum - minimum).max(f64::EPSILON);
    let plot_height = SPARKLINE_HEIGHT - 2.0 * SPARKLINE_PADDING;
    let step = CONTENT_WIDTH / (count(face.series.len()) - 1.0);

    let point = |index: usize, value: f64| -> (f64, f64) {
        let x = MARGIN + count(index) * step;
        // A perfectly flat series would otherwise sit on the top edge, because
        // every value equals the maximum; centre it instead.
        let normalized = if maximum - minimum <= f64::EPSILON {
            0.5
        } else {
            (value - minimum) / span
        };
        let y = SPARKLINE_TOP + SPARKLINE_PADDING + (1.0 - normalized) * plot_height;
        (x, y)
    };

    let mut stroke = String::with_capacity(face.series.len() * 16);
    for (index, &value) in face.series.iter().enumerate() {
        let (x, y) = point(index, value);
        let command = if index == 0 { 'M' } else { 'L' };
        let _ = write!(stroke, "{command}{x:.2} {y:.2}");
    }

    let color = face.direction_color();
    let bottom = SPARKLINE_TOP + SPARKLINE_HEIGHT;
    let area = format!(
        "{stroke}L{:.2} {bottom:.2}L{:.2} {bottom:.2}Z",
        MARGIN + CONTENT_WIDTH,
        MARGIN,
    );
    canvas.filled_path_opacity(&area, color, AREA_OPACITY);
    canvas.stroked_path(&stroke, color, SPARKLINE_STROKE);

    // The newest point gets a marker, so the eye lands on "now" rather than on
    // whichever peak happens to be tallest.
    if let Some(&last) = face.series.last() {
        let (x, y) = point(face.series.len() - 1, last);
        canvas.circle(x, y, SPARKLINE_STROKE + 2.0, GROUND);
        canvas.circle(x, y, SPARKLINE_STROKE + 0.5, color);
    }
}

fn draw_range(canvas: &mut Canvas, face: &TokenFace) {
    let label_top = 300.0;
    let mark = &face.currency_mark;
    let low = format!("L {mark}{}", compact_price(face.low));
    let high = format!("H {mark}{}", compact_price(face.high));

    canvas.text(
        &Text::new(
            MARGIN,
            baseline_from_cap_top(label_top, SIZE_CAPTION),
            &low,
            SIZE_CAPTION,
            INK_3,
        )
        .weight(WEIGHT_REGULAR),
    );
    canvas.text(
        &Text::new(
            CANVAS_WIDTH - MARGIN,
            baseline_from_cap_top(label_top, SIZE_CAPTION),
            &high,
            SIZE_CAPTION,
            INK_3,
        )
        .weight(WEIGHT_REGULAR)
        .anchor(Anchor::End),
    );

    // Where the current price sits inside the window's range. This is the one
    // fact the sparkline does not state plainly.
    let track_y = label_top + SIZE_CAPTION * 0.727 + 1.5 * GRID;
    canvas.rounded_rect(MARGIN, track_y, CONTENT_WIDTH, 3.0, 1.5, HAIRLINE);
    let span = face.high - face.low;
    if span > f64::EPSILON {
        let fraction = ((face.price - face.low) / span).clamp(0.0, 1.0);
        canvas.circle(
            MARGIN + fraction * CONTENT_WIDTH,
            track_y + 1.5,
            4.5,
            face.direction_color(),
        );
    }
}

/// Splits a price into its integer part (with thousands separators) and its
/// fractional part (including the decimal point), choosing the number of
/// decimals from the magnitude.
///
/// A token quoted at 0.00004182 and one quoted at 142.37 both have to be
/// readable on the same face, and two decimals would render the first as
/// "0.00". The breakpoints mirror what an exchange ticker does.
fn split_price(price: f64) -> (String, String) {
    let magnitude = price.abs();
    let decimals = if magnitude >= 1.0 {
        2
    } else if magnitude >= 0.01 {
        4
    } else if magnitude > 0.0 {
        8
    } else {
        2
    };

    let rendered = format!("{magnitude:.decimals$}");
    let (integer, fraction) = rendered
        .split_once('.')
        .map_or((rendered.as_str(), ""), |(head, tail)| (head, tail));
    // "0.00003710" states a precision the quote does not have. Only the
    // high-precision branches are trimmed: turning $142.50 into $142.5 would
    // make a plain price look like a rounding mistake.
    let fraction = if decimals > 2 {
        fraction.trim_end_matches('0')
    } else {
        fraction
    };
    let sign = if price < 0.0 { "-" } else { "" };
    let grouped = format!("{sign}{}", group_thousands(integer));
    if fraction.is_empty() {
        (grouped, String::new())
    } else {
        (grouped, format!(".{fraction}"))
    }
}

/// The same number on one line, for the range labels.
fn compact_price(price: f64) -> String {
    let (integer, fraction) = split_price(price);
    format!("{integer}{fraction}")
}

fn group_thousands(digits: &str) -> String {
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face() -> TokenFace {
        TokenFace {
            symbol: "SOL".to_owned(),
            name: "Solana".to_owned(),
            currency: "USD".to_owned(),
            currency_mark: "$".to_owned(),
            price: 142.37,
            change_percent: 2.41,
            low: 138.02,
            high: 147.6,
            series: (0..96)
                .map(|index| 140.0 + (f64::from(index) / 9.0).sin() * 4.0)
                .collect(),
        }
    }

    #[test]
    fn the_face_renders_with_its_price_and_direction() {
        let svg = render(&face());
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
        assert!(svg.contains("SOL"));
        assert!(svg.contains("142"), "the integer part is set large");
        assert!(svg.contains(".37"), "the cents are kept");
        assert!(svg.contains("+2.41%"));
        assert!(svg.contains(">24H<"));
        assert!(svg.contains(GOOD), "a rise is the good role");
    }

    #[test]
    fn a_fall_prints_its_sign_as_well_as_its_colour() {
        let mut falling = face();
        falling.change_percent = -3.08;
        let svg = render(&falling);
        assert!(
            svg.contains("-3.08%"),
            "the state survives a greyscale screenshot"
        );
        assert!(svg.contains(BAD));
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }

    #[test]
    fn the_decimal_separator_is_set_at_the_integers_size() {
        // The regression this guards: a 42%-size period beside a 112px
        // numeral is invisible, and "$101" + small "96" reads as 10,196.
        let svg = render(&face());
        assert!(
            svg.contains(">142.<"),
            "the separator rides with the integer run: {svg}"
        );
        assert!(svg.contains(">37<"), "the cents are their own smaller run");
    }

    #[test]
    fn a_sub_cent_price_keeps_one_size_and_its_own_separator() {
        let mut small = face();
        small.price = 0.000_041_82;
        let svg = render(&small);
        assert!(
            svg.contains(">.00004182<"),
            "a uniform price keeps the separator in the fraction run"
        );
    }

    #[test]
    fn a_sub_cent_token_keeps_significant_digits() {
        let (integer, fraction) = split_price(0.000_041_82);
        assert_eq!(integer, "0");
        assert_eq!(
            fraction, ".00004182",
            "two decimals would have rendered this as 0.00"
        );
        // Trailing zeros are trimmed: the quote has no precision there.
        assert_eq!(split_price(0.000_037_1).1, ".0000371");
    }

    #[test]
    fn a_large_price_is_grouped_and_still_fits_the_canvas() {
        let (integer, fraction) = split_price(104_235.5);
        assert_eq!(integer, "104,235");
        assert_eq!(fraction, ".50");
        let mut rich = face();
        rich.price = 104_235.5;
        let svg = render(&rich);
        assert!(svg.contains("104,235"));
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }

    #[test]
    fn the_price_size_shrinks_rather_than_overflowing() {
        let wide = fit_price_size("$", "104,235", ".50", false);
        let narrow = fit_price_size("$", "14", ".37", false);
        assert!(
            narrow >= wide,
            "a short price is set at least as large: {narrow} vs {wide}"
        );
    }

    #[test]
    fn a_flat_series_is_centred_rather_than_drawn_on_the_edge() {
        let mut flat = face();
        flat.series = vec![100.0; 40];
        let svg = render(&flat);
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
        // Centred means the stroke sits at the plot's middle, not its top.
        let middle = SPARKLINE_TOP + SPARKLINE_HEIGHT / 2.0;
        assert!(
            svg.contains(&format!("{middle:.2}")),
            "the flat series is drawn at the plot centre"
        );
    }

    #[test]
    fn too_short_a_series_draws_a_baseline_rather_than_a_broken_path() {
        for series in [vec![], vec![142.0]] {
            let mut sparse = face();
            sparse.series = series;
            let svg = render(&sparse);
            assert!(crate::face_render::frame_from_svg(&svg).is_ok());
            // The change chip's direction triangle is also a path, so the
            // sparkline is identified by its stroke instead.
            assert!(
                !svg.contains(r#"fill="none""#),
                "no sparkline stroke is emitted"
            );
        }
    }

    #[test]
    fn the_series_is_drawn_at_full_resolution_rather_than_decimated() {
        // The whole reason this face is rastered instead of scene-native.
        let svg = render(&face());
        let stroke = svg
            .split(r#"<path d=""#)
            .nth(2)
            .expect("the stroke path is present");
        let commands = stroke.matches('L').count();
        assert!(
            commands >= 90,
            "all 96 samples reach the path, not protocol::MAX_SCENE_LINE_POINTS' 8: {commands}"
        );
    }

    #[test]
    fn a_zero_width_range_omits_the_marker_instead_of_dividing_by_zero() {
        let mut degenerate = face();
        degenerate.low = 142.37;
        degenerate.high = 142.37;
        let svg = render(&degenerate);
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }

    #[test]
    fn a_hostile_name_cannot_escape_its_text_node() {
        let mut hostile = face();
        hostile.name = r"<script>alert(1)</script>".to_owned();
        hostile.symbol = r"&<>".to_owned();
        let svg = render(&hostile);
        assert!(!svg.contains("<script>"));
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }
}
