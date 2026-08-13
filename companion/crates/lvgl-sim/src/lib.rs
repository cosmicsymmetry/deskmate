//! Headless renderer for the firmware's LVGL templates.
//!
//! This crate links the firmware's own C sources (LVGL, the template
//! renderers under `firmware/main/ui/templates`, and the hardware-independent
//! core under `firmware/main/core`) and drives them from Rust so the exact
//! on-device pixels can be produced on the host for previews and tests. See
//! `build.rs` for the source list and `csrc/sim_shim.c` for the C-side glue.

use std::ffi::CString;
use std::fmt;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The golden-frame case table shared by this crate's `tests/golden.rs` and
/// `companion/crates/device/examples/framebuffer_diff.rs` (Task 10). See
/// `cases.rs`'s module doc for why it lives here instead of under `tests/`.
pub mod cases;

pub const LOGICAL_WIDTH: u32 = 448;
pub const LOGICAL_HEIGHT: u32 = 368;

/// A firmware template kind. Values mirror `protocol_template_kind_t` in
/// `firmware/main/core/protocol_message.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimTemplate {
    DigitalClock,
    ProgressRing,
    RowList,
    AnalogClock,
    BigNumberLabel,
    IconBadgeText,
}

/// A field value pinned for one render. Kind mirrors `protocol_field_type_t`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimFieldValue {
    Text(String),
    Integer(i64),
    Boolean(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimField {
    pub name: String,
    pub value: SimFieldValue,
}

/// The device mounting orientation. `LandscapeFlipped` is the 270° mount
/// (180° rotation of the logical canvas); see the "flipped orientation"
/// note in `csrc/sim_shim.c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimOrientation {
    Landscape,
    LandscapeFlipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRequest {
    pub template: SimTemplate,
    pub fields: Vec<SimField>,
    pub utc_offset_minutes: i16,
    pub now_unix_seconds: i64,
    pub orientation: SimOrientation,
}

/// Errors from simulator setup or rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimError {
    /// A `Simulator` already exists in this process; LVGL's global state
    /// (via the shim's static display/frame buffers) allows exactly one.
    AlreadyClaimed,
    /// `sim_init` (`lv_init` + headless display setup) failed.
    InitFailed,
    /// `sim_render` failed: unknown template kind or field resolution
    /// failure.
    RenderFailed,
    /// PNG encoding of a successfully rendered frame failed.
    EncodeFailed,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            SimError::AlreadyClaimed => "a Simulator already exists in this process",
            SimError::InitFailed => "LVGL simulator initialization failed",
            SimError::RenderFailed => "template render failed",
            SimError::EncodeFailed => "PNG encoding failed",
        };
        f.write_str(message)
    }
}

impl std::error::Error for SimError {}

/// A field passed across the FFI boundary. Kept alive only for the duration
/// of one `sim_render` call; `name`/`text` point into the `CString`s held by
/// the caller in `Simulator::render`.
#[repr(C)]
struct RawField {
    name: *const c_char,
    kind: i32,
    text: *const c_char,
    integer: i64,
    boolean: bool,
}

// SAFETY: this block declares the C shim's signatures exactly as defined in
// `csrc/sim_shim.h` / `csrc/sim_shim.c`, compiled and linked in by `build.rs`.
unsafe extern "C" {
    fn sim_init() -> bool;

    fn sim_render(
        template_kind: i32,
        fields: *const RawField,
        field_count: usize,
        utc_offset_minutes: i16,
        now_unix_seconds: i64,
        orientation_flipped: bool,
        out_pixels: *mut u16,
    ) -> bool;
}

static SIMULATOR_CLAIMED: AtomicBool = AtomicBool::new(false);

/// Owns all LVGL state; not `Sync`. At most one instance may be live at a
/// time in the process: the C shim keeps its display and frame buffer in
/// process-global statics (see `csrc/sim_shim.c`), so two live `Simulator`s
/// would silently share state. `Simulator::new` enforces this with
/// `SIMULATOR_CLAIMED`, and `Drop` releases the claim so a later `Simulator`
/// can be constructed; the underlying LVGL display itself is never torn
/// down, matching LVGL's lack of a clean `lv_deinit` story for a single
/// static display.
pub struct Simulator {
    // Presence of a raw-pointer-shaped field would suffice, but this makes
    // intent explicit: `Simulator` must never be `Sync` (`sim_render` is not
    // reentrant) and should not be assumed `Send` either, since the shim's
    // LVGL state is only ever touched from one thread. Neither is
    // implemented automatically because LVGL's C statics carry no such
    // guarantee.
    _not_sync: std::marker::PhantomData<*mut ()>,
}

impl Simulator {
    /// Claims the process-wide simulator slot and initializes LVGL's
    /// headless display. Returns `SimError::AlreadyClaimed` if a `Simulator`
    /// is still claimed after a bounded wait (see the module-level note on
    /// `SIMULATOR_CLAIMED`).
    pub fn new() -> Result<Simulator, SimError> {
        // The claim is a simple `AtomicBool` swap, as it must be: the C
        // shim's LVGL state (display, frame buffer) lives in process-global
        // statics with no synchronization of its own, so at most one
        // `Simulator` may exist at a time. Unlike a plain non-blocking swap,
        // this retries for a bounded window: cargo's default test harness
        // runs a crate's tests on separate threads concurrently, and this
        // crate's own tests each construct an independent `Simulator` and
        // expect to succeed. A blocking wait lets the first `Simulator`'s
        // `Drop` (below) release the claim before the second attempt gives
        // up, so ordinary contention serializes instead of failing; a
        // `Simulator` that is never dropped (a real bug) still surfaces as
        // `AlreadyClaimed` once the deadline passes.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if !SIMULATOR_CLAIMED.swap(true, Ordering::SeqCst) {
                break;
            }
            if Instant::now() >= deadline {
                return Err(SimError::AlreadyClaimed);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        // SAFETY: `sim_init` has no preconditions beyond being called from a
        // single thread at a time, which the claim above enforces across
        // `Simulator` instances; it is idempotent (guards on `s_display`).
        if !unsafe { sim_init() } {
            SIMULATOR_CLAIMED.store(false, Ordering::SeqCst);
            return Err(SimError::InitFailed);
        }
        Ok(Simulator {
            _not_sync: std::marker::PhantomData,
        })
    }

    /// Renders one template with the given fields at a pinned instant.
    /// Returns 448*368 RGB565 pixels in logical landscape orientation.
    pub fn render(&mut self, request: &RenderRequest) -> Result<Vec<u16>, SimError> {
        let template_kind = match request.template {
            SimTemplate::DigitalClock => 1,
            SimTemplate::ProgressRing => 2,
            SimTemplate::RowList => 3,
            SimTemplate::AnalogClock => 4,
            SimTemplate::BigNumberLabel => 5,
            SimTemplate::IconBadgeText => 6,
        };

        // Keep CStrings alive across the call: `RawField` below only holds
        // pointers into them.
        let names: Vec<CString> = request
            .fields
            .iter()
            .map(|field| CString::new(field.name.as_str()).unwrap_or_default())
            .collect();
        let texts: Vec<CString> = request
            .fields
            .iter()
            .map(|field| match &field.value {
                SimFieldValue::Text(text) => CString::new(text.as_str()).unwrap_or_default(),
                SimFieldValue::Integer(_) | SimFieldValue::Boolean(_) => CString::default(),
            })
            .collect();
        let raw: Vec<RawField> = request
            .fields
            .iter()
            .zip(names.iter())
            .zip(texts.iter())
            .map(|((field, name), text)| {
                let (kind, integer, boolean) = match &field.value {
                    SimFieldValue::Text(_) => (0, 0, false),
                    SimFieldValue::Integer(value) => (1, *value, false),
                    SimFieldValue::Boolean(value) => (2, 0, *value),
                };
                RawField {
                    name: name.as_ptr(),
                    kind,
                    text: text.as_ptr(),
                    integer,
                    boolean,
                }
            })
            .collect();

        let mut pixels = vec![0_u16; (LOGICAL_WIDTH * LOGICAL_HEIGHT) as usize];
        // SAFETY: `raw` and the `CString`s backing its pointers are alive for
        // the duration of this call; `pixels` has exactly
        // `LOGICAL_WIDTH * LOGICAL_HEIGHT` elements, matching what
        // `sim_render` writes (SIM_WIDTH * SIM_HEIGHT in the shim).
        let ok = unsafe {
            sim_render(
                template_kind,
                raw.as_ptr(),
                raw.len(),
                request.utc_offset_minutes,
                request.now_unix_seconds,
                matches!(request.orientation, SimOrientation::LandscapeFlipped),
                pixels.as_mut_ptr(),
            )
        };
        if ok {
            Ok(pixels)
        } else {
            Err(SimError::RenderFailed)
        }
    }

    /// Renders like [`Simulator::render`] and encodes the result as an 8-bit
    /// RGB PNG.
    pub fn render_png(&mut self, request: &RenderRequest) -> Result<Vec<u8>, SimError> {
        let pixels = self.render(request)?;
        let mut rgb = Vec::with_capacity(pixels.len() * 3);
        for pixel in &pixels {
            rgb.push((((pixel >> 11) & 0x1f) as u8) << 3); // R5 -> 8
            rgb.push((((pixel >> 5) & 0x3f) as u8) << 2); // G6 -> 8
            rgb.push(((pixel & 0x1f) as u8) << 3); // B5 -> 8
        }
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, LOGICAL_WIDTH, LOGICAL_HEIGHT);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(|_| SimError::EncodeFailed)?;
            writer
                .write_image_data(&rgb)
                .map_err(|_| SimError::EncodeFailed)?;
        }
        Ok(out)
    }
}

impl Drop for Simulator {
    /// Releases the process-wide claim so a later `Simulator::new` (from
    /// this or another thread) can proceed. The C shim's own state (LVGL
    /// display, frame buffer) is intentionally left initialized: `sim_init`
    /// is idempotent and re-using it is cheaper and simpler than teardown,
    /// which LVGL does not cleanly support for a single static display.
    fn drop(&mut self) {
        SIMULATOR_CLAIMED.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digital_clock_renders_deterministically_and_not_blank() {
        let mut sim = Simulator::new().expect("simulator");
        let request = RenderRequest {
            template: SimTemplate::DigitalClock,
            fields: vec![
                SimField {
                    name: "title".into(),
                    value: SimFieldValue::Text("Desk".into()),
                },
                SimField {
                    name: "show_seconds".into(),
                    value: SimFieldValue::Boolean(true),
                },
            ],
            utc_offset_minutes: 240,
            now_unix_seconds: 1_755_000_000, // 2025-08-12 12:00:00 UTC -> 16:00 local
            orientation: SimOrientation::Landscape,
        };
        let first = sim.render(&request).expect("render");
        assert_eq!(first.len(), (LOGICAL_WIDTH * LOGICAL_HEIGHT) as usize);
        // Not blank: some pixel differs from the background.
        let background = first[0];
        assert!(first.iter().any(|pixel| *pixel != background));
        // Deterministic: same request, same pixels.
        let second = sim.render(&request).expect("render again");
        assert_eq!(first, second);
    }

    #[test]
    fn flipped_orientation_is_a_180_rotation_of_landscape() {
        let mut sim = Simulator::new().expect("simulator");
        let mut request = RenderRequest {
            template: SimTemplate::BigNumberLabel,
            fields: vec![
                SimField {
                    name: "title".into(),
                    value: SimFieldValue::Text("Steps".into()),
                },
                SimField {
                    name: "value".into(),
                    value: SimFieldValue::Text("8123".into()),
                },
                SimField {
                    name: "label".into(),
                    value: SimFieldValue::Text("today".into()),
                },
            ],
            utc_offset_minutes: 0,
            now_unix_seconds: 1_755_000_000,
            orientation: SimOrientation::Landscape,
        };
        let plain = sim.render(&request).expect("render");
        request.orientation = SimOrientation::LandscapeFlipped;
        let flipped = sim.render(&request).expect("render flipped");
        let reversed: Vec<u16> = plain.iter().rev().copied().collect();
        assert_eq!(flipped, reversed);
    }
}
