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

/* Releases a mapping previously handed back through `out_ptr` by
 * asset_resolver_fn, once this registry is done reading it. Injected for the
 * same reason the resolver is: firmware's caller wires this to
 * asset_flash_unmap() (link/asset_flash.c, ESP-IDF-only), lvgl-sim's caller
 * wires a no-op (its resolver hands back a direct pointer into simulated
 * flash, nothing to unmap). May be NULL, meaning "nothing to release" --
 * every call site below already guards on that. */
typedef void (*asset_release_fn)(const void *ptr);

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
                                          asset_release_fn release,
                                          void *table_storage,
                                          size_t table_storage_size);

/* OWNERSHIP CONTRACT -- read before styling an LVGL object with an acquired
 * face.
 *
 * An acquired face stays alive until it is released: the registry never
 * evicts a face with an outstanding acquire, and font_registry_reset()
 * refuses outright while one exists. Nothing else keeps a face alive.
 * In particular **LVGL takes no reference**: lv_obj_set_style_text_font()
 * stores the bare lv_font_t* and neither copies nor counts it, so an object
 * styled with a face is still pointing at it long after the code that
 * acquired it has returned.
 *
 * Therefore: a caller that hands a face to an LVGL object owns that acquire
 * for the object's whole lifetime, and releases only after the object is
 * destroyed -- not when the styling call returns. Releasing early leaves the
 * face unpinned, and the very next acquire of a different (digest,
 * pixel_size) can pick it as the LRU victim and destroy it out from under a
 * screen that is still drawing with it. With FONT_REGISTRY_MAX_OPEN_FACES
 * slots that is ordinary behaviour, not an exotic one.
 *
 * The cost of the contract is a hard ceiling: a screen wanting more than
 * FONT_REGISTRY_MAX_OPEN_FACES distinct faces at once gets NULL for the
 * surplus rather than thrashing. Handle NULL -- fall back to a baked font or
 * refuse the scene; never draw with it. */

/* Returns a font for `digest` rendered at `pixel_size`, or NULL if the asset
 * is absent, not a font (ASSET_KIND_FONT / ASSET_KIND_ICON_FONT only), fails
 * to rasterize, or every open face slot is both full and pinned by an
 * outstanding caller. Call under lvgl_port_lock(). Each successful call must
 * be paired with font_registry_release() -- see the contract above for
 * when. */
lv_font_t *font_registry_acquire(const uint8_t *digest, int32_t pixel_size);

/* Drops one acquire. The face is not destroyed here; it merely becomes
 * eligible for eviction and for font_registry_reset(). Ignores a NULL or
 * unknown pointer. */
void font_registry_release(lv_font_t *font);

/* Destroys every open face and clears the table -- but only if no face has an
 * outstanding acquire. Call before compaction invalidates mmap pointers:
 * compaction moves blobs, so a font surviving it would read moved bytes.
 *
 * Returns the number of still-acquired faces, and a nonzero return means
 * **nothing was destroyed and the registry is unchanged**. It is not a
 * warning to log past -- it is a refusal, and compaction must not proceed.
 * Destroying a pinned face would trade a stale mapping for a dangling
 * lv_font_t* that LVGL's draw path dereferences, which is strictly worse and
 * invisible until it crashes.
 *
 * The caller's recovery is to drop those references and call again: destroy
 * the LVGL objects holding them (which releases their acquires), reset,
 * compact, then rebuild. A caller with no such teardown to run should
 * refuse the whole operation and let the host retry -- a deferred garbage
 * collection is recoverable, a dangling font pointer is not. */
uint32_t font_registry_reset(void);

/* Rasterizes every codepoint in `glyphs` into the face's own glyph/bitmap
 * cache ahead of first paint, so a cold 96px digit does not rasterize
 * inline on the LVGL task mid-render. `glyphs` is read as single-byte
 * (ASCII) codepoints, matching its documented call sites (e.g.
 * "0123456789:"); it is not a general UTF-8 decoder. */
void font_registry_warm(lv_font_t *font, const char *glyphs);
