#include "core/volatile_asset_store.h"

#include <limits.h>
#include <string.h>

/* No frame buffer, slot array, or mutable metadata lives at file scope.
 * About 105 bytes of static internal DRAM once shifted this firmware's heap
 * layout enough to break every OTA download with all tests green. The caller
 * owns volatile_asset_store_t (protocol_task puts it in its PSRAM context),
 * and every 329,740-byte frame comes from the injected PSRAM allocator. */

static uint16_t read_u16_le(const uint8_t *bytes)
{
    return (uint16_t)((uint16_t)bytes[0] | ((uint16_t)bytes[1] << 8U));
}

static bool digest_in_set(const uint8_t digest[ASSET_DIGEST_BYTES],
                          const uint8_t *const *set, size_t count)
{
    for (size_t i = 0U; i < count; ++i) {
        if (memcmp(digest, set[i], ASSET_DIGEST_BYTES) == 0) {
            return true;
        }
    }
    return false;
}

static void clear_slot(volatile_asset_slot_t *slot)
{
    memset(slot, 0, sizeof *slot);
}

static void free_slot(volatile_asset_store_t *store,
                      volatile_asset_slot_t *slot)
{
    if (slot->bytes != NULL && store->callbacks.deallocate != NULL) {
        store->callbacks.deallocate(store->callbacks.ctx, slot->bytes);
    }
    clear_slot(slot);
}

static int find_committed_index(const volatile_asset_store_t *store,
                                const uint8_t digest[ASSET_DIGEST_BYTES])
{
    for (size_t i = 0U; i < VOLATILE_ASSET_SLOT_COUNT; ++i) {
        if (store->slots[i].state == VOLATILE_ASSET_SLOT_COMMITTED &&
            memcmp(store->slots[i].digest, digest, ASSET_DIGEST_BYTES) == 0) {
            return (int)i;
        }
    }
    return -1;
}

volatile_asset_store_result_t volatile_asset_store_init(
    volatile_asset_store_t *store,
    const volatile_asset_store_callbacks_t *callbacks)
{
    if (store == NULL || callbacks == NULL || callbacks->allocate == NULL ||
        callbacks->deallocate == NULL || callbacks->digest_matches == NULL) {
        return VOLATILE_ASSET_STORE_ERR_ARGUMENT;
    }
    memset(store, 0, sizeof *store);
    store->callbacks = *callbacks;
    store->incoming_index = -1;
    return VOLATILE_ASSET_STORE_OK;
}

void volatile_asset_store_abort_incoming(volatile_asset_store_t *store)
{
    if (store == NULL || store->callbacks.deallocate == NULL) {
        return;
    }
    if (store->incoming_index >= 0 &&
        store->incoming_index < (int8_t)VOLATILE_ASSET_SLOT_COUNT) {
        volatile_asset_slot_t *slot = &store->slots[store->incoming_index];
        if (slot->state == VOLATILE_ASSET_SLOT_INCOMING) {
            free_slot(store, slot);
        }
    }
    asset_transfer_abort(&store->incoming_transfer);
    store->incoming_index = -1;
}

volatile_asset_store_result_t volatile_asset_store_begin(
    volatile_asset_store_t *store,
    const uint8_t digest[ASSET_DIGEST_BYTES], uint8_t kind,
    uint32_t total_length)
{
    if (store == NULL || digest == NULL || store->callbacks.allocate == NULL ||
        store->callbacks.deallocate == NULL ||
        store->callbacks.digest_matches == NULL) {
        return VOLATILE_ASSET_STORE_ERR_ARGUMENT;
    }
    if (kind != ASSET_KIND_IMAGE) {
        return VOLATILE_ASSET_STORE_ERR_KIND;
    }
    /* Raw only: a volatile asset is exactly one canonical full-panel frame.
     * No RLE decoder or decoded-size ambiguity exists in this task. */
    if (total_length != VOLATILE_ASSET_FRAME_BYTES) {
        return VOLATILE_ASSET_STORE_ERR_LENGTH;
    }

    /* Every valid second begin supersedes the unfinished one, regardless of
     * digest. The previously committed/displayed allocation is another slot
     * and is never touched by this abort. */
    volatile_asset_store_abort_incoming(store);
    if (find_committed_index(store, digest) >= 0) {
        return VOLATILE_ASSET_STORE_ALREADY_PRESENT;
    }

    int empty_index = -1;
    for (size_t i = 0U; i < VOLATILE_ASSET_SLOT_COUNT; ++i) {
        if (store->slots[i].state == VOLATILE_ASSET_SLOT_EMPTY) {
            empty_index = (int)i;
            break;
        }
    }
    if (empty_index < 0) {
        return VOLATILE_ASSET_STORE_ERR_FULL;
    }

    uint8_t *bytes = store->callbacks.allocate(store->callbacks.ctx,
                                                total_length);
    if (bytes == NULL) {
        return VOLATILE_ASSET_STORE_ERR_ALLOC;
    }
    volatile_asset_slot_t *slot = &store->slots[empty_index];
    clear_slot(slot);
    memcpy(slot->digest, digest, ASSET_DIGEST_BYTES);
    slot->bytes = bytes;
    slot->length = total_length;
    slot->kind = kind;
    slot->state = VOLATILE_ASSET_SLOT_INCOMING;
    if (asset_transfer_begin(&store->incoming_transfer, digest, kind,
                             total_length, true) != ASSET_TRANSFER_OK) {
        free_slot(store, slot);
        return VOLATILE_ASSET_STORE_ERR_ARGUMENT;
    }
    store->incoming_index = (int8_t)empty_index;
    return VOLATILE_ASSET_STORE_OK;
}

volatile_asset_store_result_t volatile_asset_store_write(
    volatile_asset_store_t *store,
    const uint8_t digest[ASSET_DIGEST_BYTES], uint32_t offset,
    const void *data, uint32_t length)
{
    if (store == NULL || digest == NULL || data == NULL) {
        return VOLATILE_ASSET_STORE_ERR_ARGUMENT;
    }
    if (store->incoming_index < 0 ||
        store->incoming_index >= (int8_t)VOLATILE_ASSET_SLOT_COUNT) {
        return VOLATILE_ASSET_STORE_ERR_INACTIVE;
    }
    asset_transfer_result_t accepted = asset_transfer_accept_chunk(
        &store->incoming_transfer, digest, offset, length);
    if (accepted != ASSET_TRANSFER_OK) {
        volatile_asset_store_result_t result =
            accepted == ASSET_TRANSFER_ERR_DIGEST
                ? VOLATILE_ASSET_STORE_ERR_DIGEST
                : accepted == ASSET_TRANSFER_ERR_INACTIVE
                    ? VOLATILE_ASSET_STORE_ERR_INACTIVE
                    : accepted == ASSET_TRANSFER_ERR_LENGTH
                        ? VOLATILE_ASSET_STORE_ERR_LENGTH
                        : VOLATILE_ASSET_STORE_ERR_OFFSET;
        /* Durable transfers tolerate an exact retry after a lost Ack. A
         * volatile frame does not: this tier's hostile contract refuses any
         * duplicate/overlap and abandons the candidate atomically. */
        volatile_asset_store_abort_incoming(store);
        return result;
    }

    volatile_asset_slot_t *slot = &store->slots[store->incoming_index];
    memcpy(slot->bytes + offset, data, length);
    return VOLATILE_ASSET_STORE_OK;
}

bool volatile_asset_frame_valid(const uint8_t *bytes, size_t length)
{
    if (bytes == NULL || length < VOLATILE_ASSET_HEADER_BYTES) {
        return false;
    }
    uint32_t width = read_u16_le(bytes + 4U);
    uint32_t height = read_u16_le(bytes + 6U);
    uint32_t stride = read_u16_le(bytes + 8U);

    /* Check the decoded arithmetic before comparing canonical dimensions.
     * A hostile 0xffff x 0xffff header would overflow width*height*2 in
     * uint32_t even though each individual packed field is only 16 bits. */
    if (width != 0U && height > UINT32_MAX / width) {
        return false;
    }
    uint32_t pixels = width * height;
    if (pixels > (UINT32_MAX - VOLATILE_ASSET_HEADER_BYTES) / 2U) {
        return false;
    }
    uint32_t decoded_length =
        VOLATILE_ASSET_HEADER_BYTES + pixels * 2U;

    return bytes[0] == VOLATILE_ASSET_IMAGE_MAGIC &&
           bytes[1] == VOLATILE_ASSET_RGB565_FORMAT &&
           bytes[2] == 0U && bytes[3] == 0U &&
           width == (uint32_t)SCENE_CANVAS_WIDTH &&
           height == (uint32_t)SCENE_CANVAS_HEIGHT &&
           stride == VOLATILE_ASSET_FRAME_STRIDE &&
           bytes[10] == 0U && bytes[11] == 0U &&
           decoded_length == VOLATILE_ASSET_FRAME_BYTES &&
           length == decoded_length;
}

volatile_asset_store_result_t volatile_asset_store_commit(
    volatile_asset_store_t *store,
    const uint8_t digest[ASSET_DIGEST_BYTES])
{
    if (store == NULL || digest == NULL) {
        return VOLATILE_ASSET_STORE_ERR_ARGUMENT;
    }
    if (store->incoming_index < 0 ||
        store->incoming_index >= (int8_t)VOLATILE_ASSET_SLOT_COUNT ||
        !store->incoming_transfer.active) {
        return VOLATILE_ASSET_STORE_ERR_INACTIVE;
    }
    if (memcmp(digest, store->incoming_transfer.digest,
               ASSET_DIGEST_BYTES) != 0) {
        volatile_asset_store_abort_incoming(store);
        return VOLATILE_ASSET_STORE_ERR_DIGEST;
    }
    if (!asset_transfer_is_complete(&store->incoming_transfer)) {
        volatile_asset_store_abort_incoming(store);
        return VOLATILE_ASSET_STORE_ERR_INCOMPLETE;
    }

    volatile_asset_slot_t *slot = &store->slots[store->incoming_index];
    if (!store->callbacks.digest_matches(store->callbacks.ctx, slot->bytes,
                                         slot->length, slot->digest)) {
        volatile_asset_store_abort_incoming(store);
        return VOLATILE_ASSET_STORE_ERR_DIGEST;
    }
    if (!volatile_asset_frame_valid(slot->bytes, slot->length)) {
        volatile_asset_store_abort_incoming(store);
        return VOLATILE_ASSET_STORE_ERR_FRAME;
    }

    slot->state = VOLATILE_ASSET_SLOT_COMMITTED;
    asset_transfer_abort(&store->incoming_transfer);
    store->incoming_index = -1;
    return VOLATILE_ASSET_STORE_OK;
}

volatile_asset_store_result_t volatile_asset_store_find(
    const volatile_asset_store_t *store,
    const uint8_t digest[ASSET_DIGEST_BYTES], const void **out_ptr,
    uint32_t *out_length, uint8_t *out_kind)
{
    if (store == NULL || digest == NULL) {
        return VOLATILE_ASSET_STORE_ERR_ARGUMENT;
    }
    int index = find_committed_index(store, digest);
    if (index < 0) {
        return VOLATILE_ASSET_STORE_ERR_NOT_FOUND;
    }
    const volatile_asset_slot_t *slot = &store->slots[index];
    if (out_ptr != NULL) {
        *out_ptr = slot->bytes;
    }
    if (out_length != NULL) {
        *out_length = slot->length;
    }
    if (out_kind != NULL) {
        *out_kind = slot->kind;
    }
    return VOLATILE_ASSET_STORE_OK;
}

volatile_asset_store_result_t volatile_asset_store_release(
    volatile_asset_store_t *store, const uint8_t *const *keep,
    size_t keep_count, size_t *out_released_count)
{
    if (store == NULL || (keep_count > 0U && keep == NULL)) {
        return VOLATILE_ASSET_STORE_ERR_ARGUMENT;
    }
    volatile_asset_store_abort_incoming(store);
    size_t released = 0U;
    for (size_t i = 0U; i < VOLATILE_ASSET_SLOT_COUNT; ++i) {
        volatile_asset_slot_t *slot = &store->slots[i];
        if (slot->state == VOLATILE_ASSET_SLOT_COMMITTED &&
            !digest_in_set(slot->digest, keep, keep_count)) {
            free_slot(store, slot);
            released++;
        }
    }
    if (out_released_count != NULL) {
        *out_released_count = released;
    }
    return VOLATILE_ASSET_STORE_OK;
}

void volatile_asset_store_destroy(volatile_asset_store_t *store)
{
    if (store == NULL) {
        return;
    }
    if (store->callbacks.deallocate != NULL) {
        volatile_asset_store_abort_incoming(store);
        for (size_t i = 0U; i < VOLATILE_ASSET_SLOT_COUNT; ++i) {
            free_slot(store, &store->slots[i]);
        }
    }
    memset(store, 0, sizeof *store);
    store->incoming_index = -1;
}

bool volatile_asset_store_release_must_teardown(
    const volatile_asset_store_t *store,
    const uint8_t *const *used_digests, size_t used_count,
    const uint8_t *const *keep, size_t keep_count)
{
    if (store == NULL || (used_count > 0U && used_digests == NULL) ||
        (keep_count > 0U && keep == NULL)) {
        return used_count > 0U;
    }
    for (size_t i = 0U; i < used_count; ++i) {
        const uint8_t *digest = used_digests[i];
        if (digest == NULL || find_committed_index(store, digest) < 0 ||
            !digest_in_set(digest, keep, keep_count)) {
            return true;
        }
    }
    return false;
}

