#include "asset_store.h"

#include <string.h>

/* Header field byte offsets, all little-endian. Kept as named offsets
 * (rather than a packed struct) for the same reason net_ring.c and
 * dev_capture.c avoid struct-to-bytes casts: struct layout is
 * compiler/ABI-dependent and this format has to be readable by whatever
 * wrote it, byte for byte, on flash. */
#define HDR_OFFSET_MAGIC 0U
#define HDR_OFFSET_FORMAT_VERSION 4U
#define HDR_OFFSET_RECORD_CAPACITY 8U
#define HDR_OFFSET_BLOB_REGION_OFFSET 12U
#define HDR_OFFSET_BLOB_REGION_SIZE 16U

/* Blob region placement is sector-aligned so the blob region can be erased
 * in whole sectors without touching the record array that precedes it. */
#define ASSET_STORE_SECTOR_SIZE 4096U

/* Record field byte offsets within a single ASSET_RECORD_BYTES record. */
#define REC_OFFSET_DIGEST 0U
#define REC_OFFSET_OFFSET ASSET_DIGEST_BYTES
#define REC_OFFSET_LENGTH (ASSET_DIGEST_BYTES + 4U)
#define REC_OFFSET_KIND (ASSET_DIGEST_BYTES + 8U)
#define REC_OFFSET_STATE (ASSET_DIGEST_BYTES + 9U)

static void write_u32_le(uint8_t *bytes, uint32_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8U);
    bytes[2] = (uint8_t)(value >> 16U);
    bytes[3] = (uint8_t)(value >> 24U);
}

static uint32_t read_u32_le(const uint8_t *bytes)
{
    return (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8U) |
           ((uint32_t)bytes[2] << 16U) | ((uint32_t)bytes[3] << 24U);
}

static uint32_t round_up_to_sector(uint32_t value)
{
    uint32_t remainder = value % ASSET_STORE_SECTOR_SIZE;
    if (remainder == 0U) {
        return value;
    }
    return value + (ASSET_STORE_SECTOR_SIZE - remainder);
}

/* record_capacity * ASSET_RECORD_BYTES is computed as uint32_t * uint32_t
 * further down; done unchecked, a large enough record_capacity wraps
 * silently instead of failing the size check that follows it. This bounds
 * record_capacity to whatever the multiplication (plus the header) can
 * hold in 32 bits, so the multiplication below it can never overflow. */
static bool record_capacity_fits(uint32_t record_capacity)
{
    return record_capacity <= (UINT32_MAX - ASSET_HEADER_BYTES) / ASSET_RECORD_BYTES;
}

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

static bool asset_state_is_valid(uint8_t state)
{
    switch (state) {
    case ASSET_STATE_UNCOMMITTED:
    case ASSET_STATE_COMMITTED:
    case ASSET_STATE_DEAD:
        return true;
    default:
        return false;
    }
}

asset_store_result_t asset_store_format(const asset_flash_io_t *io,
                                        uint32_t partition_size,
                                        uint32_t record_capacity)
{
    if (io == NULL || io->read == NULL || io->write == NULL || io->erase == NULL) {
        return ASSET_STORE_ERR_ARGUMENT;
    }
    if (!record_capacity_fits(record_capacity)) {
        return ASSET_STORE_ERR_ARGUMENT;
    }

    uint32_t record_array_size = record_capacity * ASSET_RECORD_BYTES;
    uint32_t blob_region_offset =
        round_up_to_sector(ASSET_HEADER_BYTES + record_array_size);
    if (blob_region_offset > partition_size) {
        return ASSET_STORE_ERR_ARGUMENT;
    }
    uint32_t blob_region_size = partition_size - blob_region_offset;

    if (io->erase(io->ctx, 0U, partition_size) != 0) {
        return ASSET_STORE_ERR_IO;
    }

    /* Every header field is written explicitly; the reserved tail bytes
     * stay erased (0xFF) rather than zeroed, matching the "erased means
     * absent" convention the record state byte also relies on. */
    uint8_t header[ASSET_HEADER_BYTES];
    memset(header, 0xFF, sizeof header);
    memcpy(header + HDR_OFFSET_MAGIC, ASSET_STORE_MAGIC, 4U);
    write_u32_le(header + HDR_OFFSET_FORMAT_VERSION, ASSET_STORE_FORMAT_VERSION);
    write_u32_le(header + HDR_OFFSET_RECORD_CAPACITY, record_capacity);
    write_u32_le(header + HDR_OFFSET_BLOB_REGION_OFFSET, blob_region_offset);
    write_u32_le(header + HDR_OFFSET_BLOB_REGION_SIZE, blob_region_size);

    if (io->write(io->ctx, 0U, header, sizeof header) != 0) {
        return ASSET_STORE_ERR_IO;
    }

    return ASSET_STORE_OK;
}

asset_store_result_t asset_store_open(asset_store_t *store,
                                      const asset_flash_io_t *io,
                                      uint32_t partition_size)
{
    if (store == NULL || io == NULL || io->read == NULL) {
        return ASSET_STORE_ERR_ARGUMENT;
    }

    uint8_t header[ASSET_HEADER_BYTES];
    if (io->read(io->ctx, 0U, header, sizeof header) != 0) {
        return ASSET_STORE_ERR_IO;
    }

    if (memcmp(header + HDR_OFFSET_MAGIC, ASSET_STORE_MAGIC, 4U) != 0) {
        return ASSET_STORE_ERR_NOT_FORMATTED;
    }
    uint32_t format_version = read_u32_le(header + HDR_OFFSET_FORMAT_VERSION);
    if (format_version != ASSET_STORE_FORMAT_VERSION) {
        return ASSET_STORE_ERR_NOT_FORMATTED;
    }

    uint32_t record_capacity = read_u32_le(header + HDR_OFFSET_RECORD_CAPACITY);
    uint32_t blob_region_offset = read_u32_le(header + HDR_OFFSET_BLOB_REGION_OFFSET);
    uint32_t blob_region_size = read_u32_le(header + HDR_OFFSET_BLOB_REGION_SIZE);

    /* record_capacity comes straight off flash here, so an out-of-range
     * value (corrupt header, or a hostile one) must be rejected before the
     * multiplication below it, not after -- otherwise it wraps and this
     * whole check can be defeated by choosing a capacity that overflows
     * back down to something small. */
    if (!record_capacity_fits(record_capacity)) {
        return ASSET_STORE_ERR_CORRUPT;
    }

    /* blob_region_offset must fit the record array it follows, and the
     * whole blob region must fit inside the partition -- both are cheap
     * to check and both catch a header that decoded but does not describe
     * this partition. */
    if (blob_region_offset < ASSET_HEADER_BYTES + record_capacity * ASSET_RECORD_BYTES) {
        return ASSET_STORE_ERR_CORRUPT;
    }
    if (blob_region_offset > partition_size ||
        blob_region_size > partition_size - blob_region_offset) {
        return ASSET_STORE_ERR_CORRUPT;
    }

    store->io = io;
    store->partition_size = partition_size;
    store->record_capacity = record_capacity;
    store->blob_region_offset = blob_region_offset;
    store->blob_region_size = blob_region_size;
    return ASSET_STORE_OK;
}

void asset_store_record_encode(const asset_record_t *record, uint8_t *out)
{
    memcpy(out + REC_OFFSET_DIGEST, record->digest, ASSET_DIGEST_BYTES);
    write_u32_le(out + REC_OFFSET_OFFSET, record->offset);
    write_u32_le(out + REC_OFFSET_LENGTH, record->length);
    out[REC_OFFSET_KIND] = record->kind;
    out[REC_OFFSET_STATE] = record->state;
    /* Bytes past the fixed fields are reserved for future record metadata.
     * Leaving them erased (0xFF), rather than zeroed, means a later field
     * added here can still be committed with a bits-only-clear write. */
    memset(out + REC_OFFSET_STATE + 1U, 0xFF,
           ASSET_RECORD_BYTES - (REC_OFFSET_STATE + 1U));
}

asset_store_result_t asset_store_record_decode(const uint8_t *bytes,
                                               asset_record_t *out)
{
    uint8_t kind = bytes[REC_OFFSET_KIND];
    uint8_t state = bytes[REC_OFFSET_STATE];
    if (!asset_kind_is_valid(kind) || !asset_state_is_valid(state)) {
        return ASSET_STORE_ERR_CORRUPT;
    }

    memcpy(out->digest, bytes + REC_OFFSET_DIGEST, ASSET_DIGEST_BYTES);
    out->offset = read_u32_le(bytes + REC_OFFSET_OFFSET);
    out->length = read_u32_le(bytes + REC_OFFSET_LENGTH);
    out->kind = kind;
    out->state = state;
    return ASSET_STORE_OK;
}
