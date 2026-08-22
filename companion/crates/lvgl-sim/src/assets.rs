//! Task 12: the runtime font asset used by the asset-store parity golden
//! (`crate::cases::asset_font_cases`, rendered by
//! [`Simulator::render_asset_font_png`]).
//!
//! Runtime fonts break the automatic simulator/firmware agreement that
//! `build.rs` otherwise gets for free by compiling the firmware's own C
//! sources: the font bytes now come from a content-addressed asset store
//! rather than a baked-in `deskmate_font_*.c` array, so nothing forces the
//! simulator to resolve a digest to the same bytes the device would. This
//! module — together with `csrc/sim_shim.c`'s RAM-backed asset store shim,
//! which reuses `firmware/main/core/asset_store.c` unmodified — discharges
//! that obligation explicitly (spec §6): the digest below is what
//! [`sim_asset_register`]/`font_registry_acquire` key on, on the host,
//! exactly as `protocol_asset_resolver`/`font_registry_acquire` do on
//! device.
//!
//! ## Why the vendored TTF is patched, not raw
//!
//! [`INTER_SUBSET_TTF`] is not a plain subset of
//! `tools/fonts/Inter-Regular.ttf`. `lv_tiny_ttf` (the runtime rasterizer
//! `font_registry_acquire` calls into, `firmware/main/ui/font_registry.c`)
//! wraps `stb_truetype`, which resolves glyphs by a raw `cmap` lookup and
//! never applies OpenType GSUB features. Inter's digits are *proportional*
//! by default — the tabular ("tnum") forms only become reachable once a
//! text shaper applies the `tnum` feature — so an unpatched Inter loaded at
//! runtime would render **proportional** digits, exactly the failure mode
//! `tools/genfonts.sh` and `tools/fonts/patch_tabular_figures.py` already
//! document and fix for the four baked `deskmate_font_*` faces
//! (`lv_font_conv` has the identical raw-cmap limitation). This golden
//! renders `12:34`, where digit advance width and colon alignment are
//! precisely what would regress if that patch were skipped.
//!
//! [`INTER_SUBSET_TTF`] was produced by, in order:
//!
//! 1. `python3 tools/fonts/patch_tabular_figures.py tools/fonts/Inter-Regular.ttf out.ttf`
//!    — repoints the digit codepoints (U+0030-U+0039) at Inter's tabular
//!    glyph outlines, the same transform `tools/genfonts.sh` runs before
//!    baking `deskmate_font_28`/`_56`/`_96`.
//! 2. `pyftsubset out.ttf --output-file=Inter-subset.ttf
//!    --unicodes="U+0020,U+0030-0039,U+003A,U+0041-005A" --layout-features=''
//!    --no-hinting --desubroutinize --drop-tables+=GSUB,GPOS,GDEF,DSIG
//!    --name-IDs='' --glyph-names`
//!    — down to exactly the glyphs this golden needs (digits, colon, A-Z,
//!    space); `stb_truetype` never reads GSUB/GPOS/GDEF, so dropping them
//!    (step 1's cmap rewrite already did the substitution work they would
//!    have driven) is safe. The result is ~4 KiB, comfortably under the
//!    64 KiB the task-12 brief allows, and its digit cmap entries point at
//!    Inter's `.tf` (tabular) glyphs, confirmed by inspection after
//!    subsetting.
//!
//! Licence: `assets/OFL.txt`, copied verbatim from `tools/fonts/OFL.txt`
//! (Inter is SIL Open Font License 1.1; a patched/subsetted derivative
//! remains covered by the same licence, which is why the licence file is
//! committed alongside the derived TTF rather than only alongside the
//! original).

use std::ffi::CString;
use std::os::raw::c_char;

use crate::cases::AssetFontCase;
use crate::{LOGICAL_HEIGHT, LOGICAL_WIDTH, SimError, SimOrientation, Simulator, pixels_to_png};

/// The patched + subset Inter TTF described in this module's doc comment.
/// Committed at `assets/Inter-subset.ttf`.
pub const INTER_SUBSET_TTF: &[u8] = include_bytes!("../assets/Inter-subset.ttf");

/// SHA-256 of [`INTER_SUBSET_TTF`], written down rather than computed at
/// test time — the digest is this asset's identity in the store (what
/// `sim_asset_register`/`font_registry_acquire` key on), so a silent file
/// replacement must show up as a hash mismatch instead of quietly changing
/// what golden case the digest names. `tests/asset_font.rs` asserts this
/// constant still matches the committed file. Regenerate both together with
/// `shasum -a 256 assets/Inter-subset.ttf` if the file is intentionally
/// replaced.
pub const INTER_SUBSET_SHA256: [u8; 32] = [
    0x40, 0xbb, 0xba, 0xc7, 0x15, 0x46, 0x5a, 0xdf, 0x7b, 0xa5, 0x39, 0xf5, 0x3a, 0x0c, 0xb1, 0x69,
    0x91, 0xa2, 0xc1, 0xf7, 0x3a, 0x4f, 0xb5, 0x7a, 0xf2, 0x09, 0xd3, 0x9e, 0x0c, 0x62, 0xb3, 0x27,
];

// SAFETY: declares csrc/sim_shim.c's Task 12 addition exactly as defined in
// csrc/sim_shim.h, compiled and linked in by build.rs alongside the rest of
// the shim's extern "C" surface (see lib.rs's own such block).
unsafe extern "C" {
    fn sim_render_asset_font(
        digest: *const u8,
        ttf_bytes: *const u8,
        ttf_len: u32,
        pixel_size: i32,
        text: *const c_char,
        orientation_flipped: bool,
        out_pixels: *mut u16,
    ) -> bool;
}

impl Simulator {
    /// Registers `case`'s font asset (idempotent per digest — see
    /// `sim_asset_register`'s own comment) and renders its text centred on
    /// the canvas at its pinned pixel size, through
    /// `font_registry_acquire` rather than any baked `deskmate_font_*`
    /// face. Encodes the result as an 8-bit RGB PNG, matching
    /// [`Simulator::render_png`].
    pub fn render_asset_font_png(&mut self, case: &AssetFontCase) -> Result<Vec<u8>, SimError> {
        let text = CString::new(case.text.as_str()).unwrap_or_default();
        let mut pixels = vec![0_u16; (LOGICAL_WIDTH * LOGICAL_HEIGHT) as usize];
        // SAFETY: `case.digest` is exactly ASSET_DIGEST_BYTES (32) long,
        // `case.ttf_bytes` outlives this call (`'static`), `text` is a live
        // CString for the duration of the call, and `pixels` has exactly
        // LOGICAL_WIDTH * LOGICAL_HEIGHT elements, matching what
        // sim_render_asset_font writes.
        let ok = unsafe {
            sim_render_asset_font(
                case.digest.as_ptr(),
                case.ttf_bytes.as_ptr(),
                u32::try_from(case.ttf_bytes.len()).unwrap_or(u32::MAX),
                case.pixel_size,
                text.as_ptr(),
                matches!(case.orientation, SimOrientation::LandscapeFlipped),
                pixels.as_mut_ptr(),
            )
        };
        if !ok {
            return Err(SimError::RenderFailed);
        }
        pixels_to_png(&pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inter_subset_sha256_matches_committed_file() {
        // Deliberately independent of `cases::asset_font_cases` /
        // `INTER_SUBSET_SHA256` as used for registration: this recomputes
        // the hash from the embedded bytes and checks it against the
        // written-down constant, so a replaced or corrupted asset file
        // fails loudly here instead of silently registering under a digest
        // that no longer describes its own bytes.
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(INTER_SUBSET_TTF);
        let computed: [u8; 32] = hasher.finalize().into();
        assert_eq!(
            computed, INTER_SUBSET_SHA256,
            "assets/Inter-subset.ttf no longer matches the recorded SHA-256 -- \
             update INTER_SUBSET_SHA256 if this file was intentionally replaced"
        );
    }
}
