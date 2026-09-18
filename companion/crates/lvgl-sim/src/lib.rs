//! Headless renderer for Deskmate's LVGL surfaces.
//!
//! This crate links the shipping scene interpreter and the hardware-independent
//! firmware core, so Rust can produce exact device pixels: the goldens, the
//! web preview and the on-board framebuffer diff all render through
//! the same C the device runs. See `build.rs` for the source list and
//! `csrc/sim_shim.c` for the C-side glue.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Shared render cases used by golden tests and the physical framebuffer
/// harness.
pub mod cases;

/// The runtime font asset used by the 72px asset-backed SceneText golden.
/// See its module doc for why the vendored TTF is patched before being subset.
mod assets;

/// Rendering a declarative scene through the firmware's own decoder and
/// `ui/scene_view.c` interpreter. See its module doc for why it goes through the wire format
/// rather than filling a `scene_t` over FFI.
pub mod scene;

pub const LOGICAL_WIDTH: u32 = 448;
pub const LOGICAL_HEIGHT: u32 = 368;

/// The device mounting orientation. `LandscapeFlipped` is the 270° mount
/// (180° rotation of the logical canvas); see the "flipped orientation"
/// note in `csrc/sim_shim.c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimOrientation {
    Landscape,
    LandscapeFlipped,
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
    /// PNG encoding of a successfully rendered frame failed.
    EncodeFailed,
    /// The scene failed `protocol::validate_scene`, the host mirror of
    /// the device's `scene_model_validate()`. Caught before encoding, so the
    /// reason survives; the device would have refused the same scene with no
    /// reason attached.
    SceneInvalid(protocol::MessageError),
    /// An asset a scene names could not be put in the simulator's
    /// asset store — a full store, or a malformed image.
    AssetRegistrationFailed,
    /// `sim_render_scene`'s own `sim_init`, asset store, or font
    /// registry setup failed. Nothing about this scene's bytes or content —
    /// see `SceneDecodeFailed` and `SceneRenderRefused` for those.
    SceneSetupFailed,
    /// `firmware/main/core/scene_decode.c`'s `scene_decode()` refused
    /// the encoded payload. This is the failure mode the encode/decode design
    /// introduces — the encoder and decoder disagreeing about the wire shape
    /// — and it must stay distinguishable from `SceneRenderRefused` (a
    /// drawing-time failure) without a debugger.
    SceneDecodeFailed(scene::SceneDecodeReason),
    /// `firmware/main/ui/scene_view.c`'s `scene_view_show()` refused
    /// the decoded scene — an asset it could not acquire, or an allocation
    /// failure. The payload decoded fine; drawing it did not work.
    SceneRenderRefused,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            SimError::AlreadyClaimed => "a Simulator already exists in this process",
            SimError::InitFailed => "LVGL simulator initialization failed",
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

// SAFETY: this block declares the C shim's signatures exactly as defined in
// `csrc/sim_shim.h` / `csrc/sim_shim.c`, compiled and linked in by `build.rs`.
unsafe extern "C" {
    fn sim_init() -> bool;

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
    // intent explicit: `Simulator` must never be `Sync` (the renderer is not
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
}

/// Encodes 448*368 RGB565 pixels (logical landscape, as returned by
/// [`Simulator::render_scene`]) as an 8-bit RGB PNG.
///
/// This is public so hardware/parity diagnostics can write failure frames
/// with the same bit-identical encoder as the simulator's golden suites.
///
/// # Errors
///
/// Returns [`SimError::EncodeFailed`] if the PNG header or image data cannot
/// be encoded.
pub fn pixels_to_png(pixels: &[u16]) -> Result<Vec<u8>, SimError> {
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

    fn face_case(name: &str) -> crate::scene::SceneRenderRequest {
        crate::cases::face_scene_cases()
            .into_iter()
            .find(|(case_name, _)| case_name == name)
            .unwrap_or_else(|| panic!("no face case named {name}"))
            .1
    }

    #[test]
    fn digital_clock_renders_deterministically_and_not_blank() {
        let mut sim = Simulator::new().expect("simulator");
        let request = face_case("digital-clock--typical--landscape");
        let first = sim.render_scene(&request).expect("render");
        assert_eq!(first.len(), (LOGICAL_WIDTH * LOGICAL_HEIGHT) as usize);
        // Not blank: some pixel differs from the background.
        let background = first[0];
        assert!(first.iter().any(|pixel| *pixel != background));
        // Deterministic: same request, same pixels.
        let second = sim.render_scene(&request).expect("render again");
        assert_eq!(first, second);
    }

    /// `copy_frame_out` builds the flipped frame by reversing the finished
    /// buffer index-by-index, which is why a flipped golden would pin nothing
    /// a landscape one does not. Everything downstream -- the preview's
    /// upright-at-both-mountings rule, `framebuffer_diff`'s orientation rows --
    /// rests on that being a pure 180 degree rotation, so it is pinned here.
    #[test]
    fn flipped_orientation_is_a_180_rotation_of_landscape() {
        let mut sim = Simulator::new().expect("simulator");
        let plain = sim
            .render_scene(&face_case("analog-clock--typical--landscape"))
            .expect("render");
        let flipped = sim
            .render_scene(&face_case("analog-clock--typical--flipped"))
            .expect("render flipped");
        let reversed: Vec<u16> = plain.iter().rev().copied().collect();
        assert_eq!(flipped, reversed);
    }
}
