//! The weather face: a condition-coloured hero over an hourly strip.
//!
//! # Where the Pixel Weather influence actually lands
//!
//! Three things, and deliberately not a fourth. The face is **coloured by its
//! condition** rather than being one neutral template with a swapped icon; the
//! illustration is **geometric and flat** rather than photographic; and the
//! hourly forecast is a **strip of equal columns** rather than a chart with
//! axes.
//!
//! What is not borrowed is the full-bleed saturated background. This panel is
//! emissive and `DESIGN.md` pins `--stage` to black in both schemes because an
//! unlit pixel emits nothing -- a 448x368 field of bright colour is a desk lamp.
//! So the colour is contained in a rounded hero module sitting on black, which
//! is also how every other surface in this product is built.
//!
//! # Why the illustrations are drawn rather than shipped as assets
//!
//! Every glyph here is circles, ellipses and paths parameterized by one scale,
//! so the same code draws the 56px hero sun and the 16px strip sun. Shipping
//! them as images would mean an asset per condition per size, each one a
//! durable asset competing for the device's `MAX_ASSET_DIGESTS` keep-set
//! budget, to draw shapes that cost a dozen bytes of path data.

use super::svg::{Anchor, Canvas, Text, count, fit};
use super::theme::{
    CANVAS_HEIGHT, CANVAS_WIDTH, CONTENT_WIDTH, GRID, GROUND, HERO_STEPS, INK, INK_3, MARGIN,
    RADIUS_MODULE, SIZE_BODY, SIZE_CAPTION, SIZE_EYEBROW, SIZE_SUBHEAD, SURFACE, TRACKING_EYEBROW,
    WEIGHT_SEMIBOLD, baseline_from_cap_top, baseline_from_center,
};

const HERO_TOP: f64 = MARGIN;
const HERO_HEIGHT: f64 = 200.0;
const HERO_BOTTOM: f64 = HERO_TOP + HERO_HEIGHT;
const STRIP_TOP: f64 = HERO_BOTTOM + 2.0 * GRID;
const STRIP_BOTTOM: f64 = CANVAS_HEIGHT - MARGIN;
const STRIP_HEIGHT: f64 = STRIP_BOTTOM - STRIP_TOP;
/// Six columns is what fits at a legible size across 400px.
pub(super) const STRIP_COLUMNS: usize = 6;
const HERO_GLYPH_SCALE: f64 = 52.0;
const STRIP_GLYPH_SCALE: f64 = 15.0;

/// The weather conditions this face distinguishes.
///
/// Coarser than the WMO code list on purpose: "moderate drizzle" and "dense
/// drizzle" are the same picture and the same decision about a coat, so they
/// share a variant. The summary text carries whatever nuance the provider
/// reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Condition {
    ClearDay,
    ClearNight,
    PartlyCloudyDay,
    PartlyCloudyNight,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Sleet,
    Snow,
    Thunderstorm,
}

/// One condition's ground and accent.
///
/// The grounds are deep rather than saturated, which is the adaptation an
/// emissive panel forces: the hero is a large area, and a bright one would
/// dominate a dim room and cost real power. The accents are bright, because
/// they are small.
struct Palette {
    ground: &'static str,
    accent: &'static str,
    /// Secondary ink inside the hero, tuned per ground so the place name and
    /// the high/low stay at a readable contrast on each of them.
    muted: &'static str,
}

impl Condition {
    pub(crate) const fn from_wmo(code: u16, is_day: bool) -> Self {
        match code {
            0 | 1 if is_day => Self::ClearDay,
            0 | 1 => Self::ClearNight,
            2 if is_day => Self::PartlyCloudyDay,
            2 => Self::PartlyCloudyNight,
            45 | 48 => Self::Fog,
            51 | 53 | 55 => Self::Drizzle,
            56 | 57 | 66 | 67 => Self::Sleet,
            61 | 63 | 65 | 80 | 81 | 82 => Self::Rain,
            71 | 73 | 75 | 77 | 85 | 86 => Self::Snow,
            95 | 96 | 99 => Self::Thunderstorm,
            // Code 3 (overcast) lands here, and so does anything we do not
            // recognize: the
            // summary text still says what the provider reported, and drawing
            // a sun for a code we cannot read would be a lie.
            _ => Self::Cloudy,
        }
    }

    const fn palette(self) -> Palette {
        match self {
            Self::ClearDay => Palette {
                ground: "#0e3a5c",
                accent: "#ffc24d",
                muted: "#a9c6dc",
            },
            Self::ClearNight => Palette {
                ground: "#0c1330",
                accent: "#dce3f5",
                muted: "#9aa4c4",
            },
            Self::PartlyCloudyDay => Palette {
                ground: "#123249",
                accent: "#ffc24d",
                muted: "#a9c0d0",
            },
            Self::PartlyCloudyNight => Palette {
                ground: "#10182f",
                accent: "#dce3f5",
                muted: "#9aa4c4",
            },
            Self::Cloudy => Palette {
                ground: "#24282f",
                accent: "#c6cbd6",
                muted: "#9ba1ac",
            },
            Self::Fog => Palette {
                ground: "#22272b",
                accent: "#aeb6bd",
                muted: "#97a0a7",
            },
            Self::Drizzle => Palette {
                ground: "#13293d",
                accent: "#8fc4ea",
                muted: "#9db6c9",
            },
            Self::Rain => Palette {
                ground: "#102a3d",
                accent: "#6fb3e0",
                muted: "#9ab3c6",
            },
            Self::Sleet => Palette {
                ground: "#17293a",
                accent: "#9fc7e6",
                muted: "#a2b4c2",
            },
            Self::Snow => Palette {
                ground: "#1a2833",
                accent: "#e8f2ff",
                muted: "#a6b5c1",
            },
            Self::Thunderstorm => Palette {
                ground: "#1a1526",
                accent: "#ffd34d",
                muted: "#a79fb8",
            },
        }
    }
}

/// One hour in the strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HourlyStep {
    /// Already formatted in the panel's own timezone, e.g. "14" or "2PM".
    pub label: String,
    pub temperature: i32,
    pub condition: Condition,
}

/// Everything the weather face draws. Temperatures arrive rounded and in the
/// configured unit; the face does not convert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WeatherFace {
    pub place: String,
    pub temperature: i32,
    pub summary: String,
    pub condition: Condition,
    pub high: i32,
    pub low: i32,
    pub hourly: Vec<HourlyStep>,
}

pub(crate) fn render(face: &WeatherFace) -> String {
    let palette = face.condition.palette();
    let mut canvas = Canvas::new(CANVAS_WIDTH, CANVAS_HEIGHT);
    canvas.rect(0.0, 0.0, CANVAS_WIDTH, CANVAS_HEIGHT, GROUND);

    draw_hero(&mut canvas, face, &palette);
    draw_strip(&mut canvas, face);
    canvas.finish()
}

fn draw_hero(canvas: &mut Canvas, face: &WeatherFace, palette: &Palette) {
    canvas.rounded_rect(
        MARGIN,
        HERO_TOP,
        CONTENT_WIDTH,
        HERO_HEIGHT,
        RADIUS_MODULE,
        palette.ground,
    );

    let inset = MARGIN + 2.5 * GRID;
    // The glyph owns the right of the hero; the type gets what is left of it.
    let glyph_center_x = MARGIN + CONTENT_WIDTH - HERO_GLYPH_SCALE - 2.5 * GRID;
    let type_room = glyph_center_x - HERO_GLYPH_SCALE - inset - GRID;

    // The high/low sits on the eyebrow's line, hard right.
    //
    // The hero has no room for a third stacked row at the largest reading size.
    // Sharing the eyebrow line keeps the range visible and reads as context for
    // the reading rather than as a state.
    let range = format!("H {}\u{b0}   L {}\u{b0}", face.high, face.low);
    let range_width = crate::face_render::text_width(&range, SIZE_CAPTION, WEIGHT_SEMIBOLD);
    let eyebrow_baseline = baseline_from_cap_top(HERO_TOP + 2.5 * GRID, SIZE_EYEBROW);
    canvas.text(
        &Text::new(
            MARGIN + CONTENT_WIDTH - 2.5 * GRID,
            eyebrow_baseline,
            &range,
            SIZE_CAPTION,
            INK,
        )
        .weight(WEIGHT_SEMIBOLD)
        .anchor(Anchor::End)
        .opacity(0.82),
    );

    let place_room = CONTENT_WIDTH - 5.0 * GRID - range_width - 2.0 * GRID;
    let place = super::svg::normalize_whitespace(&face.place).to_uppercase();
    // Tracked, so measured with tracking. Otherwise a long place name can
    // measure as fitted while drawing straight through the high/low beside it.
    let place = super::svg::fit_tracked(
        &place,
        SIZE_EYEBROW,
        WEIGHT_SEMIBOLD,
        TRACKING_EYEBROW,
        place_room,
    );
    if !place.is_empty() {
        canvas.text(
            &Text::new(inset, eyebrow_baseline, &place, SIZE_EYEBROW, palette.muted)
                .weight(WEIGHT_SEMIBOLD)
                .tracking(TRACKING_EYEBROW),
        );
    }

    // The degree sign rides with the number: "14 °" reads as a stray mark.
    let reading = format!("{}\u{b0}", face.temperature);
    let size = super::svg::fit_size(&reading, WEIGHT_SEMIBOLD, type_room, &HERO_STEPS);
    let reading_cap_top = HERO_TOP + 6.5 * GRID;
    canvas.text(
        &Text::new(
            inset,
            baseline_from_cap_top(reading_cap_top, size),
            &reading,
            size,
            INK,
        )
        .weight(WEIGHT_SEMIBOLD),
    );

    let summary_top = reading_cap_top + size * 0.727 + 1.5 * GRID;
    let summary = super::svg::normalize_whitespace(&face.summary);
    let summary = fit(&summary, SIZE_SUBHEAD, WEIGHT_SEMIBOLD, type_room);
    if !summary.is_empty() {
        canvas.text(
            &Text::new(
                inset,
                baseline_from_cap_top(summary_top, SIZE_SUBHEAD),
                &summary,
                SIZE_SUBHEAD,
                INK,
            )
            .weight(WEIGHT_SEMIBOLD)
            .opacity(0.9),
        );
    }

    draw_glyph(
        canvas,
        glyph_center_x,
        // Centred on the reading rather than on the module, so the two heaviest
        // objects on the face sit on one optical line.
        reading_cap_top + size * 0.727 / 2.0,
        HERO_GLYPH_SCALE,
        face.condition,
        palette,
    );
}

fn draw_strip(canvas: &mut Canvas, face: &WeatherFace) {
    canvas.rounded_rect(
        MARGIN,
        STRIP_TOP,
        CONTENT_WIDTH,
        STRIP_HEIGHT,
        RADIUS_MODULE,
        SURFACE,
    );

    if face.hourly.is_empty() {
        canvas.text(
            &Text::new(
                CANVAS_WIDTH / 2.0,
                baseline_from_center(STRIP_TOP + STRIP_HEIGHT / 2.0, SIZE_CAPTION),
                "Hourly forecast unavailable",
                SIZE_CAPTION,
                INK_3,
            )
            .anchor(Anchor::Middle),
        );
        return;
    }

    let columns = face.hourly.len().min(STRIP_COLUMNS);
    let column_width = CONTENT_WIDTH / count(columns);

    for (index, step) in face.hourly.iter().take(columns).enumerate() {
        let center_x = MARGIN + (count(index) + 0.5) * column_width;

        canvas.text(
            &Text::new(
                center_x,
                baseline_from_cap_top(STRIP_TOP + 2.0 * GRID, SIZE_EYEBROW),
                &step.label,
                SIZE_EYEBROW,
                INK_3,
            )
            .weight(WEIGHT_SEMIBOLD)
            .anchor(Anchor::Middle),
        );

        draw_glyph(
            canvas,
            center_x,
            STRIP_TOP + STRIP_HEIGHT * 0.5,
            STRIP_GLYPH_SCALE,
            step.condition,
            &step.condition.palette(),
        );

        canvas.text(
            &Text::new(
                center_x,
                baseline_from_cap_top(STRIP_BOTTOM - 2.0 * GRID - SIZE_BODY * 0.727, SIZE_BODY),
                &format!("{}\u{b0}", step.temperature),
                SIZE_BODY,
                INK,
            )
            .weight(WEIGHT_SEMIBOLD)
            .anchor(Anchor::Middle),
        );
    }
}

/// Draws one condition's illustration centred on `(cx, cy)`.
///
/// `scale` is the glyph's nominal radius, so every coordinate below is a
/// fraction of it and the same code serves the hero and the strip.
fn draw_glyph(
    canvas: &mut Canvas,
    cx: f64,
    cy: f64,
    scale: f64,
    condition: Condition,
    palette: &Palette,
) {
    match condition {
        Condition::ClearDay => draw_sun(canvas, cx, cy, scale, palette.accent),
        Condition::ClearNight => draw_moon(canvas, cx, cy, scale, palette.accent, palette.ground),
        Condition::PartlyCloudyDay => {
            draw_sun(
                canvas,
                cx + scale * 0.34,
                cy - scale * 0.42,
                scale * 0.62,
                palette.accent,
            );
            draw_cloud(
                canvas,
                cx - scale * 0.1,
                cy + scale * 0.22,
                scale * 0.92,
                palette.accent,
            );
        }
        Condition::PartlyCloudyNight => {
            draw_moon(
                canvas,
                cx + scale * 0.36,
                cy - scale * 0.44,
                scale * 0.56,
                palette.accent,
                palette.ground,
            );
            draw_cloud(
                canvas,
                cx - scale * 0.1,
                cy + scale * 0.22,
                scale * 0.92,
                palette.accent,
            );
        }
        Condition::Cloudy => draw_cloud(canvas, cx, cy, scale, palette.accent),
        Condition::Fog => draw_fog(canvas, cx, cy, scale, palette.accent),
        Condition::Drizzle => {
            draw_cloud(canvas, cx, cy - scale * 0.26, scale * 0.9, palette.accent);
            draw_precipitation(canvas, cx, cy + scale * 0.62, scale, palette.accent, 0.36);
        }
        Condition::Rain => {
            draw_cloud(canvas, cx, cy - scale * 0.26, scale * 0.9, palette.accent);
            draw_precipitation(canvas, cx, cy + scale * 0.62, scale, palette.accent, 0.58);
        }
        Condition::Sleet => {
            draw_cloud(canvas, cx, cy - scale * 0.26, scale * 0.9, palette.accent);
            draw_precipitation(canvas, cx, cy + scale * 0.6, scale, palette.accent, 0.42);
            draw_flakes(canvas, cx, cy + scale * 0.66, scale, palette.accent, 1);
        }
        Condition::Snow => {
            draw_cloud(canvas, cx, cy - scale * 0.26, scale * 0.9, palette.accent);
            draw_flakes(canvas, cx, cy + scale * 0.64, scale, palette.accent, 3);
        }
        Condition::Thunderstorm => {
            draw_cloud(canvas, cx, cy - scale * 0.28, scale * 0.9, palette.accent);
            draw_bolt(canvas, cx, cy + scale * 0.62, scale, palette.accent);
        }
    }
}

fn draw_sun(canvas: &mut Canvas, cx: f64, cy: f64, scale: f64, color: &str) {
    let core = scale * 0.46;
    canvas.circle(cx, cy, core, color);
    // Eight rays, drawn as a rotated cross pair so the glyph reads as a sun
    // rather than as a dot at strip scale.
    let inner = core + scale * 0.2;
    let outer = scale * 0.98;
    let stroke = (scale * 0.12).max(1.5);
    for index in 0..8 {
        let angle = f64::from(index) * std::f64::consts::FRAC_PI_4;
        let (sin, cos) = angle.sin_cos();
        canvas.line(
            cx + cos * inner,
            cy + sin * inner,
            cx + cos * outer,
            cy + sin * outer,
            color,
            stroke,
        );
    }
}

/// A crescent carved by overlaying a ground-coloured disc.
///
/// This is why the glyph takes the ground colour as well as the accent: the
/// carve has to match whatever it sits on, and inside the hero that is the
/// condition's ground rather than the canvas's black.
fn draw_moon(canvas: &mut Canvas, cx: f64, cy: f64, scale: f64, color: &str, ground: &str) {
    canvas.circle(cx, cy, scale * 0.78, color);
    canvas.circle(cx + scale * 0.36, cy - scale * 0.26, scale * 0.7, ground);
}

fn draw_cloud(canvas: &mut Canvas, cx: f64, cy: f64, scale: f64, color: &str) {
    // Three lobes over a slab: the cheapest shape that still reads as a cloud
    // at 15px.
    canvas.circle(cx - scale * 0.42, cy + scale * 0.04, scale * 0.36, color);
    canvas.circle(cx - scale * 0.04, cy - scale * 0.24, scale * 0.46, color);
    canvas.circle(cx + scale * 0.44, cy + scale * 0.02, scale * 0.34, color);
    canvas.rounded_rect(
        cx - scale * 0.78,
        cy + scale * 0.02,
        scale * 1.56,
        scale * 0.4,
        scale * 0.2,
        color,
    );
}

fn draw_precipitation(canvas: &mut Canvas, cx: f64, cy: f64, scale: f64, color: &str, length: f64) {
    let stroke = (scale * 0.13).max(1.5);
    for offset in [-0.44, 0.0, 0.44] {
        let x = cx + scale * offset;
        canvas.line(
            x + scale * 0.06,
            cy - scale * length / 2.0,
            x - scale * 0.06,
            cy + scale * length / 2.0,
            color,
            stroke,
        );
    }
}

fn draw_flakes(canvas: &mut Canvas, cx: f64, cy: f64, scale: f64, color: &str, count: usize) {
    let radius = (scale * 0.11).max(1.2);
    let offsets: &[f64] = match count {
        1 => &[0.44],
        2 => &[-0.34, 0.34],
        _ => &[-0.46, 0.0, 0.46],
    };
    for &offset in offsets {
        canvas.circle(cx + scale * offset, cy, radius, color);
    }
}

fn draw_bolt(canvas: &mut Canvas, cx: f64, cy: f64, scale: f64, color: &str) {
    let width = scale * 0.36;
    let height = scale * 0.62;
    let definition = format!(
        "M{:.2} {:.2}L{:.2} {:.2}L{:.2} {:.2}L{:.2} {:.2}L{:.2} {:.2}L{:.2} {:.2}Z",
        cx + width * 0.5,
        cy - height / 2.0,
        cx - width * 0.55,
        cy + height * 0.1,
        cx - width * 0.05,
        cy + height * 0.1,
        cx - width * 0.45,
        cy + height / 2.0,
        cx + width * 0.62,
        cy - height * 0.08,
        cx + width * 0.05,
        cy - height * 0.08,
    );
    canvas.path(&definition, color);
}

fn draw_fog(canvas: &mut Canvas, cx: f64, cy: f64, scale: f64, color: &str) {
    let bar_height = (scale * 0.16).max(2.0);
    let gap = scale * 0.34;
    for (index, width) in [1.5_f64, 1.2, 1.6, 1.1].into_iter().enumerate() {
        let y = cy - gap * 1.5 + gap * count(index);
        let bar_width = scale * width;
        canvas.rounded_rect(
            cx - bar_width / 2.0,
            y - bar_height / 2.0,
            bar_width,
            bar_height,
            bar_height / 2.0,
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hour(label: &str, temperature: i32, condition: Condition) -> HourlyStep {
        HourlyStep {
            label: label.to_owned(),
            temperature,
            condition,
        }
    }

    fn face() -> WeatherFace {
        WeatherFace {
            place: "Dubai".to_owned(),
            temperature: 34,
            summary: "Mostly clear".to_owned(),
            condition: Condition::ClearDay,
            high: 38,
            low: 27,
            hourly: vec![
                hour("14", 34, Condition::ClearDay),
                hour("15", 35, Condition::ClearDay),
                hour("16", 34, Condition::PartlyCloudyDay),
                hour("17", 32, Condition::PartlyCloudyDay),
                hour("18", 30, Condition::Cloudy),
                hour("19", 29, Condition::ClearNight),
            ],
        }
    }

    #[test]
    fn the_face_renders_with_its_reading_and_place() {
        let svg = render(&face());
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
        assert!(svg.contains("DUBAI"));
        assert!(
            svg.contains("34\u{b0}"),
            "the reading carries its degree sign"
        );
        assert!(svg.contains("Mostly clear"));
        assert!(svg.contains("H 38\u{b0}"));
    }

    #[test]
    fn every_condition_renders_a_glyph_and_a_parseable_document() {
        // The glyph code is the only part of this face with per-variant
        // branches, so every branch gets rasterized at both scales.
        for condition in [
            Condition::ClearDay,
            Condition::ClearNight,
            Condition::PartlyCloudyDay,
            Condition::PartlyCloudyNight,
            Condition::Cloudy,
            Condition::Fog,
            Condition::Drizzle,
            Condition::Rain,
            Condition::Sleet,
            Condition::Snow,
            Condition::Thunderstorm,
        ] {
            let mut subject = face();
            subject.condition = condition;
            subject.hourly = vec![hour("09", 12, condition); STRIP_COLUMNS];
            let svg = render(&subject);
            assert!(
                crate::face_render::frame_from_svg(&svg).is_ok(),
                "{condition:?} produced an unrenderable face"
            );
            assert!(
                svg.contains(condition.palette().ground),
                "{condition:?} did not colour its hero"
            );
        }
    }

    #[test]
    fn the_wmo_mapping_respects_day_and_night_and_falls_back_to_overcast() {
        assert_eq!(Condition::from_wmo(0, true), Condition::ClearDay);
        assert_eq!(Condition::from_wmo(0, false), Condition::ClearNight);
        assert_eq!(Condition::from_wmo(2, true), Condition::PartlyCloudyDay);
        assert_eq!(Condition::from_wmo(65, true), Condition::Rain);
        assert_eq!(Condition::from_wmo(75, false), Condition::Snow);
        assert_eq!(Condition::from_wmo(95, true), Condition::Thunderstorm);
        assert_eq!(Condition::from_wmo(48, true), Condition::Fog);
        // An unknown code must not be drawn as a sun.
        assert_eq!(Condition::from_wmo(4_242, true), Condition::Cloudy);
    }

    #[test]
    fn a_freezing_code_is_sleet_rather_than_rain_or_snow() {
        for code in [56, 57, 66, 67] {
            assert_eq!(Condition::from_wmo(code, true), Condition::Sleet);
        }
    }

    #[test]
    fn a_sub_zero_reading_fits_alongside_its_sign() {
        let mut cold = face();
        cold.temperature = -18;
        cold.low = -24;
        cold.high = -9;
        cold.condition = Condition::Snow;
        let svg = render(&cold);
        assert!(svg.contains("-18\u{b0}"));
        assert!(svg.contains("L -24\u{b0}"));
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }

    #[test]
    fn a_long_place_name_is_ellipsized_rather_than_running_under_the_glyph() {
        let mut long = face();
        long.place = "Llanfairpwllgwyngyllgogerychwyrndrobwllllantysiliogogogoch".to_owned();
        long.summary = "Freezing drizzle and blowing snow showers later".to_owned();
        let svg = render(&long);
        assert!(svg.contains('\u{2026}'), "something was ellipsized");
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }

    #[test]
    fn a_missing_hourly_forecast_says_so_rather_than_drawing_an_empty_strip() {
        let mut bare = face();
        bare.hourly.clear();
        let svg = render(&bare);
        assert!(svg.contains("Hourly forecast unavailable"));
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }

    #[test]
    fn more_hours_than_columns_are_truncated_not_overlapped() {
        let mut crowded = face();
        crowded.hourly = (0..24)
            .map(|index| hour(&format!("{index:02}"), 20 + index, Condition::Rain))
            .collect();
        let svg = render(&crowded);
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
        // The 7th hour's label must not appear: it has no column.
        assert!(
            !svg.contains(">06<"),
            "only {STRIP_COLUMNS} columns are drawn"
        );
    }

    #[test]
    fn a_hostile_place_name_cannot_escape_its_text_node() {
        let mut hostile = face();
        hostile.place = r"</text><script>x</script>".to_owned();
        let svg = render(&hostile);
        assert!(!svg.contains("<script>"));
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }
}
