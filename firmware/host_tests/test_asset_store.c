#include <assert.h>
#include <string.h>

#include "core/asset_store.h"

/* RAM-backed fake flash: NOR semantics -- writes may only clear bits, and
 * erase restores 0xFF. Modelling that faithfully is the point; a plain memcpy
 * fake would hide the commit-bit trick this format depends on. */
#define FAKE_SIZE (64U * 1024U)
static uint8_t g_flash[FAKE_SIZE];

static int fake_read(void *ctx, uint32_t offset, void *out, size_t length)
{
    (void)ctx;
    if ((size_t)offset + length > FAKE_SIZE) return -1;
    memcpy(out, g_flash + offset, length);
    return 0;
}

static int fake_write(void *ctx, uint32_t offset, const void *data, size_t length)
{
    (void)ctx;
    if ((size_t)offset + length > FAKE_SIZE) return -1;
    const uint8_t *src = data;
    for (size_t i = 0; i < length; i++) {
        g_flash[offset + i] &= src[i]; /* NOR: bits only go 1 -> 0 */
    }
    return 0;
}

static int fake_erase(void *ctx, uint32_t offset, size_t length)
{
    (void)ctx;
    if ((size_t)offset + length > FAKE_SIZE) return -1;
    memset(g_flash + offset, 0xFF, length);
    return 0;
}

static asset_flash_io_t fake_io(void)
{
    asset_flash_io_t io = {
        .read = fake_read, .write = fake_write, .erase = fake_erase, .ctx = NULL,
    };
    return io;
}

static void test_format_then_open_roundtrips(void)
{
    memset(g_flash, 0x00, sizeof g_flash); /* deliberately not erased */
    asset_flash_io_t io = fake_io();
    asset_store_t store;

    assert(asset_store_format(&io, FAKE_SIZE, 64U) == ASSET_STORE_OK);
    assert(asset_store_open(&store, &io, FAKE_SIZE) == ASSET_STORE_OK);
    assert(store.record_capacity == 64U);
    assert(store.blob_region_size > 0U);
}

static void test_open_rejects_bad_magic(void)
{
    memset(g_flash, 0xFF, sizeof g_flash);
    asset_flash_io_t io = fake_io();
    asset_store_t store;

    assert(asset_store_open(&store, &io, FAKE_SIZE) == ASSET_STORE_ERR_NOT_FORMATTED);
}

static void test_record_encode_decode_roundtrips(void)
{
    asset_record_t in = { .offset = 4096U, .length = 1234U, .kind = ASSET_KIND_FONT,
                          .state = ASSET_STATE_COMMITTED };
    memset(in.digest, 0xAB, ASSET_DIGEST_BYTES);
    uint8_t bytes[ASSET_RECORD_BYTES];
    asset_record_t out;

    asset_store_record_encode(&in, bytes);
    assert(asset_store_record_decode(bytes, &out) == ASSET_STORE_OK);
    assert(out.offset == in.offset);
    assert(out.length == in.length);
    assert(out.kind == in.kind);
    assert(out.state == in.state);
    assert(memcmp(out.digest, in.digest, ASSET_DIGEST_BYTES) == 0);
}

static void test_record_decode_rejects_unknown_kind(void)
{
    uint8_t bytes[ASSET_RECORD_BYTES];
    memset(bytes, 0, sizeof bytes);
    bytes[ASSET_DIGEST_BYTES + 8U] = 0x7FU; /* kind byte, not a defined kind */
    bytes[ASSET_DIGEST_BYTES + 9U] = ASSET_STATE_COMMITTED;
    asset_record_t out;

    assert(asset_store_record_decode(bytes, &out) == ASSET_STORE_ERR_CORRUPT);
}

/* record_capacity is read straight off flash by asset_store_open, with no
 * upper bound of its own -- only the multiplication against
 * ASSET_RECORD_BYTES catches an out-of-range value, and that multiplication
 * silently wraps in 32 bits if it isn't bounded first. 0x04000001 * 64 mod
 * 2^32 == 64, which would otherwise slip past the "does the record array
 * fit before the blob region" check with a plausible-looking
 * blob_region_offset. Hand-craft the header directly: nothing that goes
 * through asset_store_format can produce this record_capacity, since the
 * fix rejects it there too. */
static void test_open_rejects_overflowing_record_capacity(void)
{
    memset(g_flash, 0xFF, sizeof g_flash);
    memcpy(g_flash + 0U, ASSET_STORE_MAGIC, 4U);
    g_flash[4] = 1U; /* format version LE u32 = 1 */
    g_flash[5] = 0U;
    g_flash[6] = 0U;
    g_flash[7] = 0U;
    g_flash[8] = 0x01U; /* record_capacity LE u32 = 0x04000001 */
    g_flash[9] = 0x00U;
    g_flash[10] = 0x00U;
    g_flash[11] = 0x04U;
    g_flash[12] = 0x00U; /* blob_region_offset LE u32 = 4096 */
    g_flash[13] = 0x10U;
    g_flash[14] = 0x00U;
    g_flash[15] = 0x00U;
    g_flash[16] = 0U; /* blob_region_size LE u32 = 0 */
    g_flash[17] = 0U;
    g_flash[18] = 0U;
    g_flash[19] = 0U;

    asset_flash_io_t io = fake_io();
    asset_store_t store;

    assert(asset_store_open(&store, &io, FAKE_SIZE) == ASSET_STORE_ERR_CORRUPT);
}

static asset_store_t formatted_store(asset_flash_io_t *io)
{
    memset(g_flash, 0x00, sizeof g_flash);
    *io = fake_io();
    asset_store_t store;
    assert(asset_store_format(io, FAKE_SIZE, 64U) == ASSET_STORE_OK);
    assert(asset_store_open(&store, io, FAKE_SIZE) == ASSET_STORE_OK);
    return store;
}

static void digest_of(uint8_t seed, uint8_t *out)
{
    memset(out, seed, ASSET_DIGEST_BYTES);
}

static void test_uncommitted_reservation_is_not_findable(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x11, digest);
    uint32_t index = 0U, blob = 0U;

    assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT, 100U, &index, &blob)
           == ASSET_STORE_OK);
    /* This is the crash-safety property: a transfer interrupted before commit
     * must never surface as a usable asset. */
    assert(asset_store_find(&store, digest, NULL, NULL) == ASSET_STORE_ERR_NOT_FOUND);

    assert(asset_store_commit(&store, index) == ASSET_STORE_OK);
    assert(asset_store_find(&store, digest, NULL, NULL) == ASSET_STORE_OK);
}

static void test_reserve_rejects_blob_overflow(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x22, digest);
    uint32_t index = 0U, blob = 0U;

    assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT,
                               store.blob_region_size + 1U, &index, &blob)
           == ASSET_STORE_ERR_FULL);
}

static void test_duplicate_digest_reports_already_present(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x33, digest);
    uint32_t index = 0U, blob = 0U;

    assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT, 64U, &index, &blob)
           == ASSET_STORE_OK);
    assert(asset_store_commit(&store, index) == ASSET_STORE_OK);
    /* Content addressing means a second push of the same bytes is a no-op.
     * This is the whole inventory protocol -- see plan Task 5. */
    assert(asset_store_find(&store, digest, NULL, NULL) == ASSET_STORE_OK);
}

static void test_compaction_plan_drops_unreferenced_and_packs(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t keep_digest[ASSET_DIGEST_BYTES];
    uint8_t drop_digest[ASSET_DIGEST_BYTES];
    digest_of(0xAA, keep_digest);
    digest_of(0xBB, drop_digest);
    uint32_t index = 0U, blob = 0U;

    assert(asset_store_reserve(&store, drop_digest, ASSET_KIND_FONT, 4096U,
                               &index, &blob) == ASSET_STORE_OK);
    assert(asset_store_commit(&store, index) == ASSET_STORE_OK);
    assert(asset_store_reserve(&store, keep_digest, ASSET_KIND_FONT, 512U,
                               &index, &blob) == ASSET_STORE_OK);
    assert(asset_store_commit(&store, index) == ASSET_STORE_OK);

    const uint8_t *keep[1] = { keep_digest };
    asset_move_t moves[8];
    size_t move_count = 0U;
    assert(asset_store_plan_compaction(&store, keep, 1U, moves, 8U, &move_count)
           == ASSET_STORE_OK);

    assert(move_count == 1U);
    assert(moves[0].length == 512U);
    assert(moves[0].to_offset == 0U); /* survivor packs down to the region start */
    assert(moves[0].from_offset == 4096U);
}

static void test_compaction_plan_reports_capacity_exhaustion(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    const uint8_t *keep[2];
    uint8_t d0[ASSET_DIGEST_BYTES], d1[ASSET_DIGEST_BYTES];
    uint32_t index = 0U, blob = 0U;

    digest_of(0xC0, d0);
    digest_of(0xC1, d1);
    keep[0] = d0;
    keep[1] = d1;
    for (uint8_t i = 0; i < 2; i++) {
        digest_of((uint8_t)(0xC0 + i), digest);
        assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT, 256U,
                                   &index, &blob) == ASSET_STORE_OK);
        assert(asset_store_commit(&store, index) == ASSET_STORE_OK);
    }

    asset_move_t moves[1];
    size_t move_count = 0U;
    /* A caller-supplied buffer that is too small must be an error, never a
     * silent truncation that would delete surviving assets. */
    assert(asset_store_plan_compaction(&store, keep, 2U, moves, 1U, &move_count)
           == ASSET_STORE_ERR_FULL);
}

int main(void)
{
    test_format_then_open_roundtrips();
    test_open_rejects_bad_magic();
    test_record_encode_decode_roundtrips();
    test_record_decode_rejects_unknown_kind();
    test_open_rejects_overflowing_record_capacity();
    test_uncommitted_reservation_is_not_findable();
    test_reserve_rejects_blob_overflow();
    test_duplicate_digest_reports_already_present();
    test_compaction_plan_drops_unreferenced_and_packs();
    test_compaction_plan_reports_capacity_exhaustion();
    return 0;
}
