//! Task 8 (stage 2a): rendering a declarative scene through the firmware's own
//! interpreter.
//!
//! The device draws a scene by decoding a CBOR display list into a `scene_t`
//! and handing it to `firmware/main/ui/scene_view.c`. This module does exactly
//! that, on the host, with the same C: `build.rs` compiles `core/scene_model.c`,
//! `core/scene_decode.c`, `core/scene_binding.c` and `ui/scene_view.c` into this
//! crate, and [`Simulator::render_scene`] feeds them the bytes
//! [`protocol::encode_scene_payload`] writes.
//!
//! # Why it goes through the decoder
//!
//! Filling a `scene_t` field by field over FFI would have been less code. It
//! would also have made the stage-2a parity gate compare a render of the wire
//! format against a render of a hand-built struct — two things that agree only
//! as long as whoever wrote the FFI marshalling got every field right. Encoding
//! and decoding instead means the simulator and the device build their
//! `scene_t` from the same bytes with the same decoder, so the only thing left
//! that can differ is the drawing.
//!
//! # Assets
//!
//! `image` and asset-font nodes name content by digest, and nothing on the host
//! resolves a digest by itself. [`SceneAsset`] entries are registered into
//! `csrc/sim_shim.c`'s RAM-backed asset store (the same
//! `firmware/main/core/asset_store.c` the device uses, over a heap buffer
//! instead of flash — see that file's Task 12 section) before the scene is
//! decoded, and `sim_render_scene` wires that store to `scene_view.c` through
//! `scene_view_set_asset_resolver`. Without that call an `image` node refuses
//! the whole scene; the shim makes it unconditionally rather than only when a
//! scene happens to contain one.

use std::ffi::CString;
use std::os::raw::c_char;

use protocol::{Scene, encode_scene_payload};

use crate::{LOGICAL_HEIGHT, LOGICAL_WIDTH, SimError, SimOrientation, Simulator, pixels_to_png};

/// `ASSET_KIND_FONT` in `firmware/main/core/asset_store.h`.
const ASSET_KIND_FONT: u8 = 1;
/// `ASSET_KIND_IMAGE` in `firmware/main/core/asset_store.h`.
const ASSET_KIND_IMAGE: u8 = 3;

/// Slack over the pixel data for the `lv_image_header_t` the shim prepends.
/// The real header is 12 bytes; this is deliberately loose because its size is
/// the C compiler's business and `sim_build_rgb565_image` refuses to write past
/// the capacity it is given, so an over-allocation costs a few bytes and an
/// under-allocation would cost a confusing failure.
const IMAGE_HEADER_SLACK: usize = 64;

/// A running timer, as the device's `scene_binding_context_t` carries one.
/// `None` on a [`SceneRenderRequest`] means no timer is active, which is what
/// makes `timer.remaining:` render `--:--` and `timer.pct` render `--`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneTimer {
    pub remaining_ms: u32,
    /// 0..=100. An `arc` node's `end_binding` scales its declared sweep by this.
    pub pct: u8,
}

/// Content a scene names by digest, registered before the scene is decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneAsset {
    /// A TTF, registered as `ASSET_KIND_FONT` and reached through
    /// `font_registry_acquire` — the path a `glyph` node always takes and an
    /// asset-font `text` node takes.
    ///
    /// `'static` because every font the simulator has is an `include_bytes!`
    /// (see [`crate::assets::INTER_SUBSET_TTF`]), matching
    /// [`crate::cases::AssetFontCase`].
    Font {
        digest: [u8; 32],
        bytes: &'static [u8],
    },
    /// Host-endian RGB565 pixels, wrapped in the LVGL binary image layout by
    /// the shim and registered as `ASSET_KIND_IMAGE`. The wrapper is built in C
    /// because `lv_image_header_t` is a bitfield struct whose byte layout is
    /// the compiler's to decide.
    Image {
        digest: [u8; 32],
        width: u32,
        height: u32,
        /// Exactly `width * height` entries, row-major.
        pixels: Vec<u16>,
    },
}

/// One scene render. Mirrors [`crate::RenderRequest`]'s shape: everything the
/// frame depends on, pinned, so the render is a pure function of the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneRenderRequest {
    pub scene: Scene,
    /// Registered before the scene is decoded. Registration is idempotent per
    /// digest, so a case may list its assets on every render without tracking
    /// whether this is the first.
    pub assets: Vec<SceneAsset>,
    pub utc_offset_minutes: i16,
    pub now_unix_seconds: i64,
    pub timer: Option<SceneTimer>,
    /// `field.<name>` bindings resolve against these. A name that is absent
    /// renders the device's `--` placeholder rather than failing.
    pub fields: Vec<(String, String)>,
    pub orientation: SimOrientation,
}

/// A `field.<name>` binding's value across the FFI boundary. Kept alive only
/// for the duration of one `sim_render_scene` call; both pointers point into
/// `CString`s held by [`Simulator::render_scene`].
#[repr(C)]
struct RawSceneField {
    name: *const c_char,
    value: *const c_char,
}

// SAFETY: this block declares csrc/sim_shim.c's Task 8 additions exactly as
// defined in csrc/sim_shim.h, compiled and linked in by build.rs alongside the
// rest of the shim's extern "C" surface (see lib.rs's own such block).
unsafe extern "C" {
    fn sim_asset_register(digest: *const u8, bytes: *const u8, len: u32, kind: u8) -> bool;

    fn sim_build_rgb565_image(
        width: i32,
        height: i32,
        pixels: *const u16,
        out: *mut u8,
        out_capacity: usize,
    ) -> usize;

    #[allow(clippy::too_many_arguments)]
    fn sim_render_scene(
        payload: *const u8,
        payload_length: usize,
        utc_offset_minutes: i16,
        now_unix_seconds: i64,
        timer_active: bool,
        timer_remaining_ms: u32,
        timer_pct: u8,
        fields: *const RawSceneField,
        field_count: usize,
        orientation_flipped: bool,
        out_pixels: *mut u16,
    ) -> bool;
}

impl Simulator {
    /// Registers `request`'s assets, encodes its scene as the standalone CBOR
    /// payload the device's `scene_decode()` takes, and renders the result
    /// through `ui/scene_view.c`. Returns 448*368 RGB565 pixels in logical
    /// landscape orientation, like [`Simulator::render`].
    ///
    /// # Errors
    ///
    /// [`SimError::SceneInvalid`] if the scene fails the protocol validator
    /// (which mirrors `scene_model_validate`, so this is the same refusal the
    /// device would issue, just earlier and with a reason attached);
    /// [`SimError::AssetRegistrationFailed`] if an asset could not be stored;
    /// [`SimError::RenderFailed`] if the payload does not decode or
    /// `scene_view_show()` refuses it — an asset it could not acquire, or an
    /// allocation failure.
    pub fn render_scene(&mut self, request: &SceneRenderRequest) -> Result<Vec<u16>, SimError> {
        for asset in &request.assets {
            register_asset(asset)?;
        }

        let payload = encode_scene_payload(&request.scene).map_err(SimError::SceneInvalid)?;

        // Keep the CStrings alive across the call: RawSceneField only holds
        // pointers into them.
        let names: Vec<CString> = request
            .fields
            .iter()
            .map(|(name, _)| crate::truncated_cstring(name))
            .collect();
        let values: Vec<CString> = request
            .fields
            .iter()
            .map(|(_, value)| crate::truncated_cstring(value))
            .collect();
        let raw: Vec<RawSceneField> = names
            .iter()
            .zip(values.iter())
            .map(|(name, value)| RawSceneField {
                name: name.as_ptr(),
                value: value.as_ptr(),
            })
            .collect();

        let timer = request.timer.unwrap_or(SceneTimer {
            remaining_ms: 0,
            pct: 0,
        });
        let mut pixels = vec![0_u16; (LOGICAL_WIDTH * LOGICAL_HEIGHT) as usize];
        // SAFETY: `payload`, `raw` and the CStrings backing its pointers are
        // alive for the duration of this call; `pixels` has exactly
        // LOGICAL_WIDTH * LOGICAL_HEIGHT elements, matching what
        // sim_render_scene writes (SIM_WIDTH * SIM_HEIGHT in the shim).
        let ok = unsafe {
            sim_render_scene(
                payload.as_ptr(),
                payload.len(),
                request.utc_offset_minutes,
                request.now_unix_seconds,
                request.timer.is_some(),
                timer.remaining_ms,
                timer.pct,
                raw.as_ptr(),
                raw.len(),
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

    /// Renders like [`Simulator::render_scene`] and encodes the result as an
    /// 8-bit RGB PNG, matching [`Simulator::render_png`].
    ///
    /// # Errors
    ///
    /// [`Simulator::render_scene`]'s errors, plus [`SimError::EncodeFailed`].
    pub fn render_scene_png(&mut self, request: &SceneRenderRequest) -> Result<Vec<u8>, SimError> {
        let pixels = self.render_scene(request)?;
        pixels_to_png(&pixels)
    }
}

fn register_asset(asset: &SceneAsset) -> Result<(), SimError> {
    let ok = match asset {
        SceneAsset::Font { digest, bytes } => {
            let len = u32::try_from(bytes.len()).map_err(|_| SimError::AssetRegistrationFailed)?;
            // SAFETY: `digest` is exactly ASSET_DIGEST_BYTES (32) long and
            // `bytes`/`len` describe a live slice for the duration of the call.
            unsafe { sim_asset_register(digest.as_ptr(), bytes.as_ptr(), len, ASSET_KIND_FONT) }
        }
        SceneAsset::Image {
            digest,
            width,
            height,
            pixels,
        } => {
            let expected = (*width as usize)
                .checked_mul(*height as usize)
                .ok_or(SimError::AssetRegistrationFailed)?;
            if pixels.len() != expected || expected == 0 {
                return Err(SimError::AssetRegistrationFailed);
            }
            let width = i32::try_from(*width).map_err(|_| SimError::AssetRegistrationFailed)?;
            let height = i32::try_from(*height).map_err(|_| SimError::AssetRegistrationFailed)?;

            let mut blob = vec![0_u8; pixels.len() * 2 + IMAGE_HEADER_SLACK];
            // SAFETY: `pixels` holds exactly width*height entries (checked
            // above) and `blob` is at least that many bytes plus header slack;
            // the shim writes no more than the capacity it is given and
            // reports how much it wrote.
            let written = unsafe {
                sim_build_rgb565_image(
                    width,
                    height,
                    pixels.as_ptr(),
                    blob.as_mut_ptr(),
                    blob.len(),
                )
            };
            if written == 0 {
                return Err(SimError::AssetRegistrationFailed);
            }
            blob.truncate(written);
            let len = u32::try_from(blob.len()).map_err(|_| SimError::AssetRegistrationFailed)?;
            // SAFETY: as above; `blob` is live for the duration of the call.
            unsafe { sim_asset_register(digest.as_ptr(), blob.as_ptr(), len, ASSET_KIND_IMAGE) }
        }
    };
    if ok {
        Ok(())
    } else {
        Err(SimError::AssetRegistrationFailed)
    }
}

#[cfg(test)]
mod tests {
    use protocol::{SceneNode, SceneRect};

    use super::*;

    fn request(scene: Scene) -> SceneRenderRequest {
        SceneRenderRequest {
            scene,
            assets: Vec::new(),
            utc_offset_minutes: 0,
            now_unix_seconds: 1_755_000_000,
            timer: None,
            fields: Vec::new(),
            orientation: SimOrientation::Landscape,
        }
    }

    /// The whole point of the module: a scene reaches the panel through the
    /// firmware's decoder and interpreter, and the same request twice gives the
    /// same pixels.
    #[test]
    fn a_scene_renders_deterministically_and_not_blank() {
        let mut sim = Simulator::new().expect("simulator");
        let scene = Scene {
            revision: 1,
            background: 0x0000_0000,
            nodes: vec![SceneNode::Rect(SceneRect {
                x: 64,
                y: 64,
                w: 128,
                h: 128,
                radius: 16,
                fill: 0x00ff_8f2e,
                opacity: 255,
            })],
        };
        let first = sim.render_scene(&request(scene.clone())).expect("render");
        assert_eq!(first.len(), (LOGICAL_WIDTH * LOGICAL_HEIGHT) as usize);
        let background = first[0];
        assert!(first.iter().any(|pixel| *pixel != background));
        let second = sim.render_scene(&request(scene)).expect("render again");
        assert_eq!(first, second);
    }

    /// A scene the protocol validator rejects never reaches the C at all, and
    /// says why. Without this the first sign would be a bare "render failed"
    /// from `scene_decode`.
    #[test]
    fn an_off_canvas_scene_is_refused_with_a_reason() {
        let mut sim = Simulator::new().expect("simulator");
        let scene = Scene {
            revision: 1,
            background: 0,
            nodes: vec![SceneNode::Rect(SceneRect {
                x: 440,
                y: 0,
                w: 64,
                h: 64,
                ..SceneRect::default()
            })],
        };
        assert!(matches!(
            sim.render_scene(&request(scene)),
            Err(SimError::SceneInvalid(_))
        ));
    }

    /// An `image` node naming a digest nothing registered is refused whole --
    /// `scene_view_show` returns false rather than drawing the other nodes
    /// without it. Pinned because it is also the failure a missing
    /// `scene_view_set_asset_resolver` call would produce, and the two must not
    /// be confusable later.
    #[test]
    fn an_unregistered_image_digest_refuses_the_scene() {
        let mut sim = Simulator::new().expect("simulator");
        let scene = Scene {
            revision: 1,
            background: 0,
            nodes: vec![SceneNode::Image(protocol::SceneImage {
                x: 16,
                y: 16,
                w: 32,
                h: 32,
                digest: [0xAB; 32],
                recolor: false,
                color: 0,
            })],
        };
        assert_eq!(
            sim.render_scene(&request(scene)),
            Err(SimError::RenderFailed)
        );
    }

    /// The flip is the same 180° rotation of the logical canvas the template
    /// path applies, so an asymmetric scene must reverse exactly.
    #[test]
    fn flipped_orientation_is_a_180_rotation_of_landscape() {
        let mut sim = Simulator::new().expect("simulator");
        let scene = Scene {
            revision: 1,
            background: 0,
            nodes: vec![SceneNode::Rect(SceneRect {
                x: 0,
                y: 0,
                w: 96,
                h: 48,
                radius: 0,
                fill: 0x0000_ff00,
                opacity: 255,
            })],
        };
        let mut req = request(scene);
        let plain = sim.render_scene(&req).expect("render");
        req.orientation = SimOrientation::LandscapeFlipped;
        let flipped = sim.render_scene(&req).expect("render flipped");
        let reversed: Vec<u16> = plain.iter().rev().copied().collect();
        assert_eq!(flipped, reversed);
    }
}
