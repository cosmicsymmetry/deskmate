#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "core/protocol_message.h"
#include "core/template_fields.h"
#include "lvgl.h"
#include "ui/fonts/deskmate_fonts.h"
#include "ui/templates/weather_icon.h"

/* Raised from 14 for the complication language (spec §6.2, amended
 * 2026-08-13): the row list now tracks a module per row on top of its ten
 * row labels. The largest face (row list) uses 19 slots. */
#define TEMPLATE_OBJECT_CAPACITY 24U

/* Design system constants (spec §6.2). Every per-face template and the
 * standalone clock inherit these; do not hardcode raw colors/fonts
 * elsewhere. */
#define DESKMATE_GRID              8      /* spacing unit, spec §6.2 */
#define DESKMATE_MARGIN            (3 * DESKMATE_GRID)  /* 24px canvas margin */
#define DESKMATE_COLOR_CANVAS      lv_color_hex(0x000000)
#define DESKMATE_COLOR_PRIMARY     lv_color_hex(0xf5f5f7)
#define DESKMATE_COLOR_SECONDARY   lv_color_hex(0x9a9aa5)
#define DESKMATE_COLOR_TERTIARY    lv_color_hex(0x5c5c66)
/* Surface for card modules. Dark enough that an AMOLED still reads the
 * canvas as off, light enough to separate a module from it at desk
 * distance. */
#define DESKMATE_COLOR_SURFACE     lv_color_hex(0x1a1a1f)
#define DESKMATE_COLOR_STALE       lv_color_hex(0xf2c94c)  /* reserved, unchanged */
#define DESKMATE_COLOR_ERROR       lv_color_hex(0xff6b6b)  /* reserved, unchanged */
#define DESKMATE_FONT_CAPTION      (&deskmate_font_18)
#define DESKMATE_FONT_BODY         (&deskmate_font_28)
#define DESKMATE_FONT_DISPLAY      (&deskmate_font_56)
#define DESKMATE_FONT_HERO         (&deskmate_font_96)

/* Grid multiples used by the shared module grammar, named so a face never
 * spells a raw radius or tracking value. */
#define DESKMATE_RADIUS_MODULE     (3 * DESKMATE_GRID)  /* 24px card corner */
#define DESKMATE_RADIUS_CHIP       (2 * DESKMATE_GRID)  /* 16px pill corner */
#define DESKMATE_CHIP_HEIGHT       (4 * DESKMATE_GRID)  /* 32px pill */
#define DESKMATE_EYEBROW_TRACKING  3

/* ---------------------------------------------------------------------
 * Per-face identity palette (spec §6.2, amended 2026-08-13 by user
 * direction after design review).
 *
 * The one-accent rule is gone: each face carries a hue that identifies it
 * before the reading is parsed, which is what makes a swipeable carousel
 * legible at a glance. Three roles per face:
 *
 *   hue   the identity itself — gauges, filled chips, key secondary figures
 *   tint  the hue held back, for type set on the black canvas where the
 *         full hue would out-shout the primary reading
 *   ink   near-black drawn from the hue, for type set *on* a hue fill
 *
 * Reserved colours are unchanged and appear only in the shared state
 * footer: STALE #f2c94c and ERROR #ff6b6b. The clock hue is deliberately
 * pushed to 28° (from the spike's 32°) to open the gap to stale-gold's 45°,
 * since the clock faces are the ones whose hue is nearest it.
 * -------------------------------------------------------------------- */
typedef struct {
    lv_color_t hue;
    lv_color_t tint;
    lv_color_t ink;
} deskmate_palette_t;

/* Total over `protocol_template_kind_t`; an unknown kind returns the clock
 * palette rather than an undefined colour. */
deskmate_palette_t deskmate_palette(protocol_template_kind_t kind);

/* --- Shared module grammar (firmware/main/ui/templates/template_style.c).
 * Every face composes from these so the language stays one language. All
 * coordinates are absolute on the 448x368 canvas unless a parent is
 * passed. --- */

/* A filled surface card. */
lv_obj_t *deskmate_module(lv_obj_t *parent, int32_t x, int32_t y, int32_t w,
                          int32_t h);

/* A filled pill carrying `ink` type on `fill`. Content-sized: setting its
 * text later re-flows the pill. */
lv_obj_t *deskmate_chip(lv_obj_t *parent, int32_t x, int32_t y,
                        lv_color_t fill, lv_color_t ink);

/* Sets a chip's text, hiding it entirely when the text is empty: a
 * content-sized pill given an empty string would otherwise collapse to a
 * coloured blob. */
void deskmate_chip_set_text(lv_obj_t *chip, const char *text);

/* Small letter-spaced caption type: the label above a module's value. */
lv_obj_t *deskmate_eyebrow(lv_obj_t *parent, const char *text, int32_t x,
                           int32_t y, lv_color_t color);

/* A fixed-width, one-line, ellipsizing label, so its alignment survives a
 * text change. */
lv_obj_t *deskmate_label_box(lv_obj_t *parent, int32_t x, int32_t y,
                             int32_t w, lv_text_align_t align,
                             lv_color_t color, const lv_font_t *font);

/* True when every byte of `text` is covered by the digits-only DISPLAY/HERO
 * subsets (0-9, ':', '-', '%', and U+00B0 DEGREE SIGN; spec §5.2). Those two
 * tiers are subset fonts, so any other glyph — including the '.' of
 * LV_LABEL_LONG_DOT's ellipsis — renders as a placeholder box. Faces that
 * accept free-form text in a numeric slot call this to decide whether the
 * value may be typeset large, or must step down to DESKMATE_FONT_BODY, which
 * carries the full text range. An empty string is not "numeric": callers
 * substitute their own placeholder first. */
static inline bool deskmate_text_is_numeric(const char *text)
{
    if (text == NULL || text[0] == '\0') {
        return false;
    }
    for (const unsigned char *cursor = (const unsigned char *)text;
         *cursor != '\0'; ++cursor) {
        if ((*cursor >= (unsigned char)'0' && *cursor <= (unsigned char)'9') ||
            *cursor == (unsigned char)':' || *cursor == (unsigned char)'-' ||
            *cursor == (unsigned char)'%') {
            continue;
        }
        /* U+00B0 is the two-byte UTF-8 sequence C2 B0. */
        if (cursor[0] == 0xC2U && cursor[1] == 0xB0U) {
            ++cursor;
            continue;
        }
        return false;
    }
    return true;
}

/* Picks the largest tier that both covers `text`'s glyphs and fits `max_width`
 * on one line, walking DESKMATE_FONT_HERO -> DISPLAY -> BODY. BODY is the
 * floor for two reasons: it is the only tier with letters, and the only one
 * that can ellipsize, since its range includes the '.' that
 * LV_LABEL_LONG_DOT appends. */
static inline const lv_font_t *deskmate_number_font(const char *text,
                                                    int32_t max_width)
{
    if (!deskmate_text_is_numeric(text)) {
        return DESKMATE_FONT_BODY;
    }
    const lv_font_t *tiers[2] = { DESKMATE_FONT_HERO, DESKMATE_FONT_DISPLAY };
    for (size_t index = 0U; index < 2U; ++index) {
        lv_point_t size;
        lv_text_get_size(&size, text, tiers[index], 0, 0, LV_COORD_MAX,
                         LV_TEXT_FLAG_NONE);
        if (size.x <= max_width) {
            return tiers[index];
        }
    }
    return DESKMATE_FONT_BODY;
}

typedef struct {
    lv_obj_t *root;
    lv_obj_t *state_label;
    lv_obj_t *objects[TEMPLATE_OBJECT_CAPACITY];
    /* Owned by the two clock faces. Lives here rather than in a file-static
     * so a second view cannot inherit the previous card's setting. */
    bool clock_show_seconds;
    bool progress_running;
    int64_t progress_duration_seconds;
    int64_t progress_remaining_ms;
    int64_t progress_displayed_seconds;
    uint32_t progress_anchor_ms;
    /* Owned by the icon face. Lives here rather than in a file-static so a
     * second view cannot inherit the previous card's artwork and skip the
     * rebuild it needs. */
    weather_icon_t icon_current;
} template_widget_view_t;

bool digital_clock_create(template_widget_view_t *view,
                          lv_obj_t *parent,
                          protocol_size_class_t size);
void digital_clock_patch(template_widget_view_t *view,
                         const template_field_state_t *fields,
                         uint16_t dirty_mask);
void digital_clock_tick(template_widget_view_t *view,
                        int16_t utc_offset_minutes);

bool progress_ring_create(template_widget_view_t *view,
                          lv_obj_t *parent,
                          protocol_size_class_t size);
void progress_ring_patch(template_widget_view_t *view,
                         const template_field_state_t *fields,
                         uint16_t dirty_mask);
void progress_ring_local_action(template_widget_view_t *view,
                                protocol_event_action_t action);
void progress_ring_tick(template_widget_view_t *view);

bool row_list_create(template_widget_view_t *view,
                     lv_obj_t *parent,
                     protocol_size_class_t size);
void row_list_patch(template_widget_view_t *view,
                    const template_field_state_t *fields,
                    uint16_t dirty_mask);

bool analog_clock_create(template_widget_view_t *view,
                         lv_obj_t *parent,
                         protocol_size_class_t size);
void analog_clock_patch(template_widget_view_t *view,
                        const template_field_state_t *fields,
                        uint16_t dirty_mask);
void analog_clock_tick(template_widget_view_t *view,
                       int16_t utc_offset_minutes);

bool big_number_label_create(template_widget_view_t *view,
                             lv_obj_t *parent,
                             protocol_size_class_t size);
void big_number_label_patch(template_widget_view_t *view,
                            const template_field_state_t *fields,
                            uint16_t dirty_mask);

bool icon_badge_text_create(template_widget_view_t *view,
                            lv_obj_t *parent,
                            protocol_size_class_t size);
void icon_badge_text_patch(template_widget_view_t *view,
                           const template_field_state_t *fields,
                           uint16_t dirty_mask);
