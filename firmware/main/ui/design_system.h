#pragma once

#include <stdint.h>

#include "lvgl.h"
#include "ui/fonts/deskmate_fonts.h"

/* Shared by the standalone clock and OTA takeover. The retired card-template
 * oracle carries a frozen sibling in
 * companion/crates/lvgl-sim/reference-oracle/ui/templates/template_internal.h;
 * palette or spacing edits here do not update parity coverage for the
 * standalone clock, so compare that sibling deliberately when changing them. */
#define DESKMATE_GRID              8
#define DESKMATE_MARGIN            (3 * DESKMATE_GRID)
#define DESKMATE_COLOR_CANVAS      lv_color_hex(0x000000)
#define DESKMATE_COLOR_PRIMARY     lv_color_hex(0xf5f5f7)
#define DESKMATE_COLOR_SECONDARY   lv_color_hex(0x9a9aa5)
#define DESKMATE_COLOR_TERTIARY    lv_color_hex(0x5c5c66)
#define DESKMATE_COLOR_SURFACE     lv_color_hex(0x1a1a1f)
#define DESKMATE_FONT_CAPTION      (&deskmate_font_18)
#define DESKMATE_FONT_BODY         (&deskmate_font_28)
#define DESKMATE_FONT_DISPLAY      (&deskmate_font_56)
#define DESKMATE_FONT_HERO         (&deskmate_font_96)
#define DESKMATE_RADIUS_MODULE     (3 * DESKMATE_GRID)
#define DESKMATE_EYEBROW_TRACKING  3

lv_obj_t *deskmate_module(lv_obj_t *parent, int32_t x, int32_t y, int32_t w,
                          int32_t h);
lv_obj_t *deskmate_eyebrow(lv_obj_t *parent, const char *text, int32_t x,
                           int32_t y, lv_color_t color);
lv_obj_t *deskmate_label_box(lv_obj_t *parent, int32_t x, int32_t y,
                             int32_t w, lv_text_align_t align,
                             lv_color_t color, const lv_font_t *font);
