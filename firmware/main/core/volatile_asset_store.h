#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "core/asset_transfer.h"
#include "core/rle565.h"
#include "core/scene_model.h"

/* One displayed frame plus one incoming replacement is the minimum bound
 * that permits an atomic swap. A larger directory would permit unbounded
 * accumulation in roughly 330 KiB steps. The slots are metadata only and
 * live inside the caller-provided store object; frame bytes are always
 * allocated through the callbacks below. */
#define VOLATILE_ASSET_SLOT_COUNT 2U
#define VOLATILE_ASSET_HEADER_BYTES 12U
#define VOLATILE_ASSET_IMAGE_MAGIC 0x19U
#define VOLATILE_ASSET_RGB565_FORMAT 0x12U
#define VOLATILE_ASSET_ENCODING_RAW 0U
#define VOLATILE_ASSET_ENCODING_RLE565 1U
#define VOLATILE_ASSET_FRAME_STRIDE ((uint32_t)SCENE_CANVAS_WIDTH * 2U)
#define VOLATILE_ASSET_PIXEL_BYTES                                      \
    ((uint32_t)SCENE_CANVAS_WIDTH * (uint32_t)SCENE_CANVAS_HEIGHT * 2U)
#define VOLATILE_ASSET_FRAME_BYTES                                      \
    (VOLATILE_ASSET_HEADER_BYTES + VOLATILE_ASSET_PIXEL_BYTES)

_Static_assert(VOLATILE_ASSET_FRAME_BYTES == 329740U,
               "canonical 448x368 RGB565 frame size changed");
_Static_assert(VOLATILE_ASSET_FRAME_BYTES <= ASSET_MAX_BYTES,
               "volatile frames must fit the protocol asset bound");

typedef void *(*volatile_asset_allocate_fn)(void *ctx, size_t length);
typedef void (*volatile_asset_deallocate_fn)(void *ctx, void *ptr);
typedef bool (*volatile_asset_digest_matches_fn)(
    void *ctx, const void *bytes, size_t length,
    const uint8_t expected[ASSET_DIGEST_BYTES]);

typedef struct {
    volatile_asset_allocate_fn allocate;
    volatile_asset_deallocate_fn deallocate;
    volatile_asset_digest_matches_fn digest_matches;
    void *ctx;
} volatile_asset_store_callbacks_t;

typedef enum {
    VOLATILE_ASSET_SLOT_EMPTY = 0,
    VOLATILE_ASSET_SLOT_INCOMING,
    VOLATILE_ASSET_SLOT_COMMITTED,
} volatile_asset_slot_state_t;

typedef struct {
    uint8_t digest[ASSET_DIGEST_BYTES];
    uint8_t *bytes;
    uint32_t length;
    uint8_t kind;
    volatile_asset_slot_state_t state;
} volatile_asset_slot_t;

typedef struct {
    volatile_asset_store_callbacks_t callbacks;
    volatile_asset_slot_t slots[VOLATILE_ASSET_SLOT_COUNT];
    asset_transfer_t incoming_transfer;
    rle565_decoder_t rle_decoder;
    uint8_t incoming_encoding;
    int8_t incoming_index;
} volatile_asset_store_t;

typedef enum {
    VOLATILE_ASSET_STORE_OK = 0,
    VOLATILE_ASSET_STORE_ALREADY_PRESENT,
    VOLATILE_ASSET_STORE_ERR_ARGUMENT,
    VOLATILE_ASSET_STORE_ERR_KIND,
    VOLATILE_ASSET_STORE_ERR_LENGTH,
    VOLATILE_ASSET_STORE_ERR_FULL,
    VOLATILE_ASSET_STORE_ERR_ALLOC,
    VOLATILE_ASSET_STORE_ERR_INACTIVE,
    VOLATILE_ASSET_STORE_ERR_DIGEST,
    VOLATILE_ASSET_STORE_ERR_OFFSET,
    VOLATILE_ASSET_STORE_ERR_INCOMPLETE,
    VOLATILE_ASSET_STORE_ERR_DECODE,
    VOLATILE_ASSET_STORE_ERR_FRAME,
    VOLATILE_ASSET_STORE_ERR_NOT_FOUND,
} volatile_asset_store_result_t;

volatile_asset_store_result_t volatile_asset_store_init(
    volatile_asset_store_t *store,
    const volatile_asset_store_callbacks_t *callbacks);

volatile_asset_store_result_t volatile_asset_store_begin(
    volatile_asset_store_t *store,
    const uint8_t digest[ASSET_DIGEST_BYTES], uint8_t kind,
    uint32_t total_length);

/* Extended entry point for the wire decoder. `total_length` is the encoded
 * wire length; raw callers keep using volatile_asset_store_begin(). For RLE,
 * `decoded_length` is the allocation size and exact decoder capacity. */
volatile_asset_store_result_t volatile_asset_store_begin_encoded(
    volatile_asset_store_t *store,
    const uint8_t digest[ASSET_DIGEST_BYTES], uint8_t kind,
    uint32_t total_length, uint8_t encoding, uint32_t decoded_length);

volatile_asset_store_result_t volatile_asset_store_write(
    volatile_asset_store_t *store,
    const uint8_t digest[ASSET_DIGEST_BYTES], uint32_t offset,
    const void *data, uint32_t length);

volatile_asset_store_result_t volatile_asset_store_commit(
    volatile_asset_store_t *store,
    const uint8_t digest[ASSET_DIGEST_BYTES]);

volatile_asset_store_result_t volatile_asset_store_find(
    const volatile_asset_store_t *store,
    const uint8_t digest[ASSET_DIGEST_BYTES], const void **out_ptr,
    uint32_t *out_length, uint8_t *out_kind);

void volatile_asset_store_abort_incoming(volatile_asset_store_t *store);

/* `keep` is AssetRelease's complete keep-set, never a delete list. The
 * incoming allocation is always abandoned first; committed slots absent
 * from the set are then freed exactly once. */
volatile_asset_store_result_t volatile_asset_store_release(
    volatile_asset_store_t *store, const uint8_t *const *keep,
    size_t keep_count, size_t *out_released_count);

void volatile_asset_store_destroy(volatile_asset_store_t *store);

/* Pure keep/use predicate for AssetRelease's display teardown decision.
 * A kept volatile allocation never moves, so a scene reading only that
 * allocation need not flap through the standalone clock while an old frame
 * is freed. Any used digest that is absent from the keep-set, or that is not
 * a committed volatile digest (and may therefore be moved by durable flash
 * compaction), requires teardown. */
bool volatile_asset_store_release_must_teardown(
    const volatile_asset_store_t *store,
    const uint8_t *const *used_digests, size_t used_count,
    const uint8_t *const *keep, size_t keep_count);
