#!/usr/bin/env python3
"""Bake the OpenType 'tnum' (tabular figures) feature into a font's cmap.

Why this exists: lv_font_conv extracts glyph outlines by a direct codepoint
lookup in the font's `cmap` table. It does not perform OpenType text shaping
and never applies GSUB substitution features (kern, tnum, etc). Inter's
digits are *proportional* by default; the tabular ("tnum") variants
(zero.tf, one.tf, ... nine.tf) only become reachable once a text shaper
applies the `tnum` feature. Since lv_font_conv has no such capability, the
baked bitmaps would silently keep proportional widths unless the digit
codepoints are repointed at the tabular glyphs before conversion.

This script rewrites the `cmap` table of a copy of the input font so that
U+0030-U+0039 (0-9) resolve directly to the glyphs the font's own `tnum`
GSUB feature would have selected, then writes that copy out. It is a
scripted, reproducible transform (never hand-edited). Callers can supply its
output to tools/genfonts.sh, which requires tabular-cmap fonts but does not
invoke this script.
"""
import sys

from fontTools.ttLib import TTFont

DIGITS = [chr(c) for c in range(0x30, 0x3A)]


def find_tnum_mapping(font):
    gsub = font["GSUB"].table
    feature_records = gsub.FeatureList.FeatureRecord
    lookup_list = gsub.LookupList.Lookup

    tnum_indices = [
        i for i, fr in enumerate(feature_records) if fr.FeatureTag == "tnum"
    ]
    if not tnum_indices:
        sys.exit("patch_tabular_figures: no 'tnum' feature in source font")

    mapping = {}
    for fi in tnum_indices:
        feature = feature_records[fi].Feature
        for li in feature.LookupListIndex:
            lookup = lookup_list[li]
            if lookup.LookupType != 1:  # single substitution
                continue
            for subtable in lookup.SubTable:
                if hasattr(subtable, "mapping"):
                    mapping.update(subtable.mapping)
    return mapping


def main(src_path, dst_path):
    font = TTFont(src_path)
    tnum_mapping = find_tnum_mapping(font)

    # Several cmap subtables (e.g. the Windows-BMP and Windows-symbol
    # encodings) are decoded by fontTools as views over the *same*
    # underlying dict object, so mutating one mutates the other before it is
    # ever visited. Dedupe by dict identity first, or the second visit sees
    # an already-remapped '<name>.tf' glyph and fails the mapping lookup.
    seen_dict_ids = set()
    unique_cmaps = []
    for table in font["cmap"].tables:
        if id(table.cmap) not in seen_dict_ids:
            seen_dict_ids.add(id(table.cmap))
            unique_cmaps.append(table.cmap)

    remapped = 0
    for cmap_dict in unique_cmaps:
        for ch in DIGITS:
            codepoint = ord(ch)
            default_glyph = cmap_dict.get(codepoint)
            if default_glyph is None:
                continue
            tabular_glyph = tnum_mapping.get(default_glyph)
            if tabular_glyph is None:
                sys.exit(
                    f"patch_tabular_figures: no tnum substitute for "
                    f"{default_glyph!r} (U+{codepoint:04X})"
                )
            cmap_dict[codepoint] = tabular_glyph
            remapped += 1

    if remapped == 0:
        sys.exit("patch_tabular_figures: no digit codepoints were remapped")

    font.save(dst_path)
    print(
        f"patch_tabular_figures: remapped {remapped} digit cmap entries "
        f"-> {dst_path}"
    )


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(f"usage: {sys.argv[0]} <src.ttf> <dst.ttf>")
    main(sys.argv[1], sys.argv[2])
