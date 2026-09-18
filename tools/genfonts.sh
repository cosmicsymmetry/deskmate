#!/usr/bin/env bash
# Regenerates firmware/main/ui/fonts/*.c. Never hand-edit the outputs.
set -euo pipefail
cd "$(dirname "$0")/.."
FONT_DIR=firmware/main/ui/fonts
# Font sources are not checked in. Callers provide OFL-licensed faces whose digit
# cmap already selects tabular forms so generated clocks and timers stay aligned.
BODY=${DESKMATE_BODY_FONT:?Set DESKMATE_BODY_FONT to a tabular-digit text TTF}
HERO=${DESKMATE_HERO_FONT:?Set DESKMATE_HERO_FONT to a tabular-digit display TTF}
# Text tiers: ASCII 0x20-0x7E, the Latin-1 Supplement block 0xA0-0xFF (spec
# §5.2, amended 2026-08-13 — accented Latin row titles like "Café" and
# "Zürich" arrive from real calendar feeds and were rendering as fallback
# boxes), plus the punctuation the built-ins emit. 0xA0-0xFF subsumes the
# former explicit 0xB0 (°) and 0xB7 (·).
TEXT_RANGE='0x20-0x7E,0xA0-0xFF,0x2014,0x2018-0x2019,0x201C-0x201D'
# Hero tiers: digits : - ° % only, tabular (spec §5.2).
HERO_RANGE='0x25,0x2D,0x30-0x3A,0xB0'
mkdir -p "$FONT_DIR"

# `lv_font_conv` does not apply OpenType GSUB features, so ordinary proportional
# faces silently lose tabular alignment. Requiring prepatched inputs keeps that
# constraint explicit now that the repository no longer owns a font-patching tool.
gen() { # size, ttf, range, name
  # --lv-include lvgl.h: the vendored LVGL component exposes lvgl.h directly
  # on the include path (firmware/managed_components/lvgl__lvgl/lvgl.h), not
  # under an "lvgl/" subdirectory, and this build never defines
  # LV_LVGL_H_INCLUDE_SIMPLE; without this flag the generated file's #else
  # branch (`#include "lvgl/lvgl.h"`) fails to compile.
  npx --yes lv_font_conv@1.5.3 --font "$2" --size "$1" --bpp 4 --format lvgl \
    --range "$3" --no-compress --lv-font-name "$4" --lv-include lvgl.h \
    -o "$FONT_DIR/$4.c"
  npx --yes lv_font_conv@1.5.3 --font "$2" --size "$1" --bpp 4 --format bin \
    --range "$3" --no-compress -o "/tmp/$4.bin"
}
gen 18 "$BODY" "$TEXT_RANGE" deskmate_font_18
gen 28 "$BODY" "$TEXT_RANGE" deskmate_font_28
gen 56 "$HERO" "$HERO_RANGE" deskmate_font_56
gen 96 "$HERO" "$HERO_RANGE" deskmate_font_96
TOTAL=$(cat /tmp/deskmate_font_{18,28,56,96}.bin | wc -c | tr -d ' ')
echo "font binary total: ${TOTAL} bytes"
if [ "$TOTAL" -gt 204800 ]; then
  echo "FAIL: font budget exceeded (limit 204800 bytes, spec §5.2)" >&2
  exit 1
fi
