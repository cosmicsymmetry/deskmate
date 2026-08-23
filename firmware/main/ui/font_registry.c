#include "font_registry.h"

#include <string.h>

/* One open TTF face: which asset it came from, at which pixel size, the
 * live LVGL font object, and enough bookkeeping to run LRU eviction. This
 * struct lives in caller-supplied storage (PSRAM on the device), never in
 * this file's own .bss -- see font_registry_table_bytes() in the header. */
typedef struct {
    uint8_t digest[ASSET_DIGEST_BYTES];
    int32_t pixel_size;
    lv_font_t *font;
    // The resolver's out_ptr for this face -- remembered so it can be handed
    // back to s_release() when this entry is destroyed. A face is not the
    // pointer it was built from (lv_tiny_ttf_create_data_ex gives no way to
    // recover it), so this is the only record of what to release.
    const void *asset_ptr;
    uint32_t last_used;
    uint16_t ref_count;
    bool in_use;
} font_registry_entry_t;

/* Module state: four words. The table itself is caller-supplied (see the
 * header for why); nothing here scales with FONT_REGISTRY_MAX_OPEN_FACES. */
static asset_resolver_fn s_resolver;
static asset_release_fn s_release;
static font_registry_entry_t *s_table;
static uint32_t s_generation;

size_t font_registry_table_bytes(void)
{
    return (size_t)FONT_REGISTRY_MAX_OPEN_FACES * sizeof(font_registry_entry_t);
}

font_registry_result_t font_registry_init(asset_resolver_fn resolver,
                                          asset_release_fn release,
                                          void *table_storage,
                                          size_t table_storage_size)
{
    if (resolver == NULL || table_storage == NULL) {
        return FONT_REGISTRY_ERR_ARGUMENT;
    }
    if (table_storage_size < font_registry_table_bytes()) {
        return FONT_REGISTRY_ERR_MEMORY;
    }

    s_resolver = resolver;
    s_release = release; /* may be NULL -- see the header */
    s_table = (font_registry_entry_t *)table_storage;
    memset(s_table, 0, font_registry_table_bytes());
    s_generation = 0;

    return FONT_REGISTRY_OK;
}

// Destroys one entry's LVGL face and releases the mapping it was built from,
// in that order: lv_tiny_ttf_destroy may still read the mapped bytes while
// tearing down its internal caches, so the mapping must outlive it.
static void destroy_entry(font_registry_entry_t *entry)
{
    lv_tiny_ttf_destroy(entry->font);
    if (s_release != NULL) {
        s_release(entry->asset_ptr);
    }
}

static font_registry_entry_t *find_open(const uint8_t *digest, int32_t pixel_size)
{
    for (uint32_t i = 0; i < FONT_REGISTRY_MAX_OPEN_FACES; ++i) {
        font_registry_entry_t *entry = &s_table[i];
        if (entry->in_use && entry->pixel_size == pixel_size &&
            memcmp(entry->digest, digest, ASSET_DIGEST_BYTES) == 0) {
            return entry;
        }
    }
    return NULL;
}

/* A free slot if one exists, else the least-recently-used slot with no
 * outstanding font_registry_acquire() caller, else NULL if every slot is
 * both full and pinned. */
static font_registry_entry_t *claim_slot(void)
{
    for (uint32_t i = 0; i < FONT_REGISTRY_MAX_OPEN_FACES; ++i) {
        if (!s_table[i].in_use) {
            return &s_table[i];
        }
    }

    font_registry_entry_t *victim = NULL;
    for (uint32_t i = 0; i < FONT_REGISTRY_MAX_OPEN_FACES; ++i) {
        font_registry_entry_t *entry = &s_table[i];
        if (entry->ref_count == 0 &&
            (victim == NULL || entry->last_used < victim->last_used)) {
            victim = entry;
        }
    }
    if (victim == NULL) {
        return NULL;
    }

    destroy_entry(victim);
    memset(victim, 0, sizeof(*victim));
    return victim;
}

lv_font_t *font_registry_acquire(const uint8_t *digest, int32_t pixel_size)
{
    if (s_resolver == NULL || s_table == NULL || digest == NULL || pixel_size <= 0) {
        return NULL;
    }

    font_registry_entry_t *entry = find_open(digest, pixel_size);
    if (entry != NULL) {
        entry->ref_count++;
        entry->last_used = ++s_generation;
        return entry->font;
    }

    /* Resolve and validate before claiming (and possibly evicting) a slot,
     * so an absent or non-font digest never costs a live face. */
    const void *ptr = NULL;
    uint32_t len = 0;
    uint8_t kind = 0;
    if (!s_resolver(digest, &ptr, &len, &kind)) {
        return NULL;
    }
    /* From here on the mapping is ours, so every failure exit owes the
     * resolver its release -- the same handle leak destroy_entry() already
     * avoids on eviction. The all-pinned exit below is not a corner case
     * under the header's ownership contract: a renderer holding its faces
     * for its objects' lifetimes reaches it whenever a screen wants more
     * than FONT_REGISTRY_MAX_OPEN_FACES of them. */
    if (kind != (uint8_t)ASSET_KIND_FONT && kind != (uint8_t)ASSET_KIND_ICON_FONT) {
        if (s_release != NULL) {
            s_release(ptr);
        }
        return NULL;
    }

    font_registry_entry_t *slot = claim_slot();
    if (slot == NULL) {
        if (s_release != NULL) {
            s_release(ptr);
        }
        return NULL;
    }

    lv_font_t *font = lv_tiny_ttf_create_data_ex(ptr, len, pixel_size,
                                                 LV_FONT_KERNING_NORMAL,
                                                 LV_TINY_TTF_CACHE_GLYPH_CNT);
    if (font == NULL) {
        if (s_release != NULL) {
            s_release(ptr);
        }
        return NULL;
    }

    memcpy(slot->digest, digest, ASSET_DIGEST_BYTES);
    slot->pixel_size = pixel_size;
    slot->font = font;
    slot->asset_ptr = ptr;
    slot->ref_count = 1;
    slot->last_used = ++s_generation;
    slot->in_use = true;

    return font;
}

void font_registry_release(lv_font_t *font)
{
    if (font == NULL || s_table == NULL) {
        return;
    }
    for (uint32_t i = 0; i < FONT_REGISTRY_MAX_OPEN_FACES; ++i) {
        font_registry_entry_t *entry = &s_table[i];
        if (entry->in_use && entry->font == font) {
            if (entry->ref_count > 0) {
                entry->ref_count--;
            }
            return;
        }
    }
}

uint32_t font_registry_reset(void)
{
    if (s_table == NULL) {
        return 0U;
    }
    /* Count first, destroy second. A pinned face means an LVGL object is
     * still styled with it (the header's ownership contract), and destroying
     * it would hand LVGL's draw path a dangling pointer -- a worse and far
     * quieter failure than the stale mapping this reset exists to prevent.
     * So a single pin refuses the whole reset: the table is left exactly as
     * it was, and the caller must tear those objects down and call again.
     * All-or-nothing rather than "destroy the unpinned ones anyway" because
     * a partial reset buys the caller nothing -- compaction cannot proceed
     * either way -- while leaving the registry in a state nobody asked
     * for. */
    uint32_t pinned = 0U;
    for (uint32_t i = 0; i < FONT_REGISTRY_MAX_OPEN_FACES; ++i) {
        if (s_table[i].in_use && s_table[i].ref_count > 0) {
            ++pinned;
        }
    }
    if (pinned > 0U) {
        return pinned;
    }

    for (uint32_t i = 0; i < FONT_REGISTRY_MAX_OPEN_FACES; ++i) {
        font_registry_entry_t *entry = &s_table[i];
        if (entry->in_use) {
            destroy_entry(entry);
        }
    }
    memset(s_table, 0, font_registry_table_bytes());
    s_generation = 0;
    return 0U;
}

void font_registry_warm(lv_font_t *font, const char *glyphs)
{
    if (font == NULL || glyphs == NULL) {
        return;
    }

    const unsigned char *p = (const unsigned char *)glyphs;
    while (*p != '\0') {
        uint32_t letter = (uint32_t)*p;
        uint32_t next_letter = (p[1] != '\0') ? (uint32_t)p[1] : 0U;

        lv_font_glyph_dsc_t dsc;
        memset(&dsc, 0, sizeof(dsc));
        if (lv_font_get_glyph_dsc(font, &dsc, letter, next_letter)) {
            (void)lv_font_get_glyph_bitmap(&dsc, NULL);
            lv_font_glyph_release_draw_data(&dsc);
        }

        ++p;
    }
}
