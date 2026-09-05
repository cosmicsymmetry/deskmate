#include "asset_transfer.h"

#include <string.h>

static bool asset_kind_is_valid(uint8_t kind)
{
    switch (kind) {
    case ASSET_KIND_FONT:
    case ASSET_KIND_ICON_FONT:
    case ASSET_KIND_IMAGE:
        return true;
    default:
        return false;
    }
}

asset_transfer_result_t asset_transfer_begin(asset_transfer_t *transfer,
                                             const uint8_t *digest, uint8_t kind,
                                             uint32_t total_length)
{
    if (transfer == NULL || digest == NULL) {
        return ASSET_TRANSFER_ERR_ARGUMENT;
    }
    if (!asset_kind_is_valid(kind)) {
        return ASSET_TRANSFER_ERR_ARGUMENT;
    }
    /* Bounded the same way asset_store_reserve bounds a length: zero is not
     * a transfer, and anything past ASSET_MAX_BYTES can never be committed
     * to the store this transfer feeds. */
    if (total_length == 0U || total_length > ASSET_MAX_BYTES) {
        return ASSET_TRANSFER_ERR_LENGTH;
    }

    memset(transfer, 0, sizeof *transfer);
    memcpy(transfer->digest, digest, ASSET_DIGEST_BYTES);
    transfer->total_length = total_length;
    transfer->active = true;
    return ASSET_TRANSFER_OK;
}

asset_transfer_result_t asset_transfer_accept_chunk(asset_transfer_t *transfer,
                                                    const uint8_t *digest,
                                                    uint32_t offset,
                                                    uint32_t length)
{
    if (transfer == NULL || digest == NULL) {
        return ASSET_TRANSFER_ERR_ARGUMENT;
    }
    /* An inactive transfer has a zeroed digest, so this must be checked
     * before any digest comparison -- otherwise a chunk sent with an
     * all-zero digest against a never-begun transfer would read as a
     * digest match instead of the inactive case it actually is. */
    if (!transfer->active) {
        return ASSET_TRANSFER_ERR_INACTIVE;
    }
    if (memcmp(digest, transfer->digest, ASSET_DIGEST_BYTES) != 0) {
        return ASSET_TRANSFER_ERR_DIGEST;
    }

    /* The host resent the chunk we just accepted because our Ack was lost
     * on the way back. This is the only chunk that can legitimately repeat
     * -- chunks are strictly in-order -- so it is recognized by offset and
     * length matching the last one committed, at an offset now behind the
     * cursor. Re-advancing committed_offset here would apply those bytes
     * twice and corrupt the blob; falling through to ERR_OFFSET below would
     * make every dropped acknowledgement a fatal transfer failure. */
    if (offset == transfer->last_chunk_offset && length == transfer->last_chunk_length &&
        offset < transfer->committed_offset) {
        return ASSET_TRANSFER_DUPLICATE;
    }

    if (offset != transfer->committed_offset) {
        return ASSET_TRANSFER_ERR_OFFSET;
    }
    /* committed_offset never exceeds total_length (checked below on every
     * successful chunk), so total_length - offset cannot underflow here --
     * that is what keeps this comparison overflow-safe without computing
     * offset + length. */
    if (length == 0U || length > transfer->total_length - offset) {
        return ASSET_TRANSFER_ERR_LENGTH;
    }

    transfer->last_chunk_offset = offset;
    transfer->last_chunk_length = length;
    transfer->committed_offset = offset + length;
    return ASSET_TRANSFER_OK;
}

bool asset_transfer_is_complete(const asset_transfer_t *transfer)
{
    if (transfer == NULL) {
        return false;
    }
    return transfer->active && transfer->committed_offset == transfer->total_length;
}

void asset_transfer_abort(asset_transfer_t *transfer)
{
    if (transfer == NULL) {
        return;
    }
    memset(transfer, 0, sizeof *transfer);
}
