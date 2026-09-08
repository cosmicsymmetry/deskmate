//! The manifest-v2 `summary` expression: the one value a plugin card shows
//! on its complication tile, evaluated on the server at refresh time.
//!
//! This is deliberately a sibling of `compile.rs`'s text-node path, not a
//! caller of it: a text node's value becomes a wire `SceneValue`, bounded by
//! `protocol::MAX_SCENE_TEXT_LEN` and allowed to be a device binding; a
//! summary becomes a plain provider `Field` string, bounded by
//! [`MAX_SUMMARY_LEN`], and a device binding is refused at manifest parse
//! time (`ManifestError::SummaryUsesDeviceBinding`) because nothing here could
//! resolve one. What the two share -- the whole-value interpolation rule and
//! the expression grammar -- they share by construction:
//! [`classify_expression_source`] and [`Expr`].

use std::fmt;

use providers::ProviderSnapshot;

use crate::compile::{ExpressionSource, classify_expression_source};
use crate::expr::{EvalContext, Expr, ExprError, FUEL_BUDGET, Fuel};
use crate::manifest::PluginManifest;

/// Maximum byte length of an evaluated summary. A complication tile shows
/// one short fact; anything longer is truncated on a char boundary, never
/// refused -- a long headline is not a broken plugin.
pub const MAX_SUMMARY_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryError {
    /// The summary contains mustache delimiters but is not exactly one
    /// complete `{{ ... }}` expression -- the same whole-value rule a text
    /// node's `value` follows.
    MalformedPartialInterpolation { text: String },
    /// The expression failed to parse or evaluate.
    Expression(ExprError),
}

impl fmt::Display for SummaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for SummaryError {}

/// Evaluates `manifest.summary` against `snapshot.value`.
///
/// Returns `Ok(None)` when the manifest declares no summary, when the
/// expression evaluates to `EvalValue::Missing` (an absent field, a JSON
/// `null`, a type mismatch), and when what it evaluates to is empty: a
/// missing headline is "no value", which the tile prints as an em dash, not
/// an empty string it would print as nothing. A stale or error snapshot is
/// evaluated like any other, because
/// it still carries the last-good value (`providers::LastGood::complete`'s
/// contract). `icon()` has no icon map here and evaluates to `Missing`; a
/// codepoint is not a headline.
///
/// # Errors
///
/// See [`SummaryError`].
pub fn evaluate_summary(
    manifest: &PluginManifest,
    snapshot: &ProviderSnapshot<serde_json::Value>,
) -> Result<Option<String>, SummaryError> {
    let Some(source) = manifest.summary.as_deref() else {
        return Ok(None);
    };
    let inner = match classify_expression_source(source) {
        ExpressionSource::Literal(literal) => return Ok(bound_summary(literal)),
        ExpressionSource::Expression(inner) => inner,
        ExpressionSource::MalformedPartial => {
            return Err(SummaryError::MalformedPartialInterpolation {
                text: source.to_string(),
            });
        }
    };
    let expr = Expr::parse(inner).map_err(SummaryError::Expression)?;
    let ctx = EvalContext::with_data(&snapshot.value);
    let mut fuel = Fuel::new(FUEL_BUDGET);
    let value = expr
        .eval(&ctx, &mut fuel)
        .map_err(SummaryError::Expression)?;
    if value.is_missing() {
        return Ok(None);
    }
    Ok(bound_summary(&value.to_string()))
}

/// Bounds an evaluated summary, and answers `None` for an empty one.
///
/// Emptiness is judged here, once, after bounding, rather than per expression
/// kind: a present-but-empty JSON string (`"aqi": ""`) is ordinary provider
/// data, and it means exactly what a missing value means -- no headline. The
/// tile prints an em dash for `None` and nothing at all for `Some("")`, so the
/// two must not be distinguishable this far up.
fn bound_summary(text: &str) -> Option<String> {
    let bounded = protocol::truncate_utf8_to_bytes(text, MAX_SUMMARY_LEN);
    (!bounded.is_empty()).then(|| bounded.to_owned())
}
