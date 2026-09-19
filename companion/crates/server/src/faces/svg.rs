//! The small SVG-authoring vocabulary the three faces share.
//!
//! Everything here is a pure string builder: no I/O, no fonts, no rasterizer.
//! That is deliberate, because it makes a face's geometry testable without
//! rendering anything, and it keeps the one place that handles untrusted text
//! -- [`escape`] -- small enough to read in full.

use std::fmt::Write as _;

use crate::face_render::{FONT_FAMILY, text_width};

/// Escapes text for an XML **text node or attribute value**.
///
/// Every string a feed, a ticker or a geocoder gives us passes through here on
/// its way into the document. A raw `<` in a headline would otherwise open an
/// element, and while `usvg` forbids scripts and external references outright
/// -- so the worst case is a refused parse rather than anything executing --
/// a face that silently stops rendering when somebody's headline contains an
/// ampersand is its own bug. All five predefined entities are escaped so the
/// same function is correct in both positions.
pub(crate) fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            // XML 1.0 permits only these three control characters. Anything
            // else in that range would make the document unparseable, so it is
            // dropped rather than passed through.
            '\t' | '\n' | '\r' => escaped.push(' '),
            other if (other as u32) < 0x20 => {}
            other => escaped.push(other),
        }
    }
    escaped
}

/// A small count as a float.
///
/// Every count these faces convert is a line number, a column index or a
/// sample index -- all far below 2^24, where `usize as f64` first loses
/// precision. Going through `u32` states that instead of muting the lint, and
/// saturates rather than wrapping if the assumption ever breaks.
pub(crate) fn count(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}

/// Collapses runs of whitespace so a feed's hard-wrapped title measures and
/// wraps as one paragraph.
pub(crate) fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Anchor {
    Start,
    Middle,
    End,
}

impl Anchor {
    const fn as_attribute(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Middle => "middle",
            Self::End => "end",
        }
    }
}

/// One run of text, positioned by its baseline.
#[derive(Debug, Clone)]
pub(crate) struct Text<'a> {
    pub x: f64,
    pub baseline: f64,
    pub content: &'a str,
    pub size: f64,
    pub weight: u16,
    pub fill: &'a str,
    pub anchor: Anchor,
    /// Positive values track the type out, which is what makes a small
    /// uppercase eyebrow readable at 15px on an emissive panel.
    pub letter_spacing: f64,
    pub opacity: f64,
}

impl<'a> Text<'a> {
    pub(crate) fn new(x: f64, baseline: f64, content: &'a str, size: f64, fill: &'a str) -> Self {
        Self {
            x,
            baseline,
            content,
            size,
            weight: 400,
            fill,
            anchor: Anchor::Start,
            letter_spacing: 0.0,
            opacity: 1.0,
        }
    }

    pub(crate) const fn weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    pub(crate) const fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub(crate) const fn tracking(mut self, letter_spacing: f64) -> Self {
        self.letter_spacing = letter_spacing;
        self
    }

    pub(crate) const fn opacity(mut self, opacity: f64) -> Self {
        self.opacity = opacity;
        self
    }
}

/// The document under construction.
#[derive(Debug)]
pub(crate) struct Canvas {
    body: String,
    width: f64,
    height: f64,
}

impl Canvas {
    pub(crate) fn new(width: f64, height: f64) -> Self {
        Self {
            body: String::with_capacity(4_096),
            width,
            height,
        }
    }

    pub(crate) fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, fill: &str) {
        let _ = write!(
            self.body,
            r#"<rect x="{x:.2}" y="{y:.2}" width="{w:.2}" height="{h:.2}" fill="{fill}"/>"#
        );
    }

    pub(crate) fn rounded_rect(&mut self, x: f64, y: f64, w: f64, h: f64, radius: f64, fill: &str) {
        let _ = write!(
            self.body,
            r#"<rect x="{x:.2}" y="{y:.2}" width="{w:.2}" height="{h:.2}" rx="{radius:.2}" fill="{fill}"/>"#
        );
    }

    /// A rect drawn at partial opacity, for a tint of an existing role.
    ///
    /// Eight parameters because eight is what an SVG rect with a radius and an
    /// opacity has; bundling them into a geometry struct would add a type whose
    /// only job is to satisfy a lint.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rect_opacity(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radius: f64,
        fill: &str,
        opacity: f64,
    ) {
        let _ = write!(
            self.body,
            r#"<rect x="{x:.2}" y="{y:.2}" width="{w:.2}" height="{h:.2}" rx="{radius:.2}" fill="{fill}" fill-opacity="{opacity:.3}"/>"#
        );
    }

    pub(crate) fn circle(&mut self, cx: f64, cy: f64, r: f64, fill: &str) {
        let _ = write!(
            self.body,
            r#"<circle cx="{cx:.2}" cy="{cy:.2}" r="{r:.2}" fill="{fill}"/>"#
        );
    }

    pub(crate) fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, stroke: &str, width: f64) {
        let _ = write!(
            self.body,
            r#"<line x1="{x1:.2}" y1="{y1:.2}" x2="{x2:.2}" y2="{y2:.2}" stroke="{stroke}" stroke-width="{width:.2}" stroke-linecap="round"/>"#
        );
    }

    pub(crate) fn path(&mut self, definition: &str, fill: &str) {
        let _ = write!(self.body, r#"<path d="{definition}" fill="{fill}"/>"#);
    }

    pub(crate) fn stroked_path(&mut self, definition: &str, stroke: &str, width: f64) {
        let _ = write!(
            self.body,
            r#"<path d="{definition}" fill="none" stroke="{stroke}" stroke-width="{width:.2}" stroke-linecap="round" stroke-linejoin="round"/>"#
        );
    }

    pub(crate) fn filled_path_opacity(&mut self, definition: &str, fill: &str, opacity: f64) {
        let _ = write!(
            self.body,
            r#"<path d="{definition}" fill="{fill}" fill-opacity="{opacity:.3}"/>"#
        );
    }

    pub(crate) fn text(&mut self, text: &Text<'_>) {
        let tracking = if text.letter_spacing.abs() < f64::EPSILON {
            String::new()
        } else {
            format!(r#" letter-spacing="{:.2}""#, text.letter_spacing)
        };
        let opacity = if (text.opacity - 1.0).abs() < f64::EPSILON {
            String::new()
        } else {
            format!(r#" fill-opacity="{:.3}""#, text.opacity)
        };
        let _ = write!(
            self.body,
            r#"<text x="{x:.2}" y="{baseline:.2}" font-family="{FONT_FAMILY}" font-size="{size:.2}" font-weight="{weight}" fill="{fill}" text-anchor="{anchor}"{tracking}{opacity}>{content}</text>"#,
            x = text.x,
            baseline = text.baseline,
            size = text.size,
            weight = text.weight,
            fill = text.fill,
            anchor = text.anchor.as_attribute(),
            content = escape(text.content),
        );
    }

    pub(crate) fn finish(self) -> String {
        // No `<defs>`: every face is flat fills and strokes, which is what
        // keeps RLE565 compressing a frame to ~10 KB instead of falling back
        // to the ~330 KB raw encoding a gradient would force.
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.0} {height:.0}">{body}</svg>"#,
            width = self.width,
            height = self.height,
            body = self.body,
        )
    }
}

/// Greedily breaks `text` into at most `max_lines` lines that each measure no
/// wider than `max_width`, ellipsizing the last line when the text runs out of
/// room.
///
/// Measured, not estimated. A per-character width factor is tempting and wrong
/// here: "Illinois" and "Wollongong" have the same character count and very
/// different widths, and a fixed panel gives an underestimate nowhere to go.
pub(crate) fn wrap(
    text: &str,
    size: f64,
    weight: u16,
    max_width: f64,
    max_lines: usize,
) -> Vec<String> {
    let normalized = normalize_whitespace(text);
    if normalized.is_empty() || max_lines == 0 {
        return Vec::new();
    }

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut words = normalized.split(' ').peekable();

    while let Some(word) = words.next() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };

        if text_width(&candidate, size, weight) <= max_width {
            current = candidate;
        } else {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                if lines.len() == max_lines {
                    let remainder = std::iter::once(word)
                        .chain(words)
                        .collect::<Vec<_>>()
                        .join(" ");
                    return finish_lines(lines, &remainder, false, size, weight, max_width);
                }
            }

            // One word that does not fit on its own line. Break it by
            // character rather than letting it overhang the canvas -- and keep
            // breaking, because one split is only enough for a word under
            // twice the box width. Pushing the unmeasured remainder is how a
            // long compound noun used to overhang with every test green.
            let mut remainder = word.to_owned();
            while !remainder.is_empty() && lines.len() < max_lines {
                if text_width(&remainder, size, weight) <= max_width {
                    break;
                }
                let (head, tail) = break_word(&remainder, size, weight, max_width);
                lines.push(head);
                remainder = tail;
            }
            if lines.len() == max_lines {
                return finish_lines(
                    lines,
                    &remainder,
                    words.peek().is_some(),
                    size,
                    weight,
                    max_width,
                );
            }
            current = remainder;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Replaces the final line with an ellipsized version when text remains.
fn finish_lines(
    mut lines: Vec<String>,
    remainder: &str,
    more_words: bool,
    size: f64,
    weight: u16,
    max_width: f64,
) -> Vec<String> {
    if remainder.trim().is_empty() && !more_words {
        return lines;
    }
    if let Some(last) = lines.pop() {
        lines.push(ellipsize(&last, size, weight, max_width));
    }
    lines
}

/// The drawn width of a run, tracking included.
///
/// [`crate::face_render::text_width`] lays text out through the same engine
/// that will draw it, but it measures the run with no `letter-spacing` -- and
/// every eyebrow on these faces is tracked out for legibility at 15px. CSS
/// adds the spacing to each character's advance, so a 27-character place name
/// tracked at 1.4 is ~38px wider than its measured width.
///
/// Use this rather than the raw measurement wherever a run is tracked, or a
/// long label can draw through adjacent content despite measuring as fitted.
pub(crate) fn tracked_width(text: &str, size: f64, weight: u16, tracking: f64) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    // Counted over `chars`, which is what the shaper spaces: an accented
    // character is one advance, not two bytes.
    text_width(text, size, weight) + tracking * count(text.chars().count())
}

/// Trims overflowing text until the ellipsis fits, tracking included.
fn trim_with_ellipsis(text: &str, size: f64, weight: u16, tracking: f64, max_width: f64) -> String {
    const ELLIPSIS: char = '\u{2026}';
    let mut characters: Vec<char> = text.chars().collect();
    while !characters.is_empty() {
        characters.pop();
        let candidate: String = characters
            .iter()
            .collect::<String>()
            .trim_end()
            .chars()
            .chain(std::iter::once(ELLIPSIS))
            .collect();
        if tracked_width(&candidate, size, weight, tracking) <= max_width {
            return candidate;
        }
    }
    String::from(ELLIPSIS)
}

/// Fits a run to `max_width`, tracking included, ellipsizing only if needed.
pub(crate) fn fit_tracked(
    text: &str,
    size: f64,
    weight: u16,
    tracking: f64,
    max_width: f64,
) -> String {
    if tracked_width(text, size, weight, tracking) <= max_width {
        return text.to_owned();
    }
    trim_with_ellipsis(text, size, weight, tracking, max_width)
}

/// Returns `text` unchanged when it fits, otherwise truncates it with an
/// ellipsis to fit `max_width`.
pub(crate) fn fit(text: &str, size: f64, weight: u16, max_width: f64) -> String {
    if text_width(text, size, weight) <= max_width {
        return text.to_owned();
    }
    ellipsize(text, size, weight, max_width)
}

/// Trims characters off the end until the run plus an ellipsis fits.
fn ellipsize(text: &str, size: f64, weight: u16, max_width: f64) -> String {
    const ELLIPSIS: char = '\u{2026}';
    if text_width(text, size, weight) <= max_width {
        let mut ellipsized = text.trim_end().to_owned();
        ellipsized.push(ELLIPSIS);
        if text_width(&ellipsized, size, weight) <= max_width {
            return ellipsized;
        }
    }
    trim_with_ellipsis(text, size, weight, 0.0, max_width)
}

/// Splits an unbreakable word at the last character that fits.
fn break_word(word: &str, size: f64, weight: u16, max_width: f64) -> (String, String) {
    let characters: Vec<char> = word.chars().collect();
    let mut fits = 0;
    for index in 1..=characters.len() {
        let candidate: String = characters[..index].iter().collect();
        if text_width(&candidate, size, weight) <= max_width {
            fits = index;
        } else {
            break;
        }
    }
    // Always consume at least one character, or a word narrower than a single
    // glyph would loop forever.
    let split = fits.max(1);
    (
        characters[..split].iter().collect(),
        characters[split..].iter().collect(),
    )
}

/// The largest size from `candidates` (in the order given) at which `text` fits
/// `max_width` on one line, falling back to the last candidate.
///
/// This is what keeps a hero numeral optically as large as it can be: a
/// four-digit price and a two-digit temperature both fill the space they have
/// instead of being set to a size chosen for the widest possible input.
pub(crate) fn fit_size(text: &str, weight: u16, max_width: f64, candidates: &[f64]) -> f64 {
    for &size in candidates {
        if text_width(text, size, weight) <= max_width {
            return size;
        }
    }
    candidates.last().copied().unwrap_or(12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_neutralizes_every_predefined_entity() {
        assert_eq!(
            escape(r#"Rust & "unsafe" <tags> 'quoted'"#),
            "Rust &amp; &quot;unsafe&quot; &lt;tags&gt; &apos;quoted&apos;"
        );
    }

    #[test]
    fn escaping_drops_control_characters_that_would_break_the_parse() {
        // A feed with a stray NUL or vertical tab must not make the face
        // unrenderable; XML 1.0 simply has no way to carry them.
        assert_eq!(escape("head\u{0}\u{b}tail"), "headtail");
        assert_eq!(escape("two\nlines\tapart"), "two lines apart");
    }

    #[test]
    fn a_headline_wraps_within_its_measured_width() {
        let lines = wrap(
            "Rust 1.98 lands with a stable const generics story",
            28.0,
            600,
            360.0,
            3,
        );
        assert!(lines.len() > 1, "a long headline uses more than one line");
        assert!(lines.len() <= 3, "the line cap is respected: {lines:?}");
        for line in &lines {
            assert!(
                text_width(line, 28.0, 600) <= 360.0,
                "line {line:?} measures wider than its box"
            );
        }
    }

    #[test]
    fn overlong_text_ellipsizes_on_the_last_allowed_line() {
        let lines = wrap(&"alpha beta gamma delta ".repeat(12), 28.0, 600, 300.0, 2);
        assert_eq!(lines.len(), 2);
        assert!(
            lines[1].ends_with('\u{2026}'),
            "the final line signals the truncation: {:?}",
            lines[1]
        );
        assert!(text_width(&lines[1], 28.0, 600) <= 300.0);
    }

    #[test]
    fn an_unbreakable_word_is_split_rather_than_overhanging() {
        let lines = wrap("Donaudampfschiffahrtsgesellschaft", 28.0, 600, 120.0, 3);
        assert!(lines.len() > 1, "the word is broken across lines");
        for line in &lines {
            assert!(text_width(line, 28.0, 600) <= 120.0, "line {line:?} fits");
        }
        let prefixed = wrap("A Donaudampfschiffahrtsgesellschaft", 28.0, 600, 120.0, 8);
        assert!(prefixed.len() <= 8);
        assert_eq!(prefixed[0], "A");
        assert_eq!(prefixed[1..].concat(), "Donaudampfschiffahrtsgesellschaft");
        for line in &prefixed {
            assert!(text_width(line, 28.0, 600) <= 120.0, "line {line:?} fits");
        }
        assert_eq!(
            wrap(
                "alpha beta gamma",
                28.0,
                600,
                text_width("alpha beta", 28.0, 600),
                3
            ),
            ["alpha beta", "gamma"]
        );
    }

    #[test]
    fn a_prefixed_compound_respects_the_last_line_and_marks_truncation() {
        for max_lines in [1, 2, 3] {
            let lines = wrap(
                "A Donaudampfschiffahrtsgesellschaft",
                28.0,
                600,
                120.0,
                max_lines,
            );
            assert_eq!(lines.len(), max_lines);
            for line in &lines {
                assert!(text_width(line, 28.0, 600) <= 120.0, "line {line:?} fits");
            }
            assert!(
                lines.last().unwrap().ends_with('…'),
                "truncation is marked: {lines:?}"
            );
        }
    }

    #[test]
    fn wrapping_blank_or_zero_line_text_yields_nothing() {
        assert!(wrap("   \n  ", 28.0, 400, 300.0, 3).is_empty());
        assert!(wrap("real text", 28.0, 400, 300.0, 0).is_empty());
    }

    #[test]
    fn fit_size_picks_the_largest_candidate_that_fits() {
        let candidates = [96.0, 72.0, 56.0, 40.0];
        let wide = fit_size("1234567", 600, 200.0, &candidates);
        let narrow = fit_size("12", 600, 200.0, &candidates);
        assert!(
            narrow >= wide,
            "shorter text gets at least as large a size: {narrow} vs {wide}"
        );
        assert!(candidates.contains(&narrow));
        // The point of fitting: the chosen size actually fits the box.
        assert!(text_width("1234567", wide, 600) <= 200.0);
        assert!(text_width("12", narrow, 600) <= 200.0);
    }

    #[test]
    fn tracking_widens_a_measured_run_in_proportion_to_its_length() {
        let plain = text_width("DUBAI, UNITED ARAB EMIRATES", 15.0, 600);
        let tracked = tracked_width("DUBAI, UNITED ARAB EMIRATES", 15.0, 600, 1.4);
        assert!(
            tracked > plain + 30.0,
            "27 characters at 1.4 add ~38px: {plain} -> {tracked}"
        );
        assert!(tracked_width("", 15.0, 600, 1.4).abs() < f64::EPSILON);
        assert!(
            (tracked_width("AB", 15.0, 600, 0.0) - text_width("AB", 15.0, 600)).abs()
                < f64::EPSILON,
            "zero tracking is the plain measurement"
        );
    }

    #[test]
    fn fitting_a_tracked_run_respects_the_box_it_was_given() {
        let fitted = fit_tracked("DUBAI, UNITED ARAB EMIRATES", 15.0, 600, 1.4, 120.0);
        assert!(fitted.ends_with('\u{2026}'), "it was shortened: {fitted:?}");
        assert!(tracked_width(&fitted, 15.0, 600, 1.4) <= 120.0);
        // A run that already fits is returned untouched, not ellipsized.
        assert_eq!(fit_tracked("DUBAI", 15.0, 600, 1.4, 200.0), "DUBAI");
    }

    #[test]
    fn ellipsizing_marks_fitting_text_but_tracked_fitting_keeps_it() {
        assert_eq!(ellipsize("DUBAI", 15.0, 600, 200.0), "DUBAI…");
        assert_eq!(fit_tracked("DUBAI", 15.0, 600, 1.4, 200.0), "DUBAI");
    }

    #[test]
    fn ordinary_and_tracked_overflow_keep_the_marker_inside_the_box() {
        let text = "DUBAI, UNITED ARAB EMIRATES";
        let ordinary = ellipsize(text, 15.0, 600, 120.0);
        assert!(ordinary.ends_with('…'));
        assert!(text_width(&ordinary, 15.0, 600) <= 120.0);
        for tracking in [0.0, 1.4] {
            let fitted = fit_tracked(text, 15.0, 600, tracking, 120.0);
            assert!(fitted.ends_with('…'));
            assert!(tracked_width(&fitted, 15.0, 600, tracking) <= 120.0);
            if tracking == 0.0 {
                assert_eq!(fitted, ordinary);
            }
        }
    }

    #[test]
    fn a_finished_canvas_is_a_well_formed_single_root_document() {
        let mut canvas = Canvas::new(448.0, 368.0);
        canvas.rect(0.0, 0.0, 448.0, 368.0, "#000000");
        canvas.text(&Text::new(24.0, 40.0, "Solana & co", 24.0, "#f5f5f7").weight(600));
        let svg = canvas.finish();
        assert!(svg.starts_with("<svg xmlns="));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("Solana &amp; co"));
        // The real proof that the builder emits parseable XML.
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }
}
