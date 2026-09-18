//! The runtime font asset used by the 72px asset-backed SceneText golden
//! (`crate::cases::asset_font_scene_cases`).
//!
//! Runtime fonts break the automatic simulator/firmware agreement that
//! `build.rs` otherwise gets for free by compiling the firmware's own C
//! sources: the font bytes now come from a content-addressed asset store
//! rather than a baked-in `deskmate_font_*.c` array, so nothing forces the
//! simulator to resolve a digest to the same bytes the device would. This
//! module and the canonical [`crate::scene::SceneAsset`] value discharge that
//! obligation explicitly (spec §6): the digest below is what the shared scene
//! path keys on, exactly as the device does.
//!
//! ## Why the vendored TTF is patched, not raw
//!
//! [`INTER_SUBSET_TTF`] is not a plain subset of Inter Regular. `lv_tiny_ttf`
//! (the runtime rasterizer
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
//! precisely what would regress if that patch were skipped. The renderer only
//! needs this derived, independently licensed, content-addressed test asset at
//! build and runtime.
//!
//! [`INTER_SUBSET_TTF`] was produced by first repointing digit codepoints
//! U+0030-U+0039 at Inter's tabular glyph outlines, then running:
//!
//! `pyftsubset out.ttf --output-file=Inter-subset.ttf
//!    --unicodes="U+0020,U+0030-0039,U+003A,U+0041-005A" --layout-features=''
//!    --no-hinting --desubroutinize --drop-tables+=GSUB,GPOS,GDEF,DSIG
//!    --name-IDs='' --glyph-names`
//!
//! This reduces it to exactly the glyphs this golden needs (digits, colon, A-Z,
//!    space); `stb_truetype` never reads GSUB/GPOS/GDEF, so dropping them
//!    (step 1's cmap rewrite already did the substitution work they would
//!    have driven) is safe. The result is ~4 KiB, comfortably under the
//!    64 KiB the task-12 brief allows, and its digit cmap entries point at
//!    Inter's `.tf` (tabular) glyphs, confirmed by inspection after
//!    subsetting.
//!
//! Licence: `assets/OFL.txt`
//! (Inter is SIL Open Font License 1.1; a patched/subsetted derivative
//! remains covered by the same licence, which is why the licence file is
//! committed alongside the derived TTF rather than only alongside the
//! original).

/// The patched + subset Inter TTF described in this module's doc comment.
/// Committed at `assets/Inter-subset.ttf`.
pub(crate) const INTER_SUBSET_TTF: &[u8] = include_bytes!("../assets/Inter-subset.ttf");

/// SHA-256 of [`INTER_SUBSET_TTF`], written down rather than computed at
/// test time — the digest is this asset's identity in the store, so a silent file
/// replacement must show up as a hash mismatch instead of quietly changing
/// what golden case the digest names. `tests/asset_font.rs` asserts this
/// constant still matches the committed file. Regenerate both together with
/// `shasum -a 256 assets/Inter-subset.ttf` if the file is intentionally
/// replaced.
pub(crate) const INTER_SUBSET_SHA256: [u8; 32] = [
    0x40, 0xbb, 0xba, 0xc7, 0x15, 0x46, 0x5a, 0xdf, 0x7b, 0xa5, 0x39, 0xf5, 0x3a, 0x0c, 0xb1, 0x69,
    0x91, 0xa2, 0xc1, 0xf7, 0x3a, 0x4f, 0xb5, 0x7a, 0xf2, 0x09, 0xd3, 0x9e, 0x0c, 0x62, 0xb3, 0x27,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inter_subset_sha256_matches_committed_file() {
        // Deliberately independent of `cases::asset_font_scene_cases` /
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
