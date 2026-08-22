#include "link/dev_capture.h"

// Dev-only: this translation unit is empty unless DESKMATE_DEV_DIAG is
// defined (see dev_capture.h). It is always listed in
// firmware/main/CMakeLists.txt's SRCS -- the #ifdef, not conditional
// compilation of the file itself, is what keeps a release build free of the
// handler, the message ids, and this file's log tag (verified with
// `strings` on the release .elf; see the Task 10 report).
#ifdef DESKMATE_DEV_DIAG

#include <stdbool.h>
#include <string.h>

#include "board/board.h"
#include "core/protocol_frame.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_lvgl_port.h"
#include "freertos/FreeRTOS.h"
#include "link/usb_link.h"
#include "lvgl.h"
#include "ui/font_registry.h"

// Captures the *pre-flush* logical frame: lv_snapshot_take_to_draw_buf()
// re-renders the object tree through LVGL's normal software draw path into
// an offscreen buffer of our own, entirely separate from the CO5300 flush
// pipeline. That means this capture reproduces layout, fonts, and colors
// exactly like the panel, but NOT board_lcd_rounder_cb's even-pixel
// rounding or the 90/270 software-rotation transform applied only at flush
// time (lv_display_set_rotation, board_display_set_rotation_180) -- both
// stay the existing physical checks' responsibility (spec §3.1's claim
// boundary). The screen object's logical size is 448x368 regardless of
// which of those two rotations is active, so the captured buffer is the
// same either way; the host-side diff tool is responsible for applying the
// same 180 degree pixel reversal lvgl-sim's shim uses for the flipped mount
// before comparing against this capture.

static const char *TAG = "dev_capture";

#define DEV_CAPTURE_FRAME_WIDTH BOARD_LCD_V_RES
#define DEV_CAPTURE_FRAME_HEIGHT BOARD_LCD_H_RES
#define DEV_CAPTURE_BYTES_PER_PIXEL 2U
#define DEV_CAPTURE_FRAME_BYTES                                          \
    ((size_t)DEV_CAPTURE_FRAME_WIDTH * (size_t)DEV_CAPTURE_FRAME_HEIGHT * \
     DEV_CAPTURE_BYTES_PER_PIXEL)
#define DEV_CAPTURE_CHUNK_HEADER_SIZE 12U
#define DEV_CAPTURE_CHUNK_DATA_SIZE \
    (PROTOCOL_MAX_PAYLOAD_SIZE - DEV_CAPTURE_CHUNK_HEADER_SIZE)
#define DEV_CAPTURE_WRITE_TIMEOUT_MS 200U

// Kept off the protocol task's 8 KiB stack, same rationale as
// protocol_task.c's context->message reuse: a protocol_frame_t's payload
// array alone is ~2 KiB.
static protocol_frame_t s_frame;
static uint8_t s_wire[PROTOCOL_MAX_WIRE_FRAME];

static void write_u32_le(uint8_t *bytes, uint32_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8U);
    bytes[2] = (uint8_t)(value >> 16U);
    bytes[3] = (uint8_t)(value >> 24U);
}

static bool send_chunk(uint32_t request_id, uint32_t offset, uint32_t total,
                       const uint8_t *data, size_t length)
{
    write_u32_le(s_frame.payload + 0U, offset);
    write_u32_le(s_frame.payload + 4U, total);
    write_u32_le(s_frame.payload + 8U, protocol_crc32c(data, length));
    memcpy(s_frame.payload + DEV_CAPTURE_CHUNK_HEADER_SIZE, data, length);

    s_frame.version = PROTOCOL_VERSION;
    s_frame.message_type = DEV_CAPTURE_CHUNK_TYPE;
    s_frame.flags = 0U;
    s_frame.request_id = request_id;
    s_frame.payload_length =
        (uint16_t)(DEV_CAPTURE_CHUNK_HEADER_SIZE + length);

    size_t wire_length = 0U;
    if (protocol_frame_encode(&s_frame, s_wire, sizeof(s_wire),
                              &wire_length) != PROTOCOL_FRAME_OK) {
        ESP_LOGW(TAG, "chunk encode failed at offset %u", (unsigned)offset);
        return false;
    }
    if (usb_link_write_frame(s_wire, wire_length,
                             pdMS_TO_TICKS(DEV_CAPTURE_WRITE_TIMEOUT_MS)) !=
        ESP_OK) {
        ESP_LOGW(TAG, "chunk write failed at offset %u", (unsigned)offset);
        return false;
    }
    return true;
}

void dev_capture_handle_request(uint32_t request_id)
{
    uint8_t *buffer = heap_caps_malloc(DEV_CAPTURE_FRAME_BYTES,
                                       MALLOC_CAP_SPIRAM);
    if (buffer == NULL) {
        ESP_LOGW(TAG, "PSRAM allocation failed (%u bytes)",
                (unsigned)DEV_CAPTURE_FRAME_BYTES);
        return;
    }

    lv_draw_buf_t draw_buf;
    bool ok = lv_draw_buf_init(&draw_buf, 1, 1, LV_COLOR_FORMAT_RGB565,
                               LV_STRIDE_AUTO, buffer,
                               (uint32_t)DEV_CAPTURE_FRAME_BYTES) ==
              LV_RESULT_OK;

    if (ok) {
        lvgl_port_lock(0);
        ok = lv_snapshot_take_to_draw_buf(lv_screen_active(),
                                          LV_COLOR_FORMAT_RGB565,
                                          &draw_buf) == LV_RESULT_OK;
        lvgl_port_unlock();
    }

    if (!ok) {
        ESP_LOGW(TAG, "snapshot failed");
        heap_caps_free(buffer);
        return;
    }

    size_t sent = 0U;
    while (sent < DEV_CAPTURE_FRAME_BYTES) {
        size_t chunk_length = DEV_CAPTURE_FRAME_BYTES - sent;
        if (chunk_length > DEV_CAPTURE_CHUNK_DATA_SIZE) {
            chunk_length = DEV_CAPTURE_CHUNK_DATA_SIZE;
        }
        if (!send_chunk(request_id, (uint32_t)sent,
                       (uint32_t)DEV_CAPTURE_FRAME_BYTES, buffer + sent,
                       chunk_length)) {
            break;
        }
        sent += chunk_length;
    }

    heap_caps_free(buffer);
}

// Pinned to match lvgl-sim's asset-font golden exactly (Task 12,
// companion/crates/lvgl-sim/src/cases.rs's asset_font_cases() /
// csrc/sim_shim.c's sim_render_asset_font()): 72px, "12:34", warmed glyph
// set "0123456789:", true-black canvas (0x000000), text colour 0xf5f5f7,
// centred. A mismatch in any of these makes the device/simulator
// comparison meaningless, not just wrong.
#define DEV_ASSET_PROBE_PIXEL_SIZE 72
#define DEV_ASSET_PROBE_TEXT "12:34"
#define DEV_ASSET_PROBE_WARM_GLYPHS "0123456789:"
#define DEV_ASSET_PROBE_BG_COLOR 0x000000U
#define DEV_ASSET_PROBE_TEXT_COLOR 0xf5f5f7U

void dev_capture_handle_asset_probe(const uint8_t *digest)
{
    if (digest == NULL) {
        ESP_LOGW(TAG, "asset probe: null digest");
        return;
    }

    lvgl_port_lock(0);

    lv_font_t *font =
        font_registry_acquire(digest, DEV_ASSET_PROBE_PIXEL_SIZE);
    if (font == NULL) {
        lvgl_port_unlock();
        ESP_LOGW(TAG, "asset probe: font_registry_acquire failed");
        return;
    }
    font_registry_warm(font, DEV_ASSET_PROBE_WARM_GLYPHS);

    // Bare screen + centred label, deliberately not template_view.c's card
    // chrome -- this probe pins raw runtime-font rendering parity against
    // sim_render_asset_font(), not any one template's layout.
    lv_obj_t *screen = lv_obj_create(NULL);
    if (screen == NULL) {
        font_registry_release(font);
        lvgl_port_unlock();
        ESP_LOGW(TAG, "asset probe: screen allocation failed");
        return;
    }
    lv_obj_remove_flag(screen, LV_OBJ_FLAG_SCROLLABLE);
    lv_obj_set_style_bg_color(screen, lv_color_hex(DEV_ASSET_PROBE_BG_COLOR),
                              0);
    lv_obj_set_style_bg_opa(screen, LV_OPA_COVER, 0);

    lv_obj_t *label = lv_label_create(screen);
    if (label == NULL) {
        lv_obj_delete(screen);
        font_registry_release(font);
        lvgl_port_unlock();
        ESP_LOGW(TAG, "asset probe: label allocation failed");
        return;
    }
    lv_obj_remove_style_all(label);
    lv_obj_set_style_text_font(label, font, 0);
    lv_obj_set_style_text_color(
        label, lv_color_hex(DEV_ASSET_PROBE_TEXT_COLOR), 0);
    lv_label_set_text(label, DEV_ASSET_PROBE_TEXT);
    lv_obj_center(label);

    // auto_del=true deletes whatever screen was previously active (the
    // standalone clock, a template render, or a prior probe render), the
    // same call template_view_show() uses.
    lv_screen_load_anim(screen, LV_SCREEN_LOAD_ANIM_NONE, 0U, 0U, true);

    // The active screen now holds the font (LVGL keeps its own reference
    // via the label's style), so this probe's own hold is released once
    // the screen is live -- matching sim_render_asset_font(), which
    // releases immediately after copying the flushed frame out.
    font_registry_release(font);

    lvgl_port_unlock();
    ESP_LOGI(TAG, "asset probe rendered at %dpx", DEV_ASSET_PROBE_PIXEL_SIZE);
}

#endif // DESKMATE_DEV_DIAG
