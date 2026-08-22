#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define ASSET_DIGEST_BYTES 32U
#define ASSET_HEADER_BYTES 32U
#define ASSET_RECORD_BYTES 64U
#define ASSET_STORE_MAGIC "DMAS"
#define ASSET_STORE_FORMAT_VERSION 1U

/* One asset may not exceed 1 MiB. Defined here, in the module both
 * asset_transfer.h and protocol_message.h already depend on, so the wire
 * bound and the transfer bound cannot drift apart. */
#define ASSET_MAX_BYTES (1024U * 1024U)

/* Erased NOR flash reads 0xFF. Commit clears bits to 0x01, which needs no
 * sector erase, so an interrupted transfer leaves an uncommitted record
 * rather than a corrupt one. */
#define ASSET_STATE_UNCOMMITTED 0xFFU
#define ASSET_STATE_COMMITTED 0x01U
#define ASSET_STATE_DEAD 0x00U

typedef enum {
    ASSET_KIND_FONT = 1,
    ASSET_KIND_ICON_FONT = 2,
    ASSET_KIND_IMAGE = 3,
} asset_kind_t;

typedef enum {
    ASSET_STORE_OK = 0,
    ASSET_STORE_ERR_ARGUMENT,
    ASSET_STORE_ERR_IO,
    ASSET_STORE_ERR_NOT_FORMATTED,
    ASSET_STORE_ERR_CORRUPT,
    ASSET_STORE_ERR_FULL,
    ASSET_STORE_ERR_NOT_FOUND,
} asset_store_result_t;

/* Flash abstraction. `write` must have NOR semantics (bits clear only);
 * `erase` restores 0xFF over whole sectors. Keeping this a vtable is what
 * lets the whole store be host-tested without ESP-IDF. */
typedef struct {
    int (*read)(void *ctx, uint32_t offset, void *out, size_t length);
    int (*write)(void *ctx, uint32_t offset, const void *data, size_t length);
    int (*erase)(void *ctx, uint32_t offset, size_t length);
    void *ctx;
} asset_flash_io_t;

typedef struct {
    uint8_t digest[ASSET_DIGEST_BYTES];
    uint32_t offset; /* relative to blob_region_offset */
    uint32_t length;
    uint8_t kind;
    uint8_t state;
} asset_record_t;

typedef struct {
    const asset_flash_io_t *io;
    uint32_t partition_size;
    uint32_t record_capacity;
    uint32_t blob_region_offset;
    uint32_t blob_region_size;
} asset_store_t;

asset_store_result_t asset_store_format(const asset_flash_io_t *io,
                                        uint32_t partition_size,
                                        uint32_t record_capacity);
asset_store_result_t asset_store_open(asset_store_t *store,
                                      const asset_flash_io_t *io,
                                      uint32_t partition_size);
void asset_store_record_encode(const asset_record_t *record, uint8_t *out);
asset_store_result_t asset_store_record_decode(const uint8_t *bytes,
                                               asset_record_t *out);
