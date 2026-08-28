#pragma once

/* Minimal stand-in for <lvgl.h>, used ONLY by firmware/host_tests.
 *
 * `ui/font_registry.c` is the one ui/ file whose *policy* -- reference
 * counting, LRU eviction, reset -- is worth host-testing, but it includes
 * "lvgl.h" for four symbols it never inspects the innards of: it stores an
 * `lv_font_t *` opaquely, creates one with lv_tiny_ttf_create_data_ex(),
 * destroys it with lv_tiny_ttf_destroy(), and walks the three glyph calls in
 * font_registry_warm(). Building real LVGL for a host test to exercise that
 * policy would be several orders of magnitude more machinery than the policy
 * itself, so the test provides those four entry points instead.
 *
 * The declarations below are copied from LVGL 9's real headers
 * (managed_components/lvgl__lvgl/src/libs/tiny_ttf/lv_tiny_ttf.h and
 * src/font/lv_font.h) so a signature drift in font_registry.c's calls fails
 * the host build too, not just the firmware one. The struct *bodies* are
 * deliberately not LVGL's: nothing under test reads a field.
 *
 * This header is reachable only through firmware/host_tests/Makefile's
 * -Istubs; the ESP-IDF and lvgl-sim builds never see it. */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define LV_TINY_TTF_CACHE_GLYPH_CNT 128

typedef enum {
    LV_FONT_KERNING_NORMAL = 0,
    LV_FONT_KERNING_NONE = 1,
} lv_font_kerning_t;

typedef struct {
    int32_t line_height;
    int32_t base_line;
} lv_font_t;

typedef struct {
    uint32_t gid;
    uint32_t adv_w;
} lv_font_glyph_dsc_t;

typedef struct lv_draw_buf_t lv_draw_buf_t;

lv_font_t *lv_tiny_ttf_create_data_ex(const void *data, size_t data_size,
                                      int32_t font_size,
                                      lv_font_kerning_t kerning,
                                      size_t cache_size);
void lv_tiny_ttf_destroy(lv_font_t *font);

bool lv_font_get_glyph_dsc(const lv_font_t *font, lv_font_glyph_dsc_t *dsc_out,
                           uint32_t letter, uint32_t letter_next);
const void *lv_font_get_glyph_bitmap(lv_font_glyph_dsc_t *g_dsc,
                                     lv_draw_buf_t *draw_buf);
void lv_font_glyph_release_draw_data(lv_font_glyph_dsc_t *g_dsc);
