#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "core/asset_store.h"

typedef enum {
    ASSET_TRANSFER_OK = 0,
    ASSET_TRANSFER_DUPLICATE, /* chunk already accepted; not an error */
    ASSET_TRANSFER_ERR_ARGUMENT,
    ASSET_TRANSFER_ERR_INACTIVE,
    ASSET_TRANSFER_ERR_DIGEST,
    ASSET_TRANSFER_ERR_OFFSET,
    ASSET_TRANSFER_ERR_LENGTH,
} asset_transfer_result_t;

typedef struct {
    uint8_t digest[ASSET_DIGEST_BYTES];
    uint32_t total_length;
    uint32_t committed_offset;
    uint32_t last_chunk_offset;
    uint32_t last_chunk_length;
    uint8_t kind;
    bool volatile_tier;
    bool active;
} asset_transfer_t;

asset_transfer_result_t asset_transfer_begin(asset_transfer_t *transfer,
                                             const uint8_t *digest, uint8_t kind,
                                             uint32_t total_length,
                                             bool volatile_tier);
asset_transfer_result_t asset_transfer_accept_chunk(asset_transfer_t *transfer,
                                                    const uint8_t *digest,
                                                    uint32_t offset,
                                                    uint32_t length);
bool asset_transfer_is_complete(const asset_transfer_t *transfer);
uint32_t asset_transfer_resume_offset(const asset_transfer_t *transfer);
void asset_transfer_abort(asset_transfer_t *transfer);
