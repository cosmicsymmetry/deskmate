#pragma once

#include "core/asset_store.h"
#include "lvgl.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* Resolves a digest to mapped asset bytes. Firmware passes an
 * asset_flash-backed resolver (wired in Task 9); lvgl-sim passes a RAM-backed
 * one (Task 12). This indirection is why this file carries no ESP-IDF include
 * and can therefore be compiled into the simulator like every other ui/ file. */
typedef bool (*asset_resolver_fn)(const uint8_t *digest, const void **out_ptr,
                                  uint32_t *out_len, uint8_t *out_kind);

typedef enum {
    FONT_REGISTRY_OK = 0,
    FONT_REGISTRY_ERR_ARGUMENT,
    FONT_REGISTRY_ERR_MEMORY,
} font_registry_result_t;

/* Distinct (digest, pixel_size) faces the registry keeps open at once. A miss
 * beyond this count evicts the least-recently-used face that has no
 * outstanding font_registry_acquire() callers. */
#define FONT_REGISTRY_MAX_OPEN_FACES 8U

/* Exact byte size the caller must allocate for `table_storage` below.
 *
 * The LRU table itself must live in PSRAM (it is not glyph data, but at
 * FONT_REGISTRY_MAX_OPEN_FACES entries it is larger than this module wants
 * to add to internal-RAM .bss). Allocating PSRAM is an ESP-IDF API
 * (esp_heap_caps.h's heap_caps_malloc(..., MALLOC_CAP_SPIRAM)), and this
 * file must build with no ESP-IDF present -- Task 12 compiles it into
 * lvgl-sim, which builds firmware C on the host. So the table is
 * caller-supplied, exactly like the resolver: firmware's caller allocates
 * font_registry_table_bytes() bytes from PSRAM and hands the pointer in;
 * lvgl-sim's caller can use a plain host allocation. This keeps this file's
 * own static footprint at a small, fixed number of pointers/counters,
 * following the precedent in link/ota.c (commit 5699f1d) of packing state
 * rather than growing .bss. */
size_t font_registry_table_bytes(void);

font_registry_result_t font_registry_init(asset_resolver_fn resolver,
                                          void *table_storage,
                                          size_t table_storage_size);

/* Returns a font for `digest` rendered at `pixel_size`, or NULL if the asset
 * is absent, not a font (ASSET_KIND_FONT / ASSET_KIND_ICON_FONT only), fails
 * to rasterize, or every open face slot is both full and pinned by an
 * outstanding caller. Call under lvgl_port_lock(). Each successful call must
 * be paired with font_registry_release(). */
lv_font_t *font_registry_acquire(const uint8_t *digest, int32_t pixel_size);

void font_registry_release(lv_font_t *font);

/* Destroys every open face and clears the table. Call before compaction
 * invalidates mmap pointers -- compaction moves blobs, so a font surviving
 * it would read moved bytes. */
void font_registry_reset(void);

/* Rasterizes every codepoint in `glyphs` into the face's own glyph/bitmap
 * cache ahead of first paint, so a cold 96px digit does not rasterize
 * inline on the LVGL task mid-render. `glyphs` is read as single-byte
 * (ASCII) codepoints, matching its documented call sites (e.g.
 * "0123456789:"); it is not a general UTF-8 decoder. */
void font_registry_warm(lv_font_t *font, const char *glyphs);
