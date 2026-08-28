//! Headless renderer for Deskmate's LVGL surfaces.
//!
//! This crate links the shipping scene interpreter and hardware-independent
//! firmware core, plus the retired C templates from its clearly isolated
//! `reference-oracle/`. Rust can therefore produce exact device pixels and
//! keep comparing scenes against the live historical oracle. See `build.rs`
//! for the source list and `csrc/sim_shim.c` for the C-side glue.

use std::ffi::CString;
use std::fmt;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The reference-template golden cases and the scene cases used by the
/// physical framebuffer harness. See `cases.rs` for their distinct roles.
pub mod cases;

/// Task 12: the runtime font asset used by the asset-store parity golden,
/// and the `Simulator::render_asset_font_png` FFI wrapper for it. See its
/// module doc for why the vendored TTF is patched before being subset.
pub mod assets;

/// Task 5 (stage 3b): the simulator's half of §6's parity obligation --
/// resolving a plugin asset's digest to the same bytes a server-side
/// transfer path would push, sourced from `plugin::AssetSet` rather than a
/// second copy. See its module doc for why an unknown digest is a named
/// error rather than a silent fallback to a baked face.
pub mod asset_shim;

/// Task 8: rendering a declarative scene through the firmware's own decoder
/// and `ui/scene_view.c` interpreter, the device-side half of the plugin
/// display list. See its module doc for why it goes through the wire format
/// rather than filling a `scene_t` over FFI.
pub mod scene;

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
///
/// Deliberately not `Copy`: [`SimError::SceneInvalid`] carries the protocol
/// validator's own reason, and losing that reason to keep the enum a scalar
/// would trade the only diagnostic a rejected scene has for nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Task 8: the scene failed `protocol::validate_scene`, the host mirror of
    /// the device's `scene_model_validate()`. Caught before encoding, so the
    /// reason survives; the device would have refused the same scene with no
    /// reason attached.
    SceneInvalid(protocol::MessageError),
    /// Task 8: an asset a scene names could not be put in the simulator's
    /// asset store — a full store, or a malformed image.
    AssetRegistrationFailed,
    /// Task 8: `sim_render_scene`'s own `sim_init`, asset store, or font
    /// registry setup failed. Nothing about this scene's bytes or content —
    /// see `SceneDecodeFailed` and `SceneRenderRefused` for those.
    SceneSetupFailed,
    /// Task 8: `firmware/main/core/scene_decode.c`'s `scene_decode()` refused
    /// the encoded payload. This is the failure mode the encode/decode design
    /// introduces — the encoder and decoder disagreeing about the wire shape
    /// — and it must stay distinguishable from `SceneRenderRefused` (a
    /// drawing-time failure) without a debugger.
    SceneDecodeFailed(scene::SceneDecodeReason),
    /// Task 8: `firmware/main/ui/scene_view.c`'s `scene_view_show()` refused
    /// the decoded scene — an asset it could not acquire, or an allocation
    /// failure. The payload decoded fine; drawing it did not work.
    SceneRenderRefused,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            SimError::AlreadyClaimed => "a Simulator already exists in this process",
            SimError::InitFailed => "LVGL simulator initialization failed",
            SimError::RenderFailed => "template render failed",
            SimError::EncodeFailed => "PNG encoding failed",
            SimError::SceneInvalid(reason) => {
                return write!(f, "scene rejected by the protocol validator: {reason}");
            }
            SimError::AssetRegistrationFailed => "scene asset registration failed",
            SimError::SceneSetupFailed => "scene renderer setup failed",
            SimError::SceneDecodeFailed(reason) => {
                return write!(f, "scene payload rejected by scene_decode(): {reason}");
            }
            SimError::SceneRenderRefused => "scene_view_show() refused the decoded scene",
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

/// Builds a `CString` from `s`, truncating at the first interior NUL byte
/// instead of failing the whole string. `CString::new` rejects any interior
/// NUL, and the call sites used to fall back to `.unwrap_or_default()` on
/// that error — silently rendering an *empty* field for a string that has a
/// NUL anywhere in it. On device, the field text lands in a fixed C buffer
/// via `strncpy` (see `sim_shim.c`'s `build_fields`), which just stops
/// copying at the NUL and renders everything before it; truncating here
/// instead of blanking matches that behavior so the preview shows the same
/// prefix the firmware would.
pub(crate) fn truncated_cstring(s: &str) -> CString {
    match CString::new(s) {
        Ok(cstring) => cstring,
        Err(err) => {
            let nul_position = err.nul_position();
            // The bytes up to `nul_position` are exactly the prefix before
            // the first NUL that made `CString::new` fail, so they contain
            // no NUL themselves and this cannot fail.
            CString::new(&s.as_bytes()[..nul_position])
                .expect("prefix before first NUL cannot itself contain a NUL")
        }
    }
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
            .map(|field| truncated_cstring(field.name.as_str()))
            .collect();
        let texts: Vec<CString> = request
            .fields
            .iter()
            .map(|field| match &field.value {
                SimFieldValue::Text(text) => truncated_cstring(text.as_str()),
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
        pixels_to_png(&pixels)
    }
}

/// Encodes 448*368 RGB565 pixels (logical landscape, as returned by
/// [`Simulator::render`]) as an 8-bit RGB PNG. Shared by
/// [`Simulator::render_png`] and `assets::Simulator::render_asset_font_png`
/// (Task 12) so the two render paths produce bit-identical PNG encoding.
pub(crate) fn pixels_to_png(pixels: &[u16]) -> Result<Vec<u8>, SimError> {
    let mut rgb = Vec::with_capacity(pixels.len() * 3);
    for pixel in pixels {
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

    #[test]
    fn truncated_cstring_truncates_at_first_interior_nul() {
        let with_interior_nul = "Café\0Zürich";
        let result = truncated_cstring(with_interior_nul);
        assert_eq!(result.to_str().expect("valid utf-8"), "Café");

        // A string with no interior NUL is passed through unchanged.
        let clean = "Zürich";
        assert_eq!(
            truncated_cstring(clean).to_str().expect("valid utf-8"),
            clean
        );

        // A NUL as the very first byte truncates to empty, not a panic.
        assert_eq!(
            truncated_cstring("\0trailing")
                .to_str()
                .expect("valid utf-8"),
            ""
        );
    }
}
