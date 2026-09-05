//! The restricted plugin expression language.
//!
//! A `text` node's `value` or a `glyph` node's `glyph` field may carry
//! `"{{ ... }}"` source (see `manifest.rs`'s module doc). Task 1 stores that
//! source as bounded, opaque text; this module owns the grammar inside the
//! braces once a caller has stripped the mustache delimiters and decided
//! the trimmed text is not one of the closed device bindings
//! (`time:*`, `timer.*`, `field.*` -- that decision belongs to the compiler,
//! Task 3, which is also the caller that strips `{{`/`}}`). Everything that
//! reaches [`Expr::parse`] here is expected to be plain expression syntax
//! with no mustache delimiters, exactly the way the plan's own termination
//! test constructs it.
//!
//! # What this module is not
//!
//! It is deliberately not a scripting language. There are no user-defined
//! names, no assignment, and no loops -- the manifest's one repeat form
//! (Task 3) is the only repetition this stage has. The grammar is closed:
//! field access into fetched provider data (`data.a.b`), array index
//! (`data.rows[0]`), a fixed function set, and `?:` for presence. Adding a
//! general-purpose feature here is the specific failure this module exists
//! to prevent -- see the plan's stage-3b ledger entry decision.
//!
//! # Bounding
//!
//! Every evaluation is bounded on four independent axes so untrusted
//! manifest/provider input can never panic, hang, or allocate without a
//! cap in front of it:
//!
//! - **Depth** ([`MAX_DEPTH`]) is enforced at *parse* time: nested function
//!   calls and parenthesised sub-expressions past the cap are a parse
//!   error, not a runtime one.
//! - **Fuel** ([`FUEL_BUDGET`], via [`Fuel`]) is enforced at *eval* time.
//!   One `Fuel` is meant to be threaded through every node's `eval()` call
//!   for a single manifest compile (Task 3's job), so a legal expression
//!   repeated across many nodes still terminates against one shared
//!   budget -- not a fresh budget per node.
//! - **Output size** ([`MAX_OUTPUT_LEN`]) bounds every [`EvalValue::Text`]
//!   at the moment it is constructed. A value that exceeds it is rejected
//!   with the named [`ExprError::OutputTooLong`], not silently shortened --
//!   silent truncation would be indistinguishable from `truncate(s, n)`'s
//!   own deliberate one, and would misreport an oversized value as
//!   `Missing` data that "is not there". `truncate(s, n)` counts characters,
//!   but its input has already passed this byte bound, so its character
//!   prefix cannot exceed the source's byte length. Provider JSON already
//!   owns its strings; the evaluator checks their borrowed length before
//!   making its additional clone.
//! - **Path width** ([`MAX_PATH_SEGMENTS`]) bounds the number of
//!   `.field`/`[index]` steps in one path chain. Depth alone only bounds
//!   *nested* structure (parens/calls); a long flat `data.a.b.c...` chain
//!   has depth 1 and needs its own cap.
//!
//! Total source length ([`MAX_SOURCE_LEN`]) is checked first, before any
//! parsing, independently of `manifest::MAX_EXPR_SOURCE_LEN` -- this module
//! does not trust a caller to have already bounded its input.

use std::collections::HashMap;
use std::fmt;

use crate::manifest::Glyph;

// ---------------------------------------------------------------------------
// Bounds. Each protects a specific untrusted-input hazard.
// ---------------------------------------------------------------------------

/// Maximum byte length of expression source `Expr::parse` will accept,
/// checked before any parsing happens. Generous headroom above
/// `manifest::MAX_EXPR_SOURCE_LEN` (256, which bounds the *whole*
/// `"{{ ... }}"` field including delimiters and whitespace): this module
/// is a public API in its own right and does not assume every caller
/// already applied that bound.
pub const MAX_SOURCE_LEN: usize = 512;

/// Maximum nesting depth of function calls and parenthesised
/// sub-expressions, enforced at parse time. Chosen to match
/// `manifest::MAX_TOML_NESTING_DEPTH` for the same reason: generous above
/// any legitimate expression (`truncate(upper(default(data.a, "x")), 64)`
/// is depth 4), cheap insurance against pathological input.
pub const MAX_DEPTH: usize = 16;

/// Maximum number of `.field` / `[index]` steps in one path chain. Depth
/// bounds nested structure; this bounds a long flat chain, which has
/// depth 1 regardless of its length.
pub const MAX_PATH_SEGMENTS: usize = 16;

/// Default fuel budget for one manifest compile. Meant to be shared, via a
/// single [`Fuel`], across every node's `eval()` call for one `Scene`
/// (`MAX_NODES` = `protocol::MAX_SCENE_NODES`), not reset per node. Each
/// AST node visited during evaluation consumes 1 unit, so a legal
/// expression like `truncate(upper(data.a), 64)` (5 nodes: the two calls,
/// the path, one path segment, and the numeric literal) costs 5 per
/// evaluation, comfortably covering a full manifest many times over.
pub const FUEL_BUDGET: u32 = 10_000;

/// Maximum byte length of any [`EvalValue::Text`], enforced at construction
/// by rejection. For provider strings, the borrowed length is checked before
/// the evaluator makes its own clone.
pub const MAX_OUTPUT_LEN: usize = 4096;

/// Maximum decimal places `round(x, places)` accepts. Above this, `10^places`
/// stops usefully discriminating `f64` values; bounding it keeps the
/// intermediate `x * 10^places` scaling from approaching `f64::INFINITY`.
pub const MAX_ROUND_PLACES: i32 = 12;

// ---------------------------------------------------------------------------
// Errors: one variant per rejection reason.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprError {
    /// The trimmed source was empty.
    Empty,
    /// Source exceeds [`MAX_SOURCE_LEN`].
    TooLong { limit: usize, actual: usize },
    /// Nested function calls / parens exceed [`MAX_DEPTH`].
    TooDeep { limit: usize, actual: usize },
    /// A path chain exceeds [`MAX_PATH_SEGMENTS`].
    TooManyPathSegments { limit: usize, actual: usize },
    /// The parser reached the end of input mid-expression (an unterminated
    /// call, string, or dangling operator).
    UnexpectedEnd,
    /// A string literal has no closing `"` before the end of input.
    UnterminatedString,
    /// A token was found where the grammar does not allow it.
    UnexpectedToken { at: usize },
    /// Trailing characters remained after a complete expression was parsed.
    TrailingInput { at: usize },
    /// A bare identifier that is neither `data` nor a known function name.
    /// The grammar has no user-defined names, so this is always a rejection.
    UnknownIdentifier { name: String },
    /// A call to a function not in the fixed set.
    UnknownFunction { name: String },
    /// A call to a known function with the wrong number of arguments.
    WrongArity {
        function: &'static str,
        expected: usize,
        actual: usize,
    },
    /// A numeric literal or array index did not parse as a number.
    InvalidNumber { at: usize },
    /// A `Text` value (a string literal, a field read, or `upper`/`lower`'s
    /// result) would exceed [`MAX_OUTPUT_LEN`] bytes. This is distinct from
    /// `truncate(s, n)`'s own explicit truncation, which is an author's
    /// request and stays a value, not an error: this variant fires only
    /// where truncation was never asked for, so silently shortening the
    /// value would misreport "the data is not there" (`Missing`) or hide
    /// that a safety cap -- not the author -- shaped the output.
    OutputTooLong { limit: usize, actual: usize },
    /// Evaluation exhausted its shared [`Fuel`] budget.
    OutOfFuel,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ExprError {}

// ---------------------------------------------------------------------------
// Values.
// ---------------------------------------------------------------------------

/// The result of evaluating an [`Expr`]. `Missing` is not an error: a
/// missing key, a JSON `null`, an out-of-range index, or a function given
/// the wrong shape of input all evaluate to it rather than failing the
/// whole expression, which is what keeps every function total.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalValue {
    Text(String),
    Number(f64),
    Bool(bool),
    Missing,
}

impl EvalValue {
    #[must_use]
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

/// Renders a value the way it should appear on the panel. `Missing` has no
/// textual form of its own -- a caller (Task 3) that needs to decide what
/// a missing value displays as (blank, an em dash, the state footer) should
/// match on `EvalValue::Missing` explicitly rather than rely on this.
impl fmt::Display for EvalValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(s) => write!(f, "{s}"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Missing => write!(f, ""),
            Self::Number(n) => {
                if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                    #[allow(clippy::cast_possible_truncation)]
                    let as_i64 = *n as i64;
                    write!(f, "{as_i64}")
                } else {
                    write!(f, "{n}")
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fuel.
// ---------------------------------------------------------------------------

/// A shared evaluation budget. One `Fuel` is meant to be threaded, by
/// mutable reference, through every node's `eval()` call for a single
/// manifest compile -- see [`FUEL_BUDGET`]'s doc for why that sharing
/// matters.
#[derive(Debug, Clone, Copy)]
pub struct Fuel {
    remaining: u32,
}

impl Fuel {
    #[must_use]
    pub fn new(budget: u32) -> Self {
        Self { remaining: budget }
    }

    #[must_use]
    pub fn remaining(&self) -> u32 {
        self.remaining
    }

    fn consume(&mut self, cost: u32) -> Result<(), ExprError> {
        if let Some(rest) = self.remaining.checked_sub(cost) {
            self.remaining = rest;
            Ok(())
        } else {
            self.remaining = 0;
            Err(ExprError::OutOfFuel)
        }
    }
}

// ---------------------------------------------------------------------------
// Evaluation context.
// ---------------------------------------------------------------------------

/// What an expression evaluates against: the fetched provider payload
/// (`data.*` paths walk this) and, for `icon(name)`, one icon-font's
/// name-to-codepoint map -- built from Task 1's `manifest::Glyph` list via
/// [`build_icon_map`], not redesigned here.
#[derive(Debug, Clone, Copy, Default)]
pub struct EvalContext<'a> {
    data: Option<&'a serde_json::Value>,
    icons: Option<&'a HashMap<String, u32>>,
}

impl<'a> EvalContext<'a> {
    /// A context with no data and no icon map. `data.*` paths and `icon()`
    /// both evaluate to `Missing` against it; nothing panics or errors.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            data: None,
            icons: None,
        }
    }

    /// A context over fetched provider data, with no icon-font map --
    /// `icon()` calls evaluate to `Missing`.
    #[must_use]
    pub fn with_data(data: &'a serde_json::Value) -> Self {
        Self {
            data: Some(data),
            icons: None,
        }
    }

    /// A context over both fetched provider data and one icon-font's
    /// glyph map.
    #[must_use]
    pub fn new(data: &'a serde_json::Value, icons: &'a HashMap<String, u32>) -> Self {
        Self {
            data: Some(data),
            icons: Some(icons),
        }
    }
}

/// Builds the name-to-codepoint lookup [`EvalContext::new`] takes, from one
/// icon-font asset's already-parsed, already-bounded glyph list
/// (`manifest::Asset::IconFont { glyphs, .. }`). This does not redesign
/// Task 1's map -- it is a lookup index over exactly the `(name, codepoint)`
/// pairs Task 1 already validated (codepoints are guaranteed valid Unicode
/// scalar values by `manifest.rs`'s own parse-time check).
#[must_use]
pub fn build_icon_map(glyphs: &[Glyph]) -> HashMap<String, u32> {
    glyphs
        .iter()
        .map(|glyph| (glyph.name.clone(), glyph.codepoint))
        .collect()
}

// ---------------------------------------------------------------------------
// AST.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Function {
    Upper,
    Lower,
    Round,
    Truncate,
    Default,
    Icon,
}

impl Function {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "upper" => Some(Self::Upper),
            "lower" => Some(Self::Lower),
            "round" => Some(Self::Round),
            "truncate" => Some(Self::Truncate),
            "default" => Some(Self::Default),
            "icon" => Some(Self::Icon),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Upper => "upper",
            Self::Lower => "lower",
            Self::Round => "round",
            Self::Truncate => "truncate",
            Self::Default => "default",
            Self::Icon => "icon",
        }
    }

    fn arity(self) -> usize {
        match self {
            Self::Upper | Self::Lower | Self::Icon => 1,
            Self::Round | Self::Truncate | Self::Default => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PathSegment {
    Field(String),
    Index(u32),
}

/// A parsed expression. Opaque outside this module: everything a caller
/// needs is `parse` and `eval`.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr(ExprNode);

#[derive(Debug, Clone, PartialEq)]
enum ExprNode {
    Str(String),
    Num(f64),
    /// `data` followed by zero or more `.field` / `[index]` steps.
    Path(Vec<PathSegment>),
    Call(Function, Vec<ExprNode>),
    /// `lhs ?: rhs` -- presence: `lhs` if not `Missing`, else `rhs`.
    Elvis(Box<ExprNode>, Box<ExprNode>),
}

// ---------------------------------------------------------------------------
// Parser: small recursive-descent, explicit depth counter.
// ---------------------------------------------------------------------------

struct Parser {
    chars: Vec<char>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            depth: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, expected: char) -> Result<(), ExprError> {
        self.skip_ws();
        match self.peek() {
            Some(c) if c == expected => {
                self.pos += 1;
                Ok(())
            }
            Some(_) => Err(ExprError::UnexpectedToken { at: self.pos }),
            None => Err(ExprError::UnexpectedEnd),
        }
    }

    /// `?:` is two characters; only consume both if both are present.
    fn eat_elvis(&mut self) -> bool {
        self.skip_ws();
        if self.chars.get(self.pos) == Some(&'?') && self.chars.get(self.pos + 1) == Some(&':') {
            self.pos += 2;
            true
        } else {
            false
        }
    }

    fn enter_nesting(&mut self) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ExprError::TooDeep {
                limit: MAX_DEPTH,
                actual: self.depth,
            });
        }
        Ok(())
    }

    fn leave_nesting(&mut self) {
        self.depth -= 1;
    }

    fn parse_expression(&mut self) -> Result<ExprNode, ExprError> {
        let mut left = self.parse_primary()?;
        while self.eat_elvis() {
            let right = self.parse_primary()?;
            left = ExprNode::Elvis(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_primary(&mut self) -> Result<ExprNode, ExprError> {
        self.skip_ws();
        match self.peek() {
            Some('"') => self.parse_string(),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            Some('(') => {
                self.pos += 1;
                self.enter_nesting()?;
                let inner = self.parse_expression();
                self.leave_nesting();
                let inner = inner?;
                self.eat(')')?;
                Ok(inner)
            }
            Some(c) if is_ident_start(c) => self.parse_ident_led(),
            Some(_) => Err(ExprError::UnexpectedToken { at: self.pos }),
            None => Err(ExprError::UnexpectedEnd),
        }
    }

    fn parse_ident_led(&mut self) -> Result<ExprNode, ExprError> {
        let name = self.read_ident();
        self.skip_ws();
        if self.peek() == Some('(') {
            self.parse_call(&name)
        } else if name == "data" {
            self.parse_path_tail()
        } else {
            Err(ExprError::UnknownIdentifier { name })
        }
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if is_ident_continue(c) {
                s.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        s
    }

    fn parse_call(&mut self, name: &str) -> Result<ExprNode, ExprError> {
        let Some(function) = Function::from_name(name) else {
            return Err(ExprError::UnknownFunction {
                name: name.to_string(),
            });
        };
        self.eat('(')?;
        self.enter_nesting()?;
        let args_result = self.parse_call_args();
        self.leave_nesting();
        let args = args_result?;
        self.eat(')')?;
        let expected = function.arity();
        if args.len() != expected {
            return Err(ExprError::WrongArity {
                function: function.name(),
                expected,
                actual: args.len(),
            });
        }
        Ok(ExprNode::Call(function, args))
    }

    fn parse_call_args(&mut self) -> Result<Vec<ExprNode>, ExprError> {
        let mut args = Vec::new();
        self.skip_ws();
        if self.peek() == Some(')') {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expression()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                _ => break,
            }
        }
        Ok(args)
    }

    fn parse_path_tail(&mut self) -> Result<ExprNode, ExprError> {
        let mut segments = Vec::new();
        loop {
            match self.peek() {
                Some('.') => {
                    self.pos += 1;
                    if !matches!(self.peek(), Some(c) if is_ident_start(c)) {
                        return Err(ExprError::UnexpectedToken { at: self.pos });
                    }
                    let field = self.read_ident();
                    segments.push(PathSegment::Field(field));
                }
                Some('[') => {
                    self.pos += 1;
                    let index = self.parse_index()?;
                    self.eat(']')?;
                    segments.push(PathSegment::Index(index));
                }
                _ => break,
            }
            if segments.len() > MAX_PATH_SEGMENTS {
                return Err(ExprError::TooManyPathSegments {
                    limit: MAX_PATH_SEGMENTS,
                    actual: segments.len(),
                });
            }
        }
        Ok(ExprNode::Path(segments))
    }

    fn parse_index(&mut self) -> Result<u32, ExprError> {
        let start = self.pos;
        let mut digits = String::new();
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            digits.push(self.advance().expect("peek confirmed a char is present"));
        }
        if digits.is_empty() {
            return Err(ExprError::InvalidNumber { at: start });
        }
        digits
            .parse::<u32>()
            .map_err(|_| ExprError::InvalidNumber { at: start })
    }

    fn parse_number(&mut self) -> Result<ExprNode, ExprError> {
        let start = self.pos;
        let mut text = String::new();
        if self.peek() == Some('-') {
            text.push('-');
            self.pos += 1;
        }
        let mut saw_digit = false;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            saw_digit = true;
            text.push(self.advance().expect("peek confirmed a char is present"));
        }
        if self.peek() == Some('.')
            && matches!(self.chars.get(self.pos + 1), Some(c) if c.is_ascii_digit())
        {
            text.push('.');
            self.pos += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                text.push(self.advance().expect("peek confirmed a char is present"));
            }
        }
        if !saw_digit {
            return Err(ExprError::InvalidNumber { at: start });
        }
        text.parse::<f64>()
            .map(ExprNode::Num)
            .map_err(|_| ExprError::InvalidNumber { at: start })
    }

    fn parse_string(&mut self) -> Result<ExprNode, ExprError> {
        self.pos += 1; // opening quote
        let mut s = String::new();
        loop {
            match self.advance() {
                Some('"') => return Ok(ExprNode::Str(s)),
                Some('\\') => match self.advance() {
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some(other) => {
                        s.push('\\');
                        s.push(other);
                    }
                    None => return Err(ExprError::UnterminatedString),
                },
                Some(c) => s.push(c),
                None => return Err(ExprError::UnterminatedString),
            }
        }
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

// ---------------------------------------------------------------------------
// Public parse/eval entry points.
// ---------------------------------------------------------------------------

impl Expr {
    /// Parses plain expression syntax -- no `{{ }}` delimiters, those are a
    /// caller's job to strip (see the module doc). Depth is rejected here,
    /// at parse time; fuel bounds evaluation.
    pub fn parse(source: &str) -> Result<Self, ExprError> {
        let trimmed = source.trim();
        if trimmed.is_empty() {
            return Err(ExprError::Empty);
        }
        if trimmed.len() > MAX_SOURCE_LEN {
            return Err(ExprError::TooLong {
                limit: MAX_SOURCE_LEN,
                actual: trimmed.len(),
            });
        }
        let mut parser = Parser::new(trimmed);
        let node = parser.parse_expression()?;
        parser.skip_ws();
        if parser.pos < parser.chars.len() {
            return Err(ExprError::TrailingInput { at: parser.pos });
        }
        Ok(Self(node))
    }

    /// Evaluates against `ctx`, consuming from the shared `fuel` budget.
    /// Never panics: a missing key, a type mismatch, or any other
    /// non-conforming shape evaluates to `EvalValue::Missing` rather than
    /// erroring -- the only evaluation-time error is running out of fuel.
    pub fn eval(&self, ctx: &EvalContext<'_>, fuel: &mut Fuel) -> Result<EvalValue, ExprError> {
        eval_node(&self.0, ctx, fuel)
    }
}

fn eval_node(
    node: &ExprNode,
    ctx: &EvalContext<'_>,
    fuel: &mut Fuel,
) -> Result<EvalValue, ExprError> {
    fuel.consume(1)?;
    match node {
        ExprNode::Str(s) => Ok(EvalValue::Text(bound_text(s.clone())?)),
        ExprNode::Num(n) => Ok(EvalValue::Number(*n)),
        ExprNode::Path(segments) => eval_path(ctx.data, segments, fuel),
        ExprNode::Elvis(lhs, rhs) => {
            let left = eval_node(lhs, ctx, fuel)?;
            if left.is_missing() {
                eval_node(rhs, ctx, fuel)
            } else {
                Ok(left)
            }
        }
        ExprNode::Call(function, args) => eval_call(*function, args, ctx, fuel),
    }
}

fn eval_path(
    root: Option<&serde_json::Value>,
    segments: &[PathSegment],
    fuel: &mut Fuel,
) -> Result<EvalValue, ExprError> {
    let Some(mut current) = root else {
        return Ok(EvalValue::Missing);
    };
    for segment in segments {
        fuel.consume(1)?;
        let next = match (current, segment) {
            (serde_json::Value::Object(map), PathSegment::Field(name)) => map.get(name),
            (serde_json::Value::Array(items), PathSegment::Index(index)) => {
                let index = usize::try_from(*index).unwrap_or(usize::MAX);
                items.get(index)
            }
            _ => None,
        };
        match next {
            Some(value) => current = value,
            None => return Ok(EvalValue::Missing),
        }
    }
    json_to_eval_value(current)
}

fn json_to_eval_value(value: &serde_json::Value) -> Result<EvalValue, ExprError> {
    match value {
        serde_json::Value::Bool(b) => Ok(EvalValue::Bool(*b)),
        serde_json::Value::Number(n) => match n.as_f64() {
            Some(f) if f.is_finite() => Ok(EvalValue::Number(f)),
            _ => Ok(EvalValue::Missing),
        },
        serde_json::Value::String(s) => {
            check_text_len(s)?;
            Ok(EvalValue::Text(s.clone()))
        }
        // `Null` and the two compound kinds all land here: `null` has no
        // scalar EvalValue, and a path that stops on an object or array
        // (without indexing/field-ing further) is not a value this
        // language can express either.
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Ok(EvalValue::Missing)
        }
    }
}

/// Checks a borrowed string before the evaluator makes an owned copy.
fn check_text_len(s: &str) -> Result<(), ExprError> {
    if s.len() > MAX_OUTPUT_LEN {
        Err(ExprError::OutputTooLong {
            limit: MAX_OUTPUT_LEN,
            actual: s.len(),
        })
    } else {
        Ok(())
    }
}

/// Bounds evaluator-constructed `Text` to [`MAX_OUTPUT_LEN`] bytes by
/// rejection, never truncation. Provider strings use [`check_text_len`]
/// before cloning; this owned-value path covers literals and function
/// results.
fn bound_text(s: String) -> Result<String, ExprError> {
    check_text_len(&s)?;
    Ok(s)
}

fn eval_call(
    function: Function,
    args: &[ExprNode],
    ctx: &EvalContext<'_>,
    fuel: &mut Fuel,
) -> Result<EvalValue, ExprError> {
    match function {
        Function::Upper => {
            let x = eval_node(&args[0], ctx, fuel)?;
            call_upper(&x)
        }
        Function::Lower => {
            let x = eval_node(&args[0], ctx, fuel)?;
            call_lower(&x)
        }
        Function::Round => {
            let x = eval_node(&args[0], ctx, fuel)?;
            let places = eval_node(&args[1], ctx, fuel)?;
            Ok(call_round(&x, &places))
        }
        Function::Truncate => {
            let s = eval_node(&args[0], ctx, fuel)?;
            let n = eval_node(&args[1], ctx, fuel)?;
            Ok(call_truncate(&s, &n))
        }
        Function::Default => {
            // Lazily evaluated, like `?:`: the fallback costs fuel only
            // when the primary value is actually missing.
            let x = eval_node(&args[0], ctx, fuel)?;
            if x.is_missing() {
                eval_node(&args[1], ctx, fuel)
            } else {
                Ok(x)
            }
        }
        Function::Icon => {
            let name = eval_node(&args[0], ctx, fuel)?;
            Ok(call_icon(&name, ctx.icons))
        }
    }
}

fn call_upper(x: &EvalValue) -> Result<EvalValue, ExprError> {
    match x {
        EvalValue::Text(s) => Ok(EvalValue::Text(bound_text(s.to_uppercase())?)),
        _ => Ok(EvalValue::Missing),
    }
}

fn call_lower(x: &EvalValue) -> Result<EvalValue, ExprError> {
    match x {
        EvalValue::Text(s) => Ok(EvalValue::Text(bound_text(s.to_lowercase())?)),
        _ => Ok(EvalValue::Missing),
    }
}

fn call_round(x: &EvalValue, places: &EvalValue) -> EvalValue {
    let (EvalValue::Number(n), EvalValue::Number(p)) = (x, places) else {
        return EvalValue::Missing;
    };
    if !n.is_finite() || !p.is_finite() || p.fract() != 0.0 {
        return EvalValue::Missing;
    }
    if *p < 0.0 || *p > f64::from(MAX_ROUND_PLACES) {
        return EvalValue::Missing;
    }
    #[allow(clippy::cast_possible_truncation)]
    // Safe: `p` was just checked finite, integral, and within
    // 0..=MAX_ROUND_PLACES, so this cannot lose information.
    let places_i32 = *p as i32;
    let scale = 10f64.powi(places_i32);
    let scaled = n * scale;
    if !scaled.is_finite() {
        return EvalValue::Missing;
    }
    EvalValue::Number(scaled.round() / scale)
}

fn call_truncate(s: &EvalValue, n: &EvalValue) -> EvalValue {
    let (EvalValue::Text(text), EvalValue::Number(count)) = (s, n) else {
        return EvalValue::Missing;
    };
    if !count.is_finite() || *count < 0.0 {
        return EvalValue::Missing;
    }
    // Clamped into 0..=MAX_OUTPUT_LEN first, so the cast below is lossless
    // regardless of how large a caller's literal is.
    #[allow(clippy::cast_precision_loss)]
    let cap = MAX_OUTPUT_LEN as f64;
    let clamped = count.min(cap);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let max_chars = clamped as usize;
    EvalValue::Text(text.chars().take(max_chars).collect())
}

fn call_icon(name: &EvalValue, icons: Option<&HashMap<String, u32>>) -> EvalValue {
    let EvalValue::Text(name) = name else {
        return EvalValue::Missing;
    };
    let Some(map) = icons else {
        return EvalValue::Missing;
    };
    let Some(&codepoint) = map.get(name.as_str()) else {
        return EvalValue::Missing;
    };
    match char::from_u32(codepoint) {
        Some(c) => EvalValue::Text(c.to_string()),
        None => EvalValue::Missing,
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Step 1: termination, written and watched-red before any
    // implementation existed (see git history / task report).

    #[test]
    fn evaluation_always_terminates_within_its_fuel() {
        let nested = "upper(".repeat(MAX_DEPTH + 1) + "data.a" + &")".repeat(MAX_DEPTH + 1);
        assert!(matches!(
            Expr::parse(&nested),
            Err(ExprError::TooDeep { .. })
        ));

        let mut fuel = Fuel::new(FUEL_BUDGET);
        let expr = Expr::parse("truncate(upper(data.a), 64)").unwrap();
        let err = loop {
            if let Err(err) = expr.eval(&EvalContext::empty(), &mut fuel) {
                break err;
            }
        };
        assert!(matches!(err, ExprError::OutOfFuel));
    }

    #[test]
    fn depth_at_the_ceiling_is_accepted() {
        let nested = "upper(".repeat(MAX_DEPTH) + "data.a" + &")".repeat(MAX_DEPTH);
        assert!(Expr::parse(&nested).is_ok());
    }

    #[test]
    fn a_call_used_as_the_calls_depth_test_shows_the_exact_boundary() {
        let one_past = "upper(".repeat(MAX_DEPTH + 1) + "data.a" + &")".repeat(MAX_DEPTH + 1);
        match Expr::parse(&one_past) {
            Err(ExprError::TooDeep { limit, actual }) => {
                assert_eq!(limit, MAX_DEPTH);
                assert_eq!(actual, MAX_DEPTH + 1);
            }
            other => panic!("expected TooDeep, got {other:?}"),
        }
    }

    // -- Grammar: field access, index, functions, elvis.

    #[test]
    fn field_access_reads_nested_json() {
        let data = serde_json::json!({ "a": { "b": 7 } });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("data.a.b")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Number(7.0));
    }

    #[test]
    fn array_index_reads_an_element() {
        let data = serde_json::json!({ "rows": ["x", "y", "z"] });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("data.rows[1]")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Text("y".to_string()));
    }

    #[test]
    fn functions_compose() {
        let data = serde_json::json!({ "name": "hello world this is long" });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("truncate(upper(data.name), 5)")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Text("HELLO".to_string()));
    }

    #[test]
    fn elvis_falls_through_on_missing() {
        let data = serde_json::json!({});
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse(r#"data.missing ?: "fallback""#)
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Text("fallback".to_string()));
    }

    #[test]
    fn elvis_prefers_present_left_side() {
        let data = serde_json::json!({ "a": "present" });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse(r#"data.a ?: "fallback""#)
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Text("present".to_string()));
    }

    #[test]
    fn default_is_the_function_spelling_of_elvis() {
        let data = serde_json::json!({});
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse(r#"default(data.missing, "n/a")"#)
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Text("n/a".to_string()));
    }

    #[test]
    fn icon_resolves_through_the_supplied_map() {
        let data = serde_json::json!({ "category": "haze" });
        let icons: HashMap<String, u32> = [("haze".to_string(), 0xE001)].into_iter().collect();
        let ctx = EvalContext::new(&data, &icons);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("icon(data.category)")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(
            value,
            EvalValue::Text(char::from_u32(0xE001).unwrap().to_string())
        );
    }

    #[test]
    fn icon_with_an_unknown_name_is_missing() {
        let data = serde_json::json!({ "category": "unregistered" });
        let icons: HashMap<String, u32> = [("haze".to_string(), 0xE001)].into_iter().collect();
        let ctx = EvalContext::new(&data, &icons);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("icon(data.category)")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Missing);
    }

    #[test]
    fn round_rounds_to_the_requested_places() {
        let data = serde_json::json!({ "x": 7.891_23 });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("round(data.x, 2)")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Number(7.89));
    }

    #[test]
    fn unknown_identifiers_are_rejected_at_parse_time() {
        assert!(matches!(
            Expr::parse("foo.bar"),
            Err(ExprError::UnknownIdentifier { .. })
        ));
    }

    #[test]
    fn unknown_functions_are_rejected_at_parse_time() {
        assert!(matches!(
            Expr::parse("frobnicate(data.a)"),
            Err(ExprError::UnknownFunction { .. })
        ));
    }

    #[test]
    fn wrong_arity_is_rejected_at_parse_time() {
        match Expr::parse("upper(data.a, data.b)") {
            Err(ExprError::WrongArity {
                function,
                expected,
                actual,
            }) => {
                assert_eq!(function, "upper");
                assert_eq!(expected, 1);
                assert_eq!(actual, 2);
            }
            other => panic!("expected WrongArity, got {other:?}"),
        }
    }

    #[test]
    fn trailing_input_is_rejected() {
        assert!(matches!(
            Expr::parse("data.a extra"),
            Err(ExprError::TrailingInput { .. })
        ));
    }

    #[test]
    fn empty_source_is_rejected() {
        assert!(matches!(Expr::parse("   "), Err(ExprError::Empty)));
    }

    #[test]
    fn source_over_the_length_ceiling_is_rejected_by_name() {
        // Non-whitespace filler: the length check runs on the *trimmed*
        // source, before parsing, so padding with spaces (which `trim()`
        // would strip right back off) would not actually test the bound.
        let long = "a".repeat(MAX_SOURCE_LEN + 1);
        match Expr::parse(&long) {
            Err(ExprError::TooLong { limit, actual }) => {
                assert_eq!(limit, MAX_SOURCE_LEN);
                assert_eq!(actual, MAX_SOURCE_LEN + 1);
            }
            other => panic!("expected TooLong, got {other:?}"),
        }
    }

    #[test]
    fn too_many_path_segments_is_rejected_by_name() {
        let path = "data".to_string() + &".a".repeat(MAX_PATH_SEGMENTS + 1);
        match Expr::parse(&path) {
            Err(ExprError::TooManyPathSegments { limit, .. }) => {
                assert_eq!(limit, MAX_PATH_SEGMENTS);
            }
            other => panic!("expected TooManyPathSegments, got {other:?}"),
        }
    }

    // -- Step 6: hostile inputs. None may panic; each names an error or
    // evaluates to Missing.

    #[test]
    fn a_missing_key_is_missing_not_an_error() {
        let data = serde_json::json!({ "a": 1 });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("data.nonexistent.deeper")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Missing);
    }

    #[test]
    fn a_json_null_is_missing() {
        let data = serde_json::json!({ "a": null });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("data.a")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Missing);
    }

    #[test]
    fn an_array_where_an_object_is_expected_is_missing_not_a_panic() {
        let data = serde_json::json!({ "a": [1, 2, 3] });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        // `.b` on an array: neither an object-field nor an array-index step.
        let value = Expr::parse("data.a.b")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Missing);
    }

    #[test]
    fn an_object_where_an_index_is_expected_is_missing_not_a_panic() {
        let data = serde_json::json!({ "a": { "b": 1 } });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("data.a[0]")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Missing);
    }

    #[test]
    fn an_out_of_range_index_is_missing_not_a_panic() {
        let data = serde_json::json!({ "rows": [1, 2] });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("data.rows[99]")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Missing);
    }

    #[test]
    fn a_number_where_text_is_expected_is_missing_not_a_panic() {
        let data = serde_json::json!({ "a": 42 });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("upper(data.a)")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Missing);
    }

    #[test]
    fn borrowed_provider_text_is_checked_before_the_evaluator_clones_it() {
        let oversized = "x".repeat(MAX_OUTPUT_LEN + 1);
        assert_eq!(
            check_text_len(&oversized),
            Err(ExprError::OutputTooLong {
                limit: MAX_OUTPUT_LEN,
                actual: MAX_OUTPUT_LEN + 1,
            })
        );
    }

    #[test]
    fn a_ten_megabyte_provider_string_is_rejected_without_an_evaluator_clone() {
        // Controller ruling (fix round 1): a value this oversized was never
        // asked to be shortened, so it must not be silently truncated
        // (indistinguishable from `truncate(s, n)`'s deliberate behaviour)
        // or reported as `Missing` (which would claim the data isn't
        // there, when it is -- just too large). A named error is the only
        // outcome that stays both total (no panic) and honest. The JSON
        // value necessarily owns these bytes already; the regression is
        // that evaluation must reject its borrowed length before cloning
        // another 10 MiB allocation.
        let huge = "x".repeat(10 * 1024 * 1024);
        let data = serde_json::json!({ "huge": huge });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let err = Expr::parse("data.huge")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap_err();
        match err {
            ExprError::OutputTooLong { limit, actual } => {
                assert_eq!(limit, MAX_OUTPUT_LEN);
                assert_eq!(actual, 10 * 1024 * 1024);
            }
            other => panic!("expected OutputTooLong, got {other:?}"),
        }
    }

    #[test]
    fn exactly_the_output_ceiling_is_accepted_byte_for_byte() {
        let exact = "y".repeat(MAX_OUTPUT_LEN);
        let data = serde_json::json!({ "s": exact.clone() });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("data.s")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Text(exact));
    }

    #[test]
    fn one_byte_over_the_output_ceiling_is_rejected_by_name() {
        let one_over = "y".repeat(MAX_OUTPUT_LEN + 1);
        let data = serde_json::json!({ "s": one_over });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let err = Expr::parse("data.s")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap_err();
        match err {
            ExprError::OutputTooLong { limit, actual } => {
                assert_eq!(limit, MAX_OUTPUT_LEN);
                assert_eq!(actual, MAX_OUTPUT_LEN + 1);
            }
            other => panic!("expected OutputTooLong, got {other:?}"),
        }
    }

    #[test]
    fn a_nan_number_is_missing_not_a_panic_or_a_propagated_nan() {
        // JSON itself cannot encode NaN (serde_json::Number rejects it), so
        // this exercises `call_round` directly, the way an evaluator that
        // ever gains an arithmetic function producing a non-finite
        // intermediate would reach it.
        let x = EvalValue::Number(f64::NAN);
        let places = EvalValue::Number(2.0);
        assert_eq!(call_round(&x, &places), EvalValue::Missing);
    }

    #[test]
    fn round_rejects_a_nan_places_argument_too() {
        let x = EvalValue::Number(1.0);
        let places = EvalValue::Number(f64::NAN);
        assert_eq!(call_round(&x, &places), EvalValue::Missing);
    }

    #[test]
    fn round_rejects_places_outside_the_bound() {
        let x = EvalValue::Number(1.0);
        let places = EvalValue::Number(f64::from(MAX_ROUND_PLACES) + 1.0);
        assert_eq!(call_round(&x, &places), EvalValue::Missing);
    }

    #[test]
    fn truncate_is_char_boundary_safe_on_multibyte_text() {
        let data = serde_json::json!({ "s": "héllo wörld" });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("truncate(data.s, 3)")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Text("hél".to_string()));
    }

    #[test]
    fn numbers_as_strings_do_not_silently_coerce() {
        // A real API returning a numeric-looking value as a JSON string is
        // exactly the shape finding 3 warns about: the evaluator must
        // surface it as Text, not silently parse it as a Number.
        let data = serde_json::json!({ "legacy_index": "42" });
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = Expr::parse("data.legacy_index")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(value, EvalValue::Text("42".to_string()));

        let mut fuel = Fuel::new(FUEL_BUDGET);
        let rounded = Expr::parse("round(data.legacy_index, 0)")
            .unwrap()
            .eval(&ctx, &mut fuel)
            .unwrap();
        assert_eq!(rounded, EvalValue::Missing);
    }
}
