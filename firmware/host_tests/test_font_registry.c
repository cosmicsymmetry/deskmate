#include <assert.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "ui/font_registry.h"

/* ---------------------------------------------------------------------------
 * The fake LVGL declared by host_tests/stubs/lvgl.h.
 *
 * Every face is a distinct address in s_faces and an address is never reused
 * within a test, so "the registry handed back the same face" and "the
 * registry destroyed that face" are both directly observable, and a double
 * destroy trips an assert instead of passing.
 * ------------------------------------------------------------------------ */

#define FAKE_FACE_CAPACITY 32

static lv_font_t s_faces[FAKE_FACE_CAPACITY];
static bool s_face_live[FAKE_FACE_CAPACITY];
static size_t s_next_face; /* never reused, so an address identifies a face */
static uint32_t s_create_count;
static uint32_t s_destroy_count;
static bool s_create_fails;

lv_font_t *lv_tiny_ttf_create_data_ex(const void *data, size_t data_size,
                                      int32_t font_size,
                                      lv_font_kerning_t kerning,
                                      size_t cache_size)
{
    (void)data;
    (void)data_size;
    (void)font_size;
    (void)kerning;
    (void)cache_size;
    if (s_create_fails) {
        return NULL;
    }
    assert(s_next_face < FAKE_FACE_CAPACITY);
    size_t index = s_next_face++;
    s_face_live[index] = true;
    ++s_create_count;
    return &s_faces[index];
}

void lv_tiny_ttf_destroy(lv_font_t *font)
{
    assert(font >= &s_faces[0] && font < &s_faces[FAKE_FACE_CAPACITY]);
    size_t index = (size_t)(font - &s_faces[0]);
    assert(s_face_live[index]); /* never destroy a face twice */
    s_face_live[index] = false;
    ++s_destroy_count;
}

bool lv_font_get_glyph_dsc(const lv_font_t *font, lv_font_glyph_dsc_t *dsc_out,
                           uint32_t letter, uint32_t letter_next)
{
    (void)font;
    (void)dsc_out;
    (void)letter;
    (void)letter_next;
    return false;
}

const void *lv_font_get_glyph_bitmap(lv_font_glyph_dsc_t *g_dsc,
                                     lv_draw_buf_t *draw_buf)
{
    (void)g_dsc;
    (void)draw_buf;
    return NULL;
}

void lv_font_glyph_release_draw_data(lv_font_glyph_dsc_t *g_dsc)
{
    (void)g_dsc;
}

/* ---------------------------------------------------------------------------
 * The injected asset resolver/release pair -- the same seam firmware wires to
 * asset_flash_map()/asset_flash_unmap() and lvgl-sim wires to its RAM store.
 * Asset N is a distinct byte array, so `s_last_released` identifies exactly
 * which mapping was handed back.
 * ------------------------------------------------------------------------ */

#define FAKE_ASSET_COUNT 16U
/* The one asset that is not a font, used to exercise the kind rejection. */
#define FAKE_IMAGE_ASSET_INDEX (FAKE_ASSET_COUNT - 1U)

static uint8_t s_asset_bytes[FAKE_ASSET_COUNT][8];
static uint32_t s_map_count;
static uint32_t s_unmap_count;
static const void *s_last_unmapped;

static bool fake_resolver(const uint8_t *digest, const void **out_ptr,
                          uint32_t *out_len, uint8_t *out_kind)
{
    unsigned index = digest[0];
    if (index >= FAKE_ASSET_COUNT) {
        return false;
    }
    *out_ptr = s_asset_bytes[index];
    *out_len = (uint32_t)sizeof s_asset_bytes[index];
    *out_kind = (index == FAKE_IMAGE_ASSET_INDEX) ? (uint8_t)ASSET_KIND_IMAGE
                                                  : (uint8_t)ASSET_KIND_FONT;
    ++s_map_count;
    return true;
}

static void fake_release(const void *ptr)
{
    ++s_unmap_count;
    s_last_unmapped = ptr;
}

/* Storage for the registry table, standing in for the PSRAM block firmware
 * hands it. uint64_t elements so the buffer is aligned for the pointers the
 * entries hold. */
static uint64_t s_table_storage[128];

static void digest_for(uint8_t out[ASSET_DIGEST_BYTES], unsigned index)
{
    memset(out, 0, ASSET_DIGEST_BYTES);
    out[0] = (uint8_t)index;
}

static lv_font_t *acquire(unsigned asset_index, int32_t pixel_size)
{
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_for(digest, asset_index);
    return font_registry_acquire(digest, pixel_size);
}

static void begin(void)
{
    memset(s_face_live, 0, sizeof s_face_live);
    s_next_face = 0U;
    s_create_count = 0U;
    s_destroy_count = 0U;
    s_create_fails = false;
    s_map_count = 0U;
    s_unmap_count = 0U;
    s_last_unmapped = NULL;
    assert(font_registry_table_bytes() <= sizeof s_table_storage);
    assert(font_registry_init(fake_resolver, fake_release, s_table_storage,
                              sizeof s_table_storage) == FONT_REGISTRY_OK);
}

/* Fills every slot with a distinct pinned face. */
static void fill_all_slots(lv_font_t *out_faces[FONT_REGISTRY_MAX_OPEN_FACES])
{
    for (unsigned i = 0U; i < FONT_REGISTRY_MAX_OPEN_FACES; ++i) {
        out_faces[i] = acquire(i, 24);
        assert(out_faces[i] != NULL);
    }
    assert(s_create_count == FONT_REGISTRY_MAX_OPEN_FACES);
}

/* ------------------------------------------------------------------------ */

static void test_the_same_face_is_shared_and_counted(void)
{
    begin();
    lv_font_t *first = acquire(0U, 24);
    lv_font_t *second = acquire(0U, 24);
    assert(first != NULL && first == second);
    /* One open face, one mapping -- not two of each. */
    assert(s_create_count == 1U);
    assert(s_map_count == 1U);

    /* One release does not unpin a face two callers still hold. */
    font_registry_release(first);
    assert(font_registry_reset() == 1U);
    assert(s_destroy_count == 0U);

    font_registry_release(second);
    assert(font_registry_reset() == 0U);
    assert(s_destroy_count == 1U);
}

static void test_a_differing_pixel_size_is_a_separate_face(void)
{
    begin();
    lv_font_t *small = acquire(0U, 24);
    lv_font_t *large = acquire(0U, 96);
    assert(small != NULL && large != NULL && small != large);
    assert(s_create_count == 2U);
}

static void test_a_pinned_face_is_never_evicted(void)
{
    /* This is the hazard the scene renderer walks into: eight distinct faces
     * is an ordinary scene, not an exotic one. Every held face must survive a
     * ninth acquire, and the ninth must fail rather than destroy one of them
     * out from under the LVGL objects styled with it. */
    begin();
    lv_font_t *faces[FONT_REGISTRY_MAX_OPEN_FACES];
    fill_all_slots(faces);

    assert(acquire(FONT_REGISTRY_MAX_OPEN_FACES, 24) == NULL);
    assert(s_destroy_count == 0U);

    for (unsigned i = 0U; i < FONT_REGISTRY_MAX_OPEN_FACES; ++i) {
        assert(acquire(i, 24) == faces[i]);
    }
}

static void test_the_least_recently_used_unpinned_face_is_the_victim(void)
{
    begin();
    lv_font_t *faces[FONT_REGISTRY_MAX_OPEN_FACES];
    fill_all_slots(faces);

    /* Release slot 0 and slot 3; slot 0 is the older of the two, so it is the
     * one a ninth acquire must take. */
    font_registry_release(faces[0]);
    font_registry_release(faces[3]);

    lv_font_t *ninth = acquire(FONT_REGISTRY_MAX_OPEN_FACES, 24);
    assert(ninth != NULL);
    assert(s_destroy_count == 1U);
    assert(!s_face_live[(size_t)(faces[0] - &s_faces[0])]);
    /* Evicting a face hands its mapping back -- the leak stage 1 fixed. */
    assert(s_unmap_count == 1U);
    assert(s_last_unmapped == s_asset_bytes[0]);

    /* The other released face is still open, and slot 3 is still its face. */
    assert(acquire(3U, 24) == faces[3]);
}

static void test_a_failed_acquire_hands_its_mapping_back(void)
{
    /* Every early return after a successful resolve owes the resolver an
     * unmap. The all-pinned path in particular stops being theoretical the
     * moment a real renderer holds its faces for their objects' lifetimes. */
    begin();
    lv_font_t *faces[FONT_REGISTRY_MAX_OPEN_FACES];
    fill_all_slots(faces);
    assert(s_unmap_count == 0U);

    /* Every slot pinned. */
    assert(acquire(FONT_REGISTRY_MAX_OPEN_FACES, 24) == NULL);
    assert(s_unmap_count == 1U);

    /* Resolves, but is not a font. */
    assert(acquire(FAKE_IMAGE_ASSET_INDEX, 24) == NULL);
    assert(s_unmap_count == 2U);
    assert(s_last_unmapped == s_asset_bytes[FAKE_IMAGE_ASSET_INDEX]);

    /* Resolves and is a font, but rasterization fails. Free a slot first so
     * this reaches lv_tiny_ttf_create_data_ex() rather than stopping at the
     * pinned-slots check above. */
    font_registry_release(faces[0]);
    s_create_fails = true;
    assert(acquire(FONT_REGISTRY_MAX_OPEN_FACES, 24) == NULL);
    assert(s_unmap_count == 4U); /* the evicted face's, and this one's */
}

static void test_reset_destroys_nothing_while_a_face_is_pinned(void)
{
    /* font_registry_reset() runs immediately before asset compaction moves
     * the mapped bytes. Destroying a face an LVGL object still points at
     * would trade a stale mapping for a dangling pointer, which is worse and
     * invisible. So a pinned registry refuses the reset whole, and the count
     * it returns is the caller's cue to tear those objects down first. */
    begin();
    lv_font_t *held = acquire(0U, 24);
    lv_font_t *other = acquire(1U, 24);
    assert(held != NULL && other != NULL);
    font_registry_release(other);

    assert(font_registry_reset() == 1U);
    /* Nothing was destroyed -- not the pinned face, and not the unpinned one
     * either: the refusal leaves the registry exactly as it was. */
    assert(s_destroy_count == 0U);
    assert(s_unmap_count == 0U);
    assert(acquire(0U, 24) == held);
    assert(acquire(1U, 24) == other);
}

static void test_reset_clears_the_table_once_every_face_is_released(void)
{
    begin();
    lv_font_t *faces[FONT_REGISTRY_MAX_OPEN_FACES];
    fill_all_slots(faces);
    for (unsigned i = 0U; i < FONT_REGISTRY_MAX_OPEN_FACES; ++i) {
        font_registry_release(faces[i]);
    }

    assert(font_registry_reset() == 0U);
    assert(s_destroy_count == FONT_REGISTRY_MAX_OPEN_FACES);
    assert(s_unmap_count == FONT_REGISTRY_MAX_OPEN_FACES);

    /* The table really is empty: the next acquire rasterizes afresh. */
    uint32_t creates_before = s_create_count;
    lv_font_t *fresh = acquire(0U, 24);
    assert(fresh != NULL);
    assert(s_create_count == creates_before + 1U);
    font_registry_release(fresh);

    /* And an already-empty registry resets to 0 without touching anything. */
    assert(font_registry_reset() == 0U);
    assert(font_registry_reset() == 0U);
}

static void test_releasing_a_face_the_registry_never_handed_out(void)
{
    begin();
    lv_font_t *face = acquire(0U, 24);
    assert(face != NULL);

    /* A stray pointer must not decrement somebody else's count. */
    lv_font_t stranger;
    font_registry_release(&stranger);
    font_registry_release(NULL);
    assert(font_registry_reset() == 1U);
}

static void test_an_unresolvable_digest_costs_no_face(void)
{
    begin();
    lv_font_t *face = acquire(0U, 24);
    assert(face != NULL);

    assert(acquire(FAKE_ASSET_COUNT + 1U, 24) == NULL);
    assert(s_map_count == 1U);
    assert(s_unmap_count == 0U);
    assert(s_destroy_count == 0U);
    assert(acquire(0U, 24) == face);
}

static void test_argument_rejection(void)
{
    begin();
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_for(digest, 0U);
    assert(font_registry_acquire(NULL, 24) == NULL);
    assert(font_registry_acquire(digest, 0) == NULL);
    assert(font_registry_acquire(digest, -1) == NULL);
    assert(s_map_count == 0U);

    assert(font_registry_init(NULL, fake_release, s_table_storage,
                              sizeof s_table_storage) ==
           FONT_REGISTRY_ERR_ARGUMENT);
    assert(font_registry_init(fake_resolver, fake_release, NULL, 0U) ==
           FONT_REGISTRY_ERR_ARGUMENT);
    assert(font_registry_init(fake_resolver, fake_release, s_table_storage,
                              font_registry_table_bytes() - 1U) ==
           FONT_REGISTRY_ERR_MEMORY);
}

int main(void)
{
    test_the_same_face_is_shared_and_counted();
    test_a_differing_pixel_size_is_a_separate_face();
    test_a_pinned_face_is_never_evicted();
    test_the_least_recently_used_unpinned_face_is_the_victim();
    test_a_failed_acquire_hands_its_mapping_back();
    test_reset_destroys_nothing_while_a_face_is_pinned();
    test_reset_clears_the_table_once_every_face_is_released();
    test_releasing_a_face_the_registry_never_handed_out();
    test_an_unresolvable_digest_costs_no_face();
    test_argument_rejection();
    return 0;
}
