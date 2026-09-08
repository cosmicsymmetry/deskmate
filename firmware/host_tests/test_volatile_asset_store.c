#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "core/volatile_asset_store.h"

typedef struct {
    size_t allocations;
    size_t frees;
    size_t live;
    bool fail_next;
} tracked_heap_t;

static void *tracked_alloc(void *ctx, size_t length)
{
    tracked_heap_t *heap = ctx;
    if (heap->fail_next) {
        heap->fail_next = false;
        return NULL;
    }
    void *ptr = malloc(length);
    if (ptr != NULL) {
        heap->allocations++;
        heap->live++;
    }
    return ptr;
}

static void tracked_free(void *ctx, void *ptr)
{
    tracked_heap_t *heap = ctx;
    assert(ptr != NULL);
    assert(heap->live > 0U);
    free(ptr);
    heap->frees++;
    heap->live--;
}

/* A deterministic test digest, injected through the same callback seam the
 * firmware adapter uses for SHA-256. The pure store is responsible for
 * comparing content to the address; the hash implementation is deliberately
 * outside core/ so that core stays plain C11. */
static void digest_bytes(const uint8_t *bytes, size_t length,
                         uint8_t out[ASSET_DIGEST_BYTES])
{
    uint32_t state = UINT32_C(2166136261);
    for (size_t i = 0U; i < length; ++i) {
        state ^= bytes[i];
        state *= UINT32_C(16777619);
    }
    for (size_t i = 0U; i < ASSET_DIGEST_BYTES; ++i) {
        state ^= (uint32_t)i + UINT32_C(0x9e3779b9);
        state *= UINT32_C(16777619);
        out[i] = (uint8_t)(state >> ((i % 4U) * 8U));
    }
}

static bool tracked_digest_matches(void *ctx, const void *bytes, size_t length,
                                   const uint8_t expected[ASSET_DIGEST_BYTES])
{
    (void)ctx;
    uint8_t actual[ASSET_DIGEST_BYTES];
    digest_bytes(bytes, length, actual);
    return memcmp(actual, expected, sizeof actual) == 0;
}

static volatile_asset_store_t new_store(tracked_heap_t *heap)
{
    volatile_asset_store_t store;
    volatile_asset_store_callbacks_t callbacks = {
        .allocate = tracked_alloc,
        .deallocate = tracked_free,
        .digest_matches = tracked_digest_matches,
        .ctx = heap,
    };
    assert(volatile_asset_store_init(&store, &callbacks) ==
           VOLATILE_ASSET_STORE_OK);
    return store;
}

static uint8_t *new_frame(uint8_t seed,
                          uint8_t digest[ASSET_DIGEST_BYTES])
{
    uint8_t *frame = malloc(VOLATILE_ASSET_FRAME_BYTES);
    assert(frame != NULL);
    memset(frame, seed, VOLATILE_ASSET_FRAME_BYTES);
    frame[0] = VOLATILE_ASSET_IMAGE_MAGIC;
    frame[1] = VOLATILE_ASSET_RGB565_FORMAT;
    frame[2] = 0U;
    frame[3] = 0U;
    frame[4] = (uint8_t)SCENE_CANVAS_WIDTH;
    frame[5] = (uint8_t)((uint32_t)SCENE_CANVAS_WIDTH >> 8U);
    frame[6] = (uint8_t)SCENE_CANVAS_HEIGHT;
    frame[7] = (uint8_t)((uint32_t)SCENE_CANVAS_HEIGHT >> 8U);
    frame[8] = (uint8_t)VOLATILE_ASSET_FRAME_STRIDE;
    frame[9] = (uint8_t)(VOLATILE_ASSET_FRAME_STRIDE >> 8U);
    frame[10] = 0U;
    frame[11] = 0U;
    digest_bytes(frame, VOLATILE_ASSET_FRAME_BYTES, digest);
    return frame;
}

static void send_frame_chunks(volatile_asset_store_t *store,
                              const uint8_t digest[ASSET_DIGEST_BYTES],
                              const uint8_t *frame)
{
    uint32_t offset = 0U;
    while (offset < VOLATILE_ASSET_FRAME_BYTES) {
        uint32_t remaining = VOLATILE_ASSET_FRAME_BYTES - offset;
        uint32_t length = remaining > 1920U ? 1920U : remaining;
        assert(volatile_asset_store_write(store, digest, offset,
                                          frame + offset, length) ==
               VOLATILE_ASSET_STORE_OK);
        offset += length;
    }
}

static uint8_t *new_rle_wire(const uint8_t *frame, size_t *out_length)
{
    const uint32_t pixel_count = VOLATILE_ASSET_PIXEL_BYTES / 2U;
    const size_t run_count =
        (pixel_count + (uint32_t)UINT16_MAX - 1U) / (uint32_t)UINT16_MAX;
    uint8_t *wire = malloc(VOLATILE_ASSET_HEADER_BYTES + run_count * 4U);
    assert(wire != NULL);
    memcpy(wire, frame, VOLATILE_ASSET_HEADER_BYTES);
    size_t offset = VOLATILE_ASSET_HEADER_BYTES;
    uint32_t remaining = pixel_count;
    while (remaining > 0U) {
        uint16_t count = remaining > (uint32_t)UINT16_MAX
                             ? UINT16_MAX
                             : (uint16_t)remaining;
        wire[offset++] = (uint8_t)count;
        wire[offset++] = (uint8_t)(count >> 8U);
        wire[offset++] = frame[VOLATILE_ASSET_HEADER_BYTES];
        wire[offset++] = frame[VOLATILE_ASSET_HEADER_BYTES + 1U];
        remaining -= count;
    }
    *out_length = offset;
    return wire;
}

static void commit_frame(volatile_asset_store_t *store, uint8_t seed,
                         uint8_t digest[ASSET_DIGEST_BYTES])
{
    uint8_t *frame = new_frame(seed, digest);
    assert(volatile_asset_store_begin(store, digest, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_OK);
    send_frame_chunks(store, digest, frame);
    assert(volatile_asset_store_commit(store, digest) ==
           VOLATILE_ASSET_STORE_OK);
    free(frame);
}

static void assert_found(const volatile_asset_store_t *store,
                         const uint8_t digest[ASSET_DIGEST_BYTES],
                         uint8_t pixel_seed)
{
    const void *ptr = NULL;
    uint32_t length = 0U;
    uint8_t kind = 0U;
    assert(volatile_asset_store_find(store, digest, &ptr, &length, &kind) ==
           VOLATILE_ASSET_STORE_OK);
    assert(ptr != NULL);
    assert(length == VOLATILE_ASSET_FRAME_BYTES);
    assert(kind == ASSET_KIND_IMAGE);
    assert(((const uint8_t *)ptr)[VOLATILE_ASSET_HEADER_BYTES] == pixel_seed);
}

static void test_begin_ordered_chunks_commit_and_find(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t digest[ASSET_DIGEST_BYTES];
    uint8_t *frame = new_frame(0x31U, digest);

    assert(volatile_asset_store_begin(&store, digest, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_OK);
    send_frame_chunks(&store, digest, frame);
    assert(volatile_asset_store_commit(&store, digest) ==
           VOLATILE_ASSET_STORE_OK);
    assert_found(&store, digest, 0x31U);

    free(frame);
    volatile_asset_store_destroy(&store);
    assert(heap.live == 0U);
}

static void test_rle_begin_one_byte_chunks_commit_and_find(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t digest[ASSET_DIGEST_BYTES];
    uint8_t *frame = new_frame(0x35U, digest);
    size_t wire_length = 0U;
    uint8_t *wire = new_rle_wire(frame, &wire_length);

    assert(volatile_asset_store_begin_encoded(
               &store, digest, ASSET_KIND_IMAGE, (uint32_t)wire_length,
               VOLATILE_ASSET_ENCODING_RLE565,
               VOLATILE_ASSET_FRAME_BYTES) == VOLATILE_ASSET_STORE_OK);
    for (uint32_t offset = 0U; offset < (uint32_t)wire_length; ++offset) {
        assert(volatile_asset_store_write(&store, digest, offset,
                                          wire + offset, 1U) ==
               VOLATILE_ASSET_STORE_OK);
    }
    assert(volatile_asset_store_commit(&store, digest) ==
           VOLATILE_ASSET_STORE_OK);
    assert_found(&store, digest, 0x35U);

    free(wire);
    free(frame);
    volatile_asset_store_destroy(&store);
    assert(heap.live == 0U);
}

static void test_duplicate_begin_reports_already_present(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t digest[ASSET_DIGEST_BYTES];
    commit_frame(&store, 0x32U, digest);
    size_t allocations = heap.allocations;

    assert(volatile_asset_store_begin(&store, digest, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_ALREADY_PRESENT);
    assert(heap.allocations == allocations);
    assert_found(&store, digest, 0x32U);
    volatile_asset_store_destroy(&store);
}

static void test_second_begin_frees_only_the_interrupted_incoming_frame(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t active[ASSET_DIGEST_BYTES];
    uint8_t interrupted[ASSET_DIGEST_BYTES];
    uint8_t replacement[ASSET_DIGEST_BYTES];
    uint8_t *frame_b = new_frame(0x42U, interrupted);
    uint8_t *frame_c = new_frame(0x43U, replacement);
    commit_frame(&store, 0x41U, active);

    assert(volatile_asset_store_begin(&store, interrupted, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_OK);
    assert(volatile_asset_store_write(&store, interrupted, 0U, frame_b, 1920U) ==
           VOLATILE_ASSET_STORE_OK);
    assert(heap.live == 2U);
    assert(volatile_asset_store_begin(&store, replacement, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_OK);
    assert(heap.frees == 1U);
    assert(heap.live == 2U);
    assert_found(&store, active, 0x41U);

    free(frame_b);
    free(frame_c);
    volatile_asset_store_destroy(&store);
    assert(heap.live == 0U);
}

static void test_third_live_allocation_is_refused(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t first[ASSET_DIGEST_BYTES];
    uint8_t second[ASSET_DIGEST_BYTES];
    uint8_t third[ASSET_DIGEST_BYTES];
    uint8_t *frame = new_frame(0x53U, third);
    commit_frame(&store, 0x51U, first);
    commit_frame(&store, 0x52U, second);

    assert(heap.live == VOLATILE_ASSET_SLOT_COUNT);
    assert(volatile_asset_store_begin(&store, third, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_ERR_FULL);
    assert(heap.live == VOLATILE_ASSET_SLOT_COUNT);
    assert_found(&store, first, 0x51U);
    assert_found(&store, second, 0x52U);

    free(frame);
    volatile_asset_store_destroy(&store);
}

static void test_release_frees_exactly_digests_absent_from_the_keep_set(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t keep_digest[ASSET_DIGEST_BYTES];
    uint8_t drop_digest[ASSET_DIGEST_BYTES];
    commit_frame(&store, 0x61U, keep_digest);
    commit_frame(&store, 0x62U, drop_digest);
    const uint8_t *keep[] = {keep_digest};
    size_t released = 0U;

    assert(volatile_asset_store_release(&store, keep, 1U, &released) ==
           VOLATILE_ASSET_STORE_OK);
    assert(released == 1U);
    assert(heap.frees == 1U);
    assert_found(&store, keep_digest, 0x61U);
    assert(volatile_asset_store_find(&store, drop_digest, NULL, NULL, NULL) ==
           VOLATILE_ASSET_STORE_ERR_NOT_FOUND);

    assert(volatile_asset_store_release(&store, NULL, 0U, &released) ==
           VOLATILE_ASSET_STORE_OK);
    assert(released == 1U);
    assert(heap.live == 0U);
    volatile_asset_store_destroy(&store);
    assert(heap.frees == 2U);
}

static void test_non_image_zero_and_over_limit_begins_are_rejected(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t active[ASSET_DIGEST_BYTES];
    uint8_t digest[ASSET_DIGEST_BYTES] = {0};
    commit_frame(&store, 0x6fU, active);

    assert(volatile_asset_store_begin(&store, digest, ASSET_KIND_FONT,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_ERR_KIND);
    assert(volatile_asset_store_begin(&store, digest, ASSET_KIND_IMAGE, 0U) ==
           VOLATILE_ASSET_STORE_ERR_LENGTH);
    assert(volatile_asset_store_begin(&store, digest, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES + 1U) ==
           VOLATILE_ASSET_STORE_ERR_LENGTH);
    assert(heap.live == 1U);
    assert_found(&store, active, 0x6fU);
    volatile_asset_store_destroy(&store);
}

static void test_wrong_digest_refuses_commit_without_losing_prior_frame(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t active[ASSET_DIGEST_BYTES];
    uint8_t wrong[ASSET_DIGEST_BYTES];
    uint8_t actual[ASSET_DIGEST_BYTES];
    uint8_t *frame = new_frame(0x72U, actual);
    memcpy(wrong, actual, sizeof wrong);
    wrong[0] ^= 0xffU;
    commit_frame(&store, 0x71U, active);

    assert(volatile_asset_store_begin(&store, wrong, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_OK);
    send_frame_chunks(&store, wrong, frame);
    assert(volatile_asset_store_commit(&store, wrong) ==
           VOLATILE_ASSET_STORE_ERR_DIGEST);
    assert(heap.live == 1U);
    assert_found(&store, active, 0x71U);

    free(frame);
    volatile_asset_store_destroy(&store);
}

static void assert_bad_chunk_preserves_active(uint32_t first_offset,
                                              uint32_t first_length,
                                              uint32_t bad_offset,
                                              uint32_t bad_length,
                                              volatile_asset_store_result_t expected)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t active[ASSET_DIGEST_BYTES];
    uint8_t incoming[ASSET_DIGEST_BYTES];
    uint8_t *frame = new_frame(0x82U, incoming);
    commit_frame(&store, 0x81U, active);
    assert(volatile_asset_store_begin(&store, incoming, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_OK);
    if (first_length > 0U) {
        assert(volatile_asset_store_write(&store, incoming, first_offset,
                                          frame + first_offset, first_length) ==
               VOLATILE_ASSET_STORE_OK);
    }
    assert(volatile_asset_store_write(&store, incoming, bad_offset,
                                      frame + bad_offset, bad_length) == expected);
    assert(heap.live == 1U);
    assert_found(&store, active, 0x81U);
    free(frame);
    volatile_asset_store_destroy(&store);
}

static void test_duplicate_chunk_is_rejected_and_aborted(void)
{
    assert_bad_chunk_preserves_active(0U, 1920U, 0U, 1920U,
                                      VOLATILE_ASSET_STORE_ERR_OFFSET);
}

static void test_out_of_order_chunk_is_rejected_and_aborted(void)
{
    assert_bad_chunk_preserves_active(0U, 0U, 1920U, 1920U,
                                      VOLATILE_ASSET_STORE_ERR_OFFSET);
}

static void test_overlapping_chunk_is_rejected_and_aborted(void)
{
    assert_bad_chunk_preserves_active(0U, 1920U, 960U, 1920U,
                                      VOLATILE_ASSET_STORE_ERR_OFFSET);
}

static void test_incomplete_commit_is_rejected_and_aborted(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t active[ASSET_DIGEST_BYTES];
    uint8_t incoming[ASSET_DIGEST_BYTES];
    uint8_t *frame = new_frame(0x92U, incoming);
    commit_frame(&store, 0x91U, active);
    assert(volatile_asset_store_begin(&store, incoming, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_OK);
    assert(volatile_asset_store_write(&store, incoming, 0U, frame, 1920U) ==
           VOLATILE_ASSET_STORE_OK);
    assert(volatile_asset_store_commit(&store, incoming) ==
           VOLATILE_ASSET_STORE_ERR_INCOMPLETE);
    assert(heap.live == 1U);
    assert_found(&store, active, 0x91U);
    free(frame);
    volatile_asset_store_destroy(&store);
}

static void assert_corrupt_header_rejected(size_t offset,
                                           const uint8_t *replacement,
                                           size_t replacement_length)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t active[ASSET_DIGEST_BYTES];
    uint8_t incoming[ASSET_DIGEST_BYTES];
    uint8_t *frame = new_frame(0xa2U, incoming);
    memcpy(frame + offset, replacement, replacement_length);
    digest_bytes(frame, VOLATILE_ASSET_FRAME_BYTES, incoming);
    commit_frame(&store, 0xa1U, active);
    assert(volatile_asset_store_begin(&store, incoming, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_OK);
    send_frame_chunks(&store, incoming, frame);
    assert(volatile_asset_store_commit(&store, incoming) ==
           VOLATILE_ASSET_STORE_ERR_FRAME);
    assert(heap.live == 1U);
    assert_found(&store, active, 0xa1U);
    free(frame);
    volatile_asset_store_destroy(&store);
}

static void test_malformed_header_is_rejected(void)
{
    static const uint8_t nonzero_reserved[] = {1U};
    assert_corrupt_header_rejected(2U, nonzero_reserved,
                                   sizeof(nonzero_reserved));
}

static void test_wrong_magic_format_dimensions_and_stride_are_rejected(void)
{
    static const struct {
        size_t offset;
        uint8_t replacement[2];
        size_t replacement_length;
    } cases[] = {
        {0U, {0U, 0U}, 1U},
        {1U, {0U, 0U}, 1U},
        {4U, {0U, 0U}, 1U},
        {6U, {0U, 0U}, 1U},
        {8U, {0U, 0U}, 1U},
        /* Hostile but arithmetic-free: 0xffff is noncanonical width. */
        {4U, {0xffU, 0xffU}, 2U},
    };
    for (size_t i = 0U; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        assert_corrupt_header_rejected(cases[i].offset,
                                       cases[i].replacement,
                                       cases[i].replacement_length);
    }
}

static void test_allocation_failure_preserves_prior_frame(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t active[ASSET_DIGEST_BYTES];
    uint8_t incoming[ASSET_DIGEST_BYTES];
    uint8_t *frame = new_frame(0xb2U, incoming);
    commit_frame(&store, 0xb1U, active);
    heap.fail_next = true;

    assert(volatile_asset_store_begin(&store, incoming, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_ERR_ALLOC);
    assert(heap.live == 1U);
    assert_found(&store, active, 0xb1U);
    free(frame);
    volatile_asset_store_destroy(&store);
}

static void assert_bad_rle_preserves_active(const uint8_t *runs,
                                            uint32_t run_length,
                                            bool failure_on_write)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t active[ASSET_DIGEST_BYTES];
    uint8_t incoming[ASSET_DIGEST_BYTES] = {0xe2U};
    uint8_t header[VOLATILE_ASSET_HEADER_BYTES];
    uint8_t header_digest[ASSET_DIGEST_BYTES];
    uint8_t *frame = new_frame(0xe1U, header_digest);
    memcpy(header, frame, sizeof header);
    free(frame);
    commit_frame(&store, 0xe0U, active);

    uint32_t wire_length = VOLATILE_ASSET_HEADER_BYTES + run_length;
    assert(volatile_asset_store_begin_encoded(
               &store, incoming, ASSET_KIND_IMAGE, wire_length,
               VOLATILE_ASSET_ENCODING_RLE565,
               VOLATILE_ASSET_FRAME_BYTES) == VOLATILE_ASSET_STORE_OK);
    assert(volatile_asset_store_write(&store, incoming, 0U, header,
                                      sizeof header) ==
           VOLATILE_ASSET_STORE_OK);
    volatile_asset_store_result_t write_result = volatile_asset_store_write(
        &store, incoming, sizeof header, runs, run_length);
    if (failure_on_write) {
        assert(write_result == VOLATILE_ASSET_STORE_ERR_DECODE);
    } else {
        assert(write_result == VOLATILE_ASSET_STORE_OK);
        assert(volatile_asset_store_commit(&store, incoming) ==
               VOLATILE_ASSET_STORE_ERR_DECODE);
    }
    assert(heap.live == 1U);
    assert_found(&store, active, 0xe0U);
    volatile_asset_store_destroy(&store);
}

static void test_zero_count_overrun_and_trailing_rle_preserve_prior_frame(void)
{
    const uint8_t zero[] = {0, 0, 0x34, 0x12};
    assert_bad_rle_preserves_active(zero, sizeof zero, true);

    const uint8_t overrun[] = {
        0xff, 0xff, 0x34, 0x12,
        0xff, 0xff, 0x34, 0x12,
        0x04, 0x84, 0x34, 0x12,
    };
    assert_bad_rle_preserves_active(overrun, sizeof overrun, true);

    const uint8_t trailing[] = {
        0xff, 0xff, 0x34, 0x12,
        0xff, 0xff, 0x34, 0x12,
        0x02, 0x84, 0x34, 0x12,
        0xff,
    };
    assert_bad_rle_preserves_active(trailing, sizeof trailing, true);
}

static void test_truncated_and_short_rle_preserve_prior_frame(void)
{
    const uint8_t truncated[] = {1, 0, 0x34};
    assert_bad_rle_preserves_active(truncated, sizeof truncated, false);
    const uint8_t short_output[] = {1, 0, 0x34, 0x12};
    assert_bad_rle_preserves_active(short_output, sizeof short_output, false);
}

static void test_expanding_rle_is_refused_before_allocation(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t active[ASSET_DIGEST_BYTES];
    uint8_t incoming[ASSET_DIGEST_BYTES] = {0xe3U};
    commit_frame(&store, 0xe2U, active);

    assert(volatile_asset_store_begin_encoded(
               &store, incoming, ASSET_KIND_IMAGE,
               VOLATILE_ASSET_FRAME_BYTES,
               VOLATILE_ASSET_ENCODING_RLE565,
               VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_ERR_LENGTH);
    assert(heap.live == 1U);
    assert_found(&store, active, 0xe2U);
    volatile_asset_store_destroy(&store);
}

static void test_release_teardown_predicate_distinguishes_kept_volatile_bytes(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t old_digest[ASSET_DIGEST_BYTES];
    uint8_t new_digest[ASSET_DIGEST_BYTES];
    uint8_t durable_digest[ASSET_DIGEST_BYTES] = {0xddU};
    commit_frame(&store, 0xc1U, old_digest);
    commit_frame(&store, 0xc2U, new_digest);
    const uint8_t *keep_new[] = {new_digest};
    const uint8_t *use_new[] = {new_digest};
    const uint8_t *use_old[] = {old_digest};
    const uint8_t *use_durable[] = {durable_digest};

    assert(!volatile_asset_store_release_must_teardown(
        &store, use_new, 1U, keep_new, 1U));
    assert(volatile_asset_store_release_must_teardown(
        &store, use_old, 1U, keep_new, 1U));
    assert(volatile_asset_store_release_must_teardown(
        &store, use_durable, 1U, keep_new, 1U));
    assert(!volatile_asset_store_release_must_teardown(
        &store, NULL, 0U, keep_new, 1U));
    volatile_asset_store_destroy(&store);
}

static void test_abort_and_destroy_free_each_allocation_exactly_once(void)
{
    tracked_heap_t heap = {0};
    volatile_asset_store_t store = new_store(&heap);
    uint8_t active[ASSET_DIGEST_BYTES];
    uint8_t incoming[ASSET_DIGEST_BYTES];
    uint8_t *frame = new_frame(0xd2U, incoming);
    commit_frame(&store, 0xd1U, active);
    assert(volatile_asset_store_begin(&store, incoming, ASSET_KIND_IMAGE,
                                      VOLATILE_ASSET_FRAME_BYTES) ==
           VOLATILE_ASSET_STORE_OK);
    assert(volatile_asset_store_write(&store, incoming, 0U, frame, 1920U) ==
           VOLATILE_ASSET_STORE_OK);
    volatile_asset_store_abort_incoming(&store);
    volatile_asset_store_abort_incoming(&store);
    assert(heap.live == 1U);
    assert(heap.frees == 1U);
    volatile_asset_store_destroy(&store);
    volatile_asset_store_destroy(&store);
    assert(heap.live == 0U);
    assert(heap.frees == 2U);
    free(frame);
}

int main(void)
{
    test_begin_ordered_chunks_commit_and_find();
    test_rle_begin_one_byte_chunks_commit_and_find();
    test_duplicate_begin_reports_already_present();
    test_second_begin_frees_only_the_interrupted_incoming_frame();
    test_third_live_allocation_is_refused();
    test_release_frees_exactly_digests_absent_from_the_keep_set();
    test_non_image_zero_and_over_limit_begins_are_rejected();
    test_wrong_digest_refuses_commit_without_losing_prior_frame();
    test_duplicate_chunk_is_rejected_and_aborted();
    test_out_of_order_chunk_is_rejected_and_aborted();
    test_overlapping_chunk_is_rejected_and_aborted();
    test_incomplete_commit_is_rejected_and_aborted();
    test_malformed_header_is_rejected();
    test_wrong_magic_format_dimensions_and_stride_are_rejected();
    test_allocation_failure_preserves_prior_frame();
    test_zero_count_overrun_and_trailing_rle_preserve_prior_frame();
    test_truncated_and_short_rle_preserve_prior_frame();
    test_expanding_rle_is_refused_before_allocation();
    test_release_teardown_predicate_distinguishes_kept_volatile_bytes();
    test_abort_and_destroy_free_each_allocation_exactly_once();
    puts("test_volatile_asset_store: OK (21 tests)");
    return 0;
}
