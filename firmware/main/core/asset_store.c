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

/* Absolute flash offset of record `index`. Safe against overflow as long as
 * index < store->record_capacity for a store that came from asset_store_open
 * (which already rejected any record_capacity that would make this wrap) --
 * every caller below checks that bound first. */
static uint32_t record_offset(uint32_t index)
{
    return ASSET_HEADER_BYTES + index * ASSET_RECORD_BYTES;
}

/* A record slot is free (never reserved) only when it is both erased state
 * and erased digest. State alone is not enough: a record that was reserved
 * but never committed also reads ASSET_STATE_UNCOMMITTED, and its digest
 * must not be mistaken for free space. */
static bool digest_is_all_ff(const uint8_t *digest)
{
    for (size_t i = 0U; i < ASSET_DIGEST_BYTES; i++) {
        if (digest[i] != 0xFFU) {
            return false;
        }
    }
    return true;
}

/* The single source of truth for how much of the blob region is spoken for.
 * Spans committed, in-flight (uncommitted, non-free) AND dead records: a
 * dead record's bytes are reclaimable but not reclaimed until compaction
 * physically moves them (see asset_store_reserve's own comment on why dead
 * records count), and an in-flight reservation already occupies the space
 * asset_store_reserve will refuse to hand out twice. asset_store_reserve
 * and asset_store_stats both call this, so "how much reserve will grant"
 * and "how much stats reports as free" cannot drift apart -- a caller that
 * reads stats and then reserves that many bytes must succeed. */
static asset_store_result_t compute_high_water(const asset_store_t *store,
                                               uint32_t *out_high_water)
{
    uint32_t high_water = 0U;

    for (uint32_t index = 0U; index < store->record_capacity; index++) {
        uint8_t bytes[ASSET_RECORD_BYTES];
        if (store->io->read(store->io->ctx, record_offset(index), bytes, sizeof bytes) != 0) {
            return ASSET_STORE_ERR_IO;
        }
        uint8_t state = bytes[REC_OFFSET_STATE];
        bool is_free_slot = (state == ASSET_STATE_UNCOMMITTED) &&
            digest_is_all_ff(bytes + REC_OFFSET_DIGEST);
        if (is_free_slot) {
            continue;
        }
        if (state != ASSET_STATE_COMMITTED && state != ASSET_STATE_UNCOMMITTED &&
            state != ASSET_STATE_DEAD) {
            continue;
        }

        uint32_t offset = read_u32_le(bytes + REC_OFFSET_OFFSET);
        uint32_t rec_length = read_u32_le(bytes + REC_OFFSET_LENGTH);
        /* offset/length are untrusted flash content; bound them before
         * summing so a corrupt record cannot wrap high_water. */
        if (rec_length > store->blob_region_size ||
            offset > store->blob_region_size - rec_length) {
            return ASSET_STORE_ERR_CORRUPT;
        }
        uint32_t end = offset + rec_length;
        if (end > high_water) {
            high_water = end;
        }
    }

    *out_high_water = high_water;
    return ASSET_STORE_OK;
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

asset_store_result_t asset_store_find(const asset_store_t *store,
                                      const uint8_t *digest,
                                      asset_record_t *out_record,
                                      uint32_t *out_index)
{
    if (store == NULL || store->io == NULL || store->io->read == NULL || digest == NULL) {
        return ASSET_STORE_ERR_ARGUMENT;
    }

    for (uint32_t index = 0U; index < store->record_capacity; index++) {
        uint8_t bytes[ASSET_RECORD_BYTES];
        if (store->io->read(store->io->ctx, record_offset(index), bytes, sizeof bytes) != 0) {
            return ASSET_STORE_ERR_IO;
        }
        /* Only a committed entry is a usable asset -- an uncommitted
         * reservation is exactly the interrupted-transfer case this format
         * exists to make safe, and a dead one is superseded. */
        if (bytes[REC_OFFSET_STATE] != ASSET_STATE_COMMITTED) {
            continue;
        }
        if (memcmp(bytes + REC_OFFSET_DIGEST, digest, ASSET_DIGEST_BYTES) != 0) {
            continue;
        }

        asset_record_t record;
        if (asset_store_record_decode(bytes, &record) != ASSET_STORE_OK) {
            return ASSET_STORE_ERR_CORRUPT;
        }
        if (out_record != NULL) {
            *out_record = record;
        }
        if (out_index != NULL) {
            *out_index = index;
        }
        return ASSET_STORE_OK;
    }

    return ASSET_STORE_ERR_NOT_FOUND;
}

asset_store_result_t asset_store_reserve(const asset_store_t *store,
                                         const uint8_t *digest, uint8_t kind,
                                         uint32_t length, uint32_t *out_index,
                                         uint32_t *out_blob_offset)
{
    if (store == NULL || store->io == NULL || store->io->read == NULL ||
        store->io->write == NULL || digest == NULL || out_index == NULL ||
        out_blob_offset == NULL) {
        return ASSET_STORE_ERR_ARGUMENT;
    }
    if (!asset_kind_is_valid(kind)) {
        return ASSET_STORE_ERR_ARGUMENT;
    }
    if (length == 0U || length > ASSET_MAX_BYTES) {
        return ASSET_STORE_ERR_ARGUMENT;
    }

    /* High-water mark spans committed, uncommitted (in-flight), AND dead
     * records -- see compute_high_water's own comment for why dead records
     * count (their bytes are not reclaimed until compaction) and why this
     * is the same computation asset_store_stats uses for free_blob_bytes. */
    uint32_t high_water = 0U;
    asset_store_result_t hw_result = compute_high_water(store, &high_water);
    if (hw_result != ASSET_STORE_OK) {
        return hw_result;
    }

    bool free_found = false;
    uint32_t free_index = 0U;
    for (uint32_t index = 0U; index < store->record_capacity; index++) {
        uint8_t bytes[ASSET_RECORD_BYTES];
        if (store->io->read(store->io->ctx, record_offset(index), bytes, sizeof bytes) != 0) {
            return ASSET_STORE_ERR_IO;
        }
        if (bytes[REC_OFFSET_STATE] == ASSET_STATE_UNCOMMITTED &&
            digest_is_all_ff(bytes + REC_OFFSET_DIGEST)) {
            free_found = true;
            free_index = index;
            break;
        }
    }

    if (!free_found) {
        return ASSET_STORE_ERR_FULL;
    }
    if (length > store->blob_region_size || high_water > store->blob_region_size - length) {
        return ASSET_STORE_ERR_FULL;
    }

    asset_record_t record;
    memcpy(record.digest, digest, ASSET_DIGEST_BYTES);
    record.offset = high_water;
    record.length = length;
    record.kind = kind;
    record.state = ASSET_STATE_UNCOMMITTED; /* left erased; commit clears it later */

    uint8_t bytes[ASSET_RECORD_BYTES];
    asset_store_record_encode(&record, bytes);
    /* The slot is currently fully erased (0xFF), so writing the whole
     * encoded record -- including its erased state byte and erased
     * reserved tail -- is a pure bit-clear, matching every other flash
     * write in this module. */
    if (store->io->write(store->io->ctx, record_offset(free_index), bytes, sizeof bytes) != 0) {
        return ASSET_STORE_ERR_IO;
    }

    *out_index = free_index;
    *out_blob_offset = high_water;
    return ASSET_STORE_OK;
}

asset_store_result_t asset_store_commit(const asset_store_t *store, uint32_t index)
{
    if (store == NULL || store->io == NULL || store->io->read == NULL ||
        store->io->write == NULL) {
        return ASSET_STORE_ERR_ARGUMENT;
    }
    if (index >= store->record_capacity) {
        return ASSET_STORE_ERR_ARGUMENT;
    }

    /* Defence in depth against a caller (protocol_task's AssetCommit
     * handler) whose in-RAM transfer bookkeeping has desynced from this
     * slot's actual on-flash state -- e.g. a compaction that ran between
     * reserve() and commit() and, because the record was still
     * UNCOMMITTED and therefore excluded from the kept set, erased its
     * digest/offset/length/kind back to 0xFF without the caller's
     * knowledge. Writing COMMITTED over that would silently mint a
     * "valid" record with a length of 0xFFFFFFFF, which every future
     * asset_store_reserve()/asset_store_stats() call would then trip over
     * as corruption -- turning one desync into a bricked store. Refuse
     * instead: the slot must decode cleanly, be in the UNCOMMITTED state
     * a reservation leaves it in, and carry a real (not all-erased)
     * digest. */
    uint8_t bytes[ASSET_RECORD_BYTES];
    if (store->io->read(store->io->ctx, record_offset(index), bytes, sizeof bytes) != 0) {
        return ASSET_STORE_ERR_IO;
    }
    asset_record_t record;
    if (asset_store_record_decode(bytes, &record) != ASSET_STORE_OK) {
        return ASSET_STORE_ERR_CORRUPT;
    }
    if (record.state != ASSET_STATE_UNCOMMITTED ||
        digest_is_all_ff(bytes + REC_OFFSET_DIGEST)) {
        return ASSET_STORE_ERR_CORRUPT;
    }

    uint8_t state = ASSET_STATE_COMMITTED;
    if (store->io->write(store->io->ctx, record_offset(index) + REC_OFFSET_STATE, &state,
                         1U) != 0) {
        return ASSET_STORE_ERR_IO;
    }
    return ASSET_STORE_OK;
}

asset_store_result_t asset_store_mark_dead(const asset_store_t *store, uint32_t index)
{
    if (store == NULL || store->io == NULL || store->io->write == NULL) {
        return ASSET_STORE_ERR_ARGUMENT;
    }
    if (index >= store->record_capacity) {
        return ASSET_STORE_ERR_ARGUMENT;
    }

    uint8_t state = ASSET_STATE_DEAD;
    if (store->io->write(store->io->ctx, record_offset(index) + REC_OFFSET_STATE, &state,
                         1U) != 0) {
        return ASSET_STORE_ERR_IO;
    }
    return ASSET_STORE_OK;
}

asset_store_result_t asset_store_reclaim_boot_orphans(const asset_store_t *store,
                                                       uint32_t *out_reclaimed_count)
{
    if (store == NULL || store->io == NULL || store->io->read == NULL ||
        store->io->write == NULL) {
        return ASSET_STORE_ERR_ARGUMENT;
    }

    uint32_t reclaimed_count = 0U;
    for (uint32_t index = 0U; index < store->record_capacity; index++) {
        uint8_t bytes[ASSET_RECORD_BYTES];
        if (store->io->read(store->io->ctx, record_offset(index), bytes, sizeof bytes) != 0) {
            return ASSET_STORE_ERR_IO;
        }
        if (bytes[REC_OFFSET_STATE] != ASSET_STATE_UNCOMMITTED) {
            continue;
        }
        if (digest_is_all_ff(bytes + REC_OFFSET_DIGEST)) {
            continue; /* never written -- free space, not an orphan */
        }

        asset_store_result_t result = asset_store_mark_dead(store, index);
        if (result != ASSET_STORE_OK) {
            return result;
        }
        reclaimed_count++;
    }

    if (out_reclaimed_count != NULL) {
        *out_reclaimed_count = reclaimed_count;
    }
    return ASSET_STORE_OK;
}

asset_store_result_t asset_store_stats(const asset_store_t *store,
                                       asset_store_stats_t *out_stats)
{
    if (store == NULL || store->io == NULL || store->io->read == NULL || out_stats == NULL) {
        return ASSET_STORE_ERR_ARGUMENT;
    }

    /* free_blob_bytes is derived from the exact same high-water computation
     * asset_store_reserve uses to place the next blob, not from
     * (region - used - reclaimable). Deriving it from the sums would fold
     * an in-flight (uncommitted) reservation's bytes into "free", since
     * uncommitted records are neither used nor reclaimable -- which is
     * precisely how a caller could read free space that reserve then
     * refuses to grant. used_blob_bytes/reclaimable_blob_bytes deliberately
     * stay committed-only/dead-only, so used + reclaimable + free need not
     * sum to blob_region_size while a transfer is in flight; the gap is
     * exactly the uncommitted bytes, and that is intentional. */
    uint32_t high_water = 0U;
    asset_store_result_t hw_result = compute_high_water(store, &high_water);
    if (hw_result != ASSET_STORE_OK) {
        return hw_result;
    }

    uint32_t committed_count = 0U;
    uint32_t used_blob_bytes = 0U;
    uint32_t reclaimable_blob_bytes = 0U;
    /* Tracks used + reclaimable, kept <= blob_region_size by construction
     * (checked before every add below), so none of these running sums can
     * ever wrap a uint32_t even if flash content is corrupt. */
    uint32_t accounted_bytes = 0U;

    for (uint32_t index = 0U; index < store->record_capacity; index++) {
        uint8_t bytes[ASSET_RECORD_BYTES];
        if (store->io->read(store->io->ctx, record_offset(index), bytes, sizeof bytes) != 0) {
            return ASSET_STORE_ERR_IO;
        }
        uint8_t state = bytes[REC_OFFSET_STATE];
        if (state != ASSET_STATE_COMMITTED && state != ASSET_STATE_DEAD) {
            continue;
        }

        uint32_t length = read_u32_le(bytes + REC_OFFSET_LENGTH);
        if (length > store->blob_region_size - accounted_bytes) {
            return ASSET_STORE_ERR_CORRUPT;
        }
        accounted_bytes += length;

        if (state == ASSET_STATE_COMMITTED) {
            committed_count++;
            used_blob_bytes += length;
        } else {
            reclaimable_blob_bytes += length;
        }
    }

    out_stats->committed_count = committed_count;
    out_stats->used_blob_bytes = used_blob_bytes;
    out_stats->reclaimable_blob_bytes = reclaimable_blob_bytes;
    /* high_water <= blob_region_size always holds: compute_high_water only
     * ever grows high_water to a record's `end`, and every `end` is
     * bounds-checked against blob_region_size before being considered. */
    out_stats->free_blob_bytes = store->blob_region_size - high_water;
    return ASSET_STORE_OK;
}

asset_store_result_t asset_store_plan_compaction(const asset_store_t *store,
                                                 const uint8_t *const *keep,
                                                 size_t keep_count,
                                                 asset_move_t *moves,
                                                 size_t moves_capacity,
                                                 size_t *out_move_count)
{
    if (store == NULL || store->io == NULL || store->io->read == NULL ||
        out_move_count == NULL) {
        return ASSET_STORE_ERR_ARGUMENT;
    }
    if (keep_count > 0U && keep == NULL) {
        return ASSET_STORE_ERR_ARGUMENT;
    }
    if (moves_capacity > 0U && moves == NULL) {
        return ASSET_STORE_ERR_ARGUMENT;
    }

    size_t move_count = 0U;
    uint32_t to_offset = 0U;
    bool have_last = false;
    uint32_t last_offset = 0U;
    uint32_t last_index = 0U;

    /* There is no offset index on flash, so committed records are visited
     * in ascending blob offset by repeatedly selecting the smallest offset
     * strictly past the previously emitted one (ties broken by record
     * index, which only matters for already-corrupt duplicate offsets and
     * exists to guarantee this loop still terminates). This costs
     * O(record_capacity^2), which is fine: asset counts on this device are
     * small and this only runs as a maintenance step, never per frame. */
    for (;;) {
        bool have_next = false;
        uint32_t next_offset = 0U;
        uint32_t next_index = 0U;
        uint8_t next_bytes[ASSET_RECORD_BYTES];

        for (uint32_t index = 0U; index < store->record_capacity; index++) {
            uint8_t bytes[ASSET_RECORD_BYTES];
            if (store->io->read(store->io->ctx, record_offset(index), bytes, sizeof bytes) != 0) {
                return ASSET_STORE_ERR_IO;
            }
            if (bytes[REC_OFFSET_STATE] != ASSET_STATE_COMMITTED) {
                continue;
            }
            uint32_t offset = read_u32_le(bytes + REC_OFFSET_OFFSET);

            bool after_last = !have_last || offset > last_offset ||
                (offset == last_offset && index > last_index);
            if (!after_last) {
                continue;
            }
            bool better = !have_next || offset < next_offset ||
                (offset == next_offset && index < next_index);
            if (better) {
                have_next = true;
                next_offset = offset;
                next_index = index;
                memcpy(next_bytes, bytes, sizeof bytes);
            }
        }

        if (!have_next) {
            break; /* no committed record left past the previous pick */
        }

        have_last = true;
        last_offset = next_offset;
        last_index = next_index;

        bool keep_this = false;
        for (size_t k = 0U; k < keep_count; k++) {
            if (memcmp(next_bytes + REC_OFFSET_DIGEST, keep[k], ASSET_DIGEST_BYTES) == 0) {
                keep_this = true;
                break;
            }
        }
        if (!keep_this) {
            /* Not kept: dispatch_asset_release's own mark-dead pass (which
             * runs before this plan is built) already flips a not-kept
             * committed record's state to DEAD -- using
             * asset_store_record_decode(), which validates kind/state but
             * not offset/length -- so a corrupt, not-kept record should
             * never even reach here still COMMITTED. This branch stays
             * defensive of that anyway: it drops the record either way, so
             * it never needs to look at its (possibly corrupt) offset/length. */
            continue;
        }

        uint32_t length = read_u32_le(next_bytes + REC_OFFSET_LENGTH);
        /* offset/length are untrusted flash content; bound them the same
         * way asset_store_reserve and asset_store_stats do. Unlike those, a
         * failure here must not fail the whole plan: a digest the host still
         * wants kept can name a record whose offset/length were corrupted on
         * flash independently of its digest/state (this format has no way to
         * detect that beyond exactly this bounds check). Refusing to plan
         * *any* compaction over one unusable kept record would wedge
         * AssetRelease -- and therefore all GC -- forever, since the host
         * has no way to stop asking for a digest it still believes it sent.
         * Skip it instead: it is unusable regardless, and a record excluded
         * from the returned plan simply does not survive compaction (only
         * moves[] entries are re-committed). This cannot drop a genuinely
         * valid survivor -- last_offset/last_index already advanced above so
         * the scan still makes forward progress, and to_offset (this
         * record's would-be destination) is left untouched, so every later
         * kept record still packs from the correct base. */
        if (length > store->blob_region_size ||
            next_offset > store->blob_region_size - length) {
            continue;
        }

        /* Never truncate: a caller-supplied buffer too small to hold the
         * whole plan must fail outright, or the caller could apply a
         * partial plan that silently deletes survivors past the cutoff. */
        if (move_count >= moves_capacity) {
            return ASSET_STORE_ERR_FULL;
        }
        /* Same reasoning as the bounds check above, for the one corruption
         * shape it cannot catch on its own: an individually in-range record
         * that still cannot pack without overlapping an already-accepted
         * survivor. On flash this format itself ever produces, that cannot
         * happen -- asset_store_reserve's high-water accounting guarantees
         * committed ranges never overlap -- so reaching this is corruption,
         * not a real layout. Skip for the same reason as above; to_offset is
         * untouched here too, so a later valid survivor is unaffected. */
        if (length > store->blob_region_size - to_offset) {
            continue;
        }

        moves[move_count].from_offset = next_offset;
        moves[move_count].to_offset = to_offset;
        moves[move_count].length = length;
        moves[move_count].record_index = next_index;
        move_count++;
        to_offset += length;
    }

    *out_move_count = move_count;
    return ASSET_STORE_OK;
}
