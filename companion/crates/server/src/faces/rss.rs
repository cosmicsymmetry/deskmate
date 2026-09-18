//! The RSS face: one lead story set large, then the stories behind it.
//!
//! # Why the lead dominates
//!
//! A panel is glanced at, not read. Five equal headlines at a size that fits
//! five of them is a face you have to stop and work through. So the newest item
//! gets the hero treatment and the rest become a quiet index underneath: the
//! glance answers "what just happened", and the detail rewards a second look.
//!
//! # Why the layout is computed rather than fixed
//!
//! A lead headline is one, two or three lines depending on the feed, and a
//! fixed divider position would leave a hole under the short ones. The lead
//! block is measured first and everything below it is placed against that
//! result, with the leftover height divided among however many follower rows
//! actually fit.

use super::svg::{Anchor, Canvas, Text, count, fit, wrap};
use super::theme::{
    CANVAS_HEIGHT, CANVAS_WIDTH, CONTENT_WIDTH, GOOD, GRID, GROUND, HAIRLINE, INK, INK_2, INK_3,
    MARGIN, SIZE_BODY, SIZE_CAPTION, SIZE_EYEBROW, SIZE_TITLE, TRACKING_EYEBROW, WEIGHT_REGULAR,
    WEIGHT_SEMIBOLD, baseline_from_cap_top, baseline_from_center,
};

/// The lead headline's leading, as a multiple of its size. Tighter than a
/// paragraph's, because a two-or-three-line headline reads as one object.
const LEAD_LEADING: f64 = 1.16;
/// The accent bar that marks the freshest item, using `--good` -- the role
/// `DESIGN.md` defines as "fresh".
const ACCENT_WIDTH: f64 = 4.0;
const ACCENT_GUTTER: f64 = 2.0 * GRID;
/// A follower row needs this much height to hold a headline and its age
/// without crowding either.
const MIN_FOLLOWER_HEIGHT: f64 = 40.0;
const MAX_FOLLOWERS: usize = 3;
const MAX_LEAD_LINES: usize = 3;

/// One item, already reduced to what the face draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeedEntry {
    pub title: String,
    /// A rendered relative age such as "14m" or "2h". Empty when the item
    /// carried no usable date, in which case the face simply omits it rather
    /// than inventing "just now".
    pub age: String,
}

/// Everything the RSS face draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RssFace {
    /// The feed's own title, set as the eyebrow.
    pub feed_title: String,
    pub entries: Vec<FeedEntry>,
}

pub(crate) fn render(face: &RssFace) -> String {
    let mut canvas = Canvas::new(CANVAS_WIDTH, CANVAS_HEIGHT);
    canvas.rect(0.0, 0.0, CANVAS_WIDTH, CANVAS_HEIGHT, GROUND);

    draw_eyebrow(&mut canvas, &face.feed_title);

    let Some((lead, followers)) = face.entries.split_first() else {
        draw_empty_state(&mut canvas);
        return canvas.finish();
    };

    let lead_bottom = draw_lead(&mut canvas, lead);
    draw_followers(&mut canvas, followers, lead_bottom);
    canvas.finish()
}

fn draw_eyebrow(canvas: &mut Canvas, feed_title: &str) {
    let eyebrow = ellipsize_eyebrow(feed_title);
    canvas.text(
        &Text::new(
            MARGIN,
            baseline_from_cap_top(MARGIN, SIZE_EYEBROW),
            &eyebrow,
            SIZE_EYEBROW,
            INK_3,
        )
        .weight(WEIGHT_SEMIBOLD)
        .tracking(TRACKING_EYEBROW),
    );
    canvas.rect(MARGIN, 52.0, CONTENT_WIDTH, 1.0, HAIRLINE);
}

/// The eyebrow is uppercased for the label voice, then measured -- uppercasing
/// widens text, so a feed title that fitted in its original case can stop
/// fitting, and the ellipsis has to be applied after the transform.
fn ellipsize_eyebrow(feed_title: &str) -> String {
    let upper = super::svg::normalize_whitespace(feed_title).to_uppercase();
    let fallback = if upper.is_empty() {
        "FEED".to_owned()
    } else {
        upper
    };
    // The eyebrow is tracked, so it is measured with tracking.
    super::svg::fit_tracked(
        &fallback,
        SIZE_EYEBROW,
        WEIGHT_SEMIBOLD,
        TRACKING_EYEBROW,
        CONTENT_WIDTH,
    )
}

/// Draws the lead story and returns the y the next element may start at.
fn draw_lead(canvas: &mut Canvas, lead: &FeedEntry) -> f64 {
    let headline_x = MARGIN + ACCENT_WIDTH + ACCENT_GUTTER;
    let headline_width = CANVAS_WIDTH - MARGIN - headline_x;
    let lines = wrap(
        &lead.title,
        SIZE_TITLE,
        WEIGHT_SEMIBOLD,
        headline_width,
        MAX_LEAD_LINES,
    );

    let cap_top = 52.0 + 2.5 * GRID;
    let leading = SIZE_TITLE * LEAD_LEADING;
    let cap_height = SIZE_TITLE * 0.727;

    if lines.is_empty() {
        return cap_top;
    }

    // The accent spans the headline's own cap height, not the whole line box,
    // so it reads as aligned with the type rather than floating around it.
    let accent_height = (count(lines.len()) - 1.0) * leading + cap_height;
    canvas.rounded_rect(
        MARGIN,
        cap_top,
        ACCENT_WIDTH,
        accent_height,
        ACCENT_WIDTH / 2.0,
        GOOD,
    );

    for (index, line) in lines.iter().enumerate() {
        canvas.text(
            &Text::new(
                headline_x,
                baseline_from_cap_top(cap_top + count(index) * leading, SIZE_TITLE),
                line,
                SIZE_TITLE,
                INK,
            )
            .weight(WEIGHT_SEMIBOLD),
        );
    }

    let mut next = cap_top + accent_height;
    if !lead.age.is_empty() {
        let meta_top = next + 1.5 * GRID;
        canvas.text(
            &Text::new(
                headline_x,
                baseline_from_cap_top(meta_top, SIZE_CAPTION),
                &lead.age,
                SIZE_CAPTION,
                INK_3,
            )
            .weight(WEIGHT_SEMIBOLD)
            .tracking(0.4),
        );
        next = meta_top + SIZE_CAPTION * 0.727;
    }
    next + 2.0 * GRID
}

fn draw_followers(canvas: &mut Canvas, followers: &[FeedEntry], top: f64) {
    let available = CANVAS_HEIGHT - MARGIN - top;
    if available < MIN_FOLLOWER_HEIGHT || followers.is_empty() {
        return;
    }

    // Show only as many rows as fit at a readable height. Squeezing a third
    // row into the space for two is how a face stops being glanceable.
    let capacity = (available / MIN_FOLLOWER_HEIGHT).floor().max(0.0);
    // `floor` already bounded it at or above zero, so the cast cannot lose a
    // sign, and a row capacity never approaches `usize::MAX`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let capacity = capacity as usize;
    let rows = followers
        .len()
        .min(MAX_FOLLOWERS)
        .min(capacity)
        .max(1)
        .min(followers.len());
    let row_height = available / count(rows);

    for (index, entry) in followers.iter().take(rows).enumerate() {
        let row_top = top + count(index) * row_height;
        // Every row gets a rule above it, the first one included: it is what
        // separates the lead block from the index below it, and without it the
        // index reads as rules holding up nothing.
        canvas.rect(MARGIN, row_top, CONTENT_WIDTH, 1.0, HAIRLINE);

        let center = row_top + row_height / 2.0;
        let age_width = if entry.age.is_empty() {
            0.0
        } else {
            crate::face_render::text_width(&entry.age, SIZE_CAPTION, WEIGHT_SEMIBOLD) + 1.5 * GRID
        };
        let title_width = CONTENT_WIDTH - age_width;
        let title = fit(
            &super::svg::normalize_whitespace(&entry.title),
            SIZE_BODY,
            WEIGHT_REGULAR,
            title_width,
        );

        canvas.text(&Text::new(
            MARGIN,
            baseline_from_center(center, SIZE_BODY),
            &title,
            SIZE_BODY,
            INK_2,
        ));

        if !entry.age.is_empty() {
            canvas.text(
                &Text::new(
                    CANVAS_WIDTH - MARGIN,
                    baseline_from_center(center, SIZE_CAPTION),
                    &entry.age,
                    SIZE_CAPTION,
                    INK_3,
                )
                .weight(WEIGHT_SEMIBOLD)
                .anchor(Anchor::End),
            );
        }
    }
}

/// A feed that parsed but carried no items. This is a real steady state for a
/// quiet feed, so it gets a composed face rather than the error footer, which
/// is reserved for a fetch that actually failed.
fn draw_empty_state(canvas: &mut Canvas) {
    let center = CANVAS_HEIGHT / 2.0;
    canvas.text(
        &Text::new(
            CANVAS_WIDTH / 2.0,
            baseline_from_center(center, SIZE_TITLE),
            "No stories yet",
            SIZE_TITLE,
            INK_2,
        )
        .weight(WEIGHT_SEMIBOLD)
        .anchor(Anchor::Middle),
    );
    canvas.text(
        &Text::new(
            CANVAS_WIDTH / 2.0,
            baseline_from_center(center + 4.0 * GRID, SIZE_CAPTION),
            "The feed is reachable and empty",
            SIZE_CAPTION,
            INK_3,
        )
        .anchor(Anchor::Middle),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, age: &str) -> FeedEntry {
        FeedEntry {
            title: title.to_owned(),
            age: age.to_owned(),
        }
    }

    fn face() -> RssFace {
        RssFace {
            feed_title: "Hacker News".to_owned(),
            entries: vec![
                entry(
                    "Rust 1.98 stabilises const generics and lands a faster linker",
                    "14m",
                ),
                entry("A postmortem of the eu-west-1 control plane outage", "1h"),
                entry("Writing a toy TCP stack in 400 lines of Zig", "3h"),
                entry("The case against microservices, revisited", "5h"),
            ],
        }
    }

    #[test]
    fn the_face_renders_and_carries_the_lead_headline() {
        let svg = render(&face());
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
        assert!(
            svg.contains("HACKER NEWS"),
            "the eyebrow is the label voice"
        );
        assert!(svg.contains("Rust 1.98"), "the lead headline is present");
    }

    #[test]
    fn a_hostile_headline_cannot_escape_its_text_node() {
        let mut hostile = face();
        hostile.entries[0].title =
            r"</text><script>fetch('http://evil.test')</script><text>".to_owned();
        let svg = render(&hostile);
        assert!(
            !svg.contains("<script>"),
            "the payload is escaped, not embedded"
        );
        assert!(svg.contains("&lt;script&gt;"));
        // The real assertion: it still rasterizes, so a hostile feed degrades
        // to ugly text rather than to a blank panel.
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }

    #[test]
    fn every_follower_row_stays_inside_the_canvas() {
        // Regression shape for the adaptive layout: a one-line lead leaves the
        // most room and a three-line lead the least, and neither may push a
        // row past the bottom margin.
        for title in ["Short", &"Long headline ".repeat(12)] {
            let mut subject = face();
            subject.entries[0].title = title.to_owned();
            let svg = render(&subject);
            for baseline in text_baselines(&svg) {
                assert!(
                    baseline <= CANVAS_HEIGHT,
                    "a baseline at {baseline} falls off the {CANVAS_HEIGHT}px canvas for lead {title:?}"
                );
            }
            assert!(crate::face_render::frame_from_svg(&svg).is_ok());
        }
    }

    #[test]
    fn an_empty_feed_draws_a_composed_face_rather_than_a_blank_panel() {
        let svg = render(&RssFace {
            feed_title: "Quiet feed".to_owned(),
            entries: Vec::new(),
        });
        assert!(svg.contains("No stories yet"));
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }

    #[test]
    fn a_single_item_feed_draws_the_lead_and_no_divider() {
        let svg = render(&RssFace {
            feed_title: "Single".to_owned(),
            entries: vec![entry("The only story in the feed today", "20m")],
        });
        assert!(svg.contains("The only story"));
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }

    #[test]
    fn an_item_without_a_date_omits_its_age_instead_of_inventing_one() {
        let svg = render(&RssFace {
            feed_title: "Undated".to_owned(),
            entries: vec![entry("A story with no pubDate", "")],
        });
        assert!(!svg.contains("just now"));
        assert!(!svg.contains("ago"));
        assert!(crate::face_render::frame_from_svg(&svg).is_ok());
    }

    /// Pulls every `y=` off the document's text nodes, so a layout assertion
    /// can be made without a pixel diff.
    fn text_baselines(svg: &str) -> Vec<f64> {
        svg.split("<text ")
            .skip(1)
            .filter_map(|node| {
                let start = node.find("y=\"")? + 3;
                let rest = &node[start..];
                let end = rest.find('"')?;
                rest[..end].parse::<f64>().ok()
            })
            .collect()
    }
}
