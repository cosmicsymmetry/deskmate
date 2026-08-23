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

static void test_committed_record_is_findable_by_digest(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x33, digest);
    uint32_t index = 0U, blob = 0U;

    assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT, 64U, &index, &blob)
           == ASSET_STORE_OK);
    assert(asset_store_commit(&store, index) == ASSET_STORE_OK);
    /* asset_store_reserve does not deduplicate by digest -- that is a
     * deliberate design choice, not an oversight. Content-addressed
     * deduplication is Task 5's job, via the AssetBegin handler reporting
     * already_present after calling asset_store_find itself. This test
     * only pins that a committed record is findable by its digest. */
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

static void test_reserve_does_not_reuse_dead_but_uncompacted_space(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t dead_digest[ASSET_DIGEST_BYTES];
    uint8_t new_digest[ASSET_DIGEST_BYTES];
    digest_of(0xD0, dead_digest);
    digest_of(0xD1, new_digest);
    uint32_t index = 0U, blob = 0U;

    /* The store is append-only: mark_dead only clears a state byte, it
     * never erases the blob bytes it once claimed. Only compaction (which
     * this test never runs) reclaims that space. So a reservation made
     * after a mark_dead, with no compaction in between, must still land at
     * or past the dead record's end -- landing inside it would let a NOR
     * bit-clear write silently corrupt the new asset with the old one's
     * leftover bits. */
    assert(asset_store_reserve(&store, dead_digest, ASSET_KIND_FONT, 4096U,
                               &index, &blob) == ASSET_STORE_OK);
    assert(asset_store_commit(&store, index) == ASSET_STORE_OK);
    assert(asset_store_mark_dead(&store, index) == ASSET_STORE_OK);

    uint32_t new_index = 0U, new_blob = 0U;
    assert(asset_store_reserve(&store, new_digest, ASSET_KIND_FONT, 512U,
                               &new_index, &new_blob) == ASSET_STORE_OK);
    assert(new_blob >= 4096U);
}

static void test_stats_free_blob_bytes_matches_what_reserve_will_grant(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest_a[ASSET_DIGEST_BYTES];
    uint8_t digest_b[ASSET_DIGEST_BYTES];
    digest_of(0xE0, digest_a);
    digest_of(0xE1, digest_b);
    uint32_t index = 0U, blob = 0U;

    /* Reserve almost the whole region, leaving a small remainder, but do
     * NOT commit it -- an in-flight reservation (a transfer stalled or
     * dropped mid-font) already occupies that space. If free_blob_bytes
     * were derived from (region - used - reclaimable) instead of the same
     * high-water computation reserve uses, it would report the whole
     * region as free here, and the second reserve below -- for exactly
     * the amount stats just claimed was free -- would fail with
     * ASSET_STORE_ERR_FULL. */
    uint32_t reserved_length = store.blob_region_size - 100U;
    assert(asset_store_reserve(&store, digest_a, ASSET_KIND_FONT, reserved_length,
                               &index, &blob) == ASSET_STORE_OK);

    asset_store_stats_t stats;
    assert(asset_store_stats(&store, &stats) == ASSET_STORE_OK);
    assert(stats.free_blob_bytes == 100U);

    uint32_t new_index = 0U, new_blob = 0U;
    assert(asset_store_reserve(&store, digest_b, ASSET_KIND_FONT,
                               stats.free_blob_bytes, &new_index, &new_blob)
           == ASSET_STORE_OK);
}

/* Finding 1 (whole-branch review, 2026-08-22): an AssetBegin/AssetChunk
 * transfer interrupted before AssetCommit -- link drop, host crash, power
 * loss -- leaves its reservation UNCOMMITTED with a real digest. No in-RAM
 * asset_transfer_t survives a reboot, so that record can never be completed;
 * asset_flash_init() must reclaim it at boot (asset_store_reclaim_boot_orphans),
 * or it is counted as spoken-for by every future asset_store_reserve() call
 * forever, with no path back to free space short of a reflash. */
static void test_boot_reclaim_marks_orphaned_uncommitted_dead(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x55, digest);
    uint32_t index = 0U, blob = 0U;

    /* Reserve and never commit -- the interrupted-transfer case. */
    assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT, 4096U, &index,
                               &blob) == ASSET_STORE_OK);

    asset_store_stats_t stats_before;
    assert(asset_store_stats(&store, &stats_before) == ASSET_STORE_OK);
    /* An in-flight (uncommitted) reservation is neither used nor
     * reclaimable by design (see asset_store_stats's own comment) -- that
     * is exactly the invisible, unrecoverable state this finding is about. */
    assert(stats_before.reclaimable_blob_bytes == 0U);

    uint32_t reclaimed_count = 0U;
    assert(asset_store_reclaim_boot_orphans(&store, &reclaimed_count) ==
          ASSET_STORE_OK);
    assert(reclaimed_count == 1U);

    asset_store_stats_t stats_after;
    assert(asset_store_stats(&store, &stats_after) == ASSET_STORE_OK);
    /* The orphan is now an ordinary dead record: reclaimable by the next
     * compaction instead of permanently unaccounted-for. */
    assert(stats_after.reclaimable_blob_bytes == 4096U);
    assert(stats_after.used_blob_bytes == 0U);

    /* Calling it again with nothing left to reclaim is a no-op. */
    uint32_t reclaimed_again = 0U;
    assert(asset_store_reclaim_boot_orphans(&store, &reclaimed_again) ==
          ASSET_STORE_OK);
    assert(reclaimed_again == 0U);
}

/* A never-written slot (state UNCOMMITTED, digest all-0xFF -- free space)
 * and a committed record must both survive the boot reclaim untouched: only
 * a real abandoned reservation is an orphan. */
static void test_boot_reclaim_leaves_free_and_committed_slots_alone(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x66, digest);
    uint32_t index = 0U, blob = 0U;

    assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT, 128U, &index,
                               &blob) == ASSET_STORE_OK);
    assert(asset_store_commit(&store, index) == ASSET_STORE_OK);

    uint32_t reclaimed_count = 0U;
    assert(asset_store_reclaim_boot_orphans(&store, &reclaimed_count) ==
          ASSET_STORE_OK);
    assert(reclaimed_count == 0U);

    /* Committed record is untouched -- still findable. */
    assert(asset_store_find(&store, digest, NULL, NULL) == ASSET_STORE_OK);

    asset_store_stats_t stats;
    assert(asset_store_stats(&store, &stats) == ASSET_STORE_OK);
    assert(stats.used_blob_bytes == 128U);
    assert(stats.reclaimable_blob_bytes == 0U);
    /* The other 63 never-written slots stayed free, not orphaned. */
    assert(stats.committed_count == 1U);
}

/* Finding 3 (whole-branch review, 2026-08-22): a committed record whose
 * offset/length were corrupted on flash independently of its digest/state
 * (bit rot -- this format has no way to detect it beyond the bounds check
 * itself) used to fail asset_store_plan_compaction() outright when that
 * record's digest was still in the caller's keep set, because the bounds
 * check ran unconditionally before the keep/not-keep decision. That wedges
 * AssetRelease -- and therefore all GC -- permanently: the host has no way
 * to stop asking to keep a digest it still believes it sent. The fix skips
 * an unusable kept record instead of failing the whole plan. */
static void test_compaction_skips_corrupt_kept_record_instead_of_failing(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t valid_digest[ASSET_DIGEST_BYTES];
    uint8_t corrupt_digest[ASSET_DIGEST_BYTES];
    digest_of(0x78, valid_digest);
    digest_of(0x77, corrupt_digest);

    /* An entirely ordinary kept record, reserved (and so landing at a lower
     * blob offset) before the one that gets corrupted below, to prove the
     * skip does not drop a genuinely valid survivor found earlier in the
     * scan. Both records are reserved and committed while everything is
     * still valid -- asset_store_reserve()/asset_store_commit() run
     * compute_high_water() internally, which has its own (unrelated,
     * out-of-scope-for-this-fix) bounds check over every record, so
     * corruption must not be introduced until both calls are done. */
    uint32_t valid_index = 0U, valid_blob = 0U;
    assert(asset_store_reserve(&store, valid_digest, ASSET_KIND_FONT, 128U,
                               &valid_index, &valid_blob) == ASSET_STORE_OK);
    assert(asset_store_commit(&store, valid_index) == ASSET_STORE_OK);

    uint32_t corrupt_index = 0U, corrupt_blob = 0U;
    assert(asset_store_reserve(&store, corrupt_digest, ASSET_KIND_FONT, 64U,
                               &corrupt_index, &corrupt_blob) == ASSET_STORE_OK);
    assert(asset_store_commit(&store, corrupt_index) == ASSET_STORE_OK);

    /* Now corrupt only the second record's length field directly in the
     * fake flash array -- bypassing the store's own write path entirely,
     * exactly like test_commit_rejects_a_record_wiped_out_from_under_it
     * does -- so it decodes as an ordinary COMMITTED record with a real
     * digest except for an out-of-range length. The record layout
     * (digest[32] + offset[4] + length[4] + ...) is asset_store.c's private
     * encoding, not exported by the header; ASSET_DIGEST_BYTES + 4 is the
     * length field's offset, mirroring how
     * test_record_decode_rejects_unknown_kind already hand-derives the
     * kind/state byte offsets below. plan_compaction() itself never calls
     * compute_high_water(), so this is safe to introduce only now, with no
     * further reserve()/commit()/stats() call to trip over it. */
    uint8_t bogus_length[4] = { 0xFFU, 0xFFU, 0xFFU, 0xFFU };
    memcpy(g_flash + ASSET_HEADER_BYTES + corrupt_index * ASSET_RECORD_BYTES +
              ASSET_DIGEST_BYTES + 4U,
          bogus_length, sizeof bogus_length);

    const uint8_t *keep[2] = { valid_digest, corrupt_digest };
    asset_move_t moves[8];
    size_t move_count = 0U;

    /* The whole point of this fix: a corrupt kept record must not fail the
     * entire plan (that would wedge AssetRelease, and therefore GC,
     * permanently -- the host has no way to stop asking for a digest it
     * still believes it sent). */
    assert(asset_store_plan_compaction(&store, keep, 2U, moves, 8U,
                                       &move_count) == ASSET_STORE_OK);

    /* The corrupt record does not survive -- it is unusable regardless of
     * what the host asked for -- but the valid survivor found before it is
     * still planned, proving the skip did not drop it. */
    assert(move_count == 1U);
    assert(moves[0].length == 128U);
    assert(moves[0].record_index == valid_index);
}

static void test_commit_rejects_a_record_wiped_out_from_under_it(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x44, digest);
    uint32_t index = 0U, blob = 0U;

    assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT, 64U, &index, &blob)
           == ASSET_STORE_OK);

    /* Simulate a compaction (or any other actor) erasing this record's
     * bytes out from under an in-flight reservation before commit runs --
     * exactly what an AssetRelease racing an unaborted
     * AssetBegin/AssetChunk/AssetCommit sequence used to leave behind
     * (protocol_task.c's dispatch_asset_release now aborts first, but this
     * pins the store's own defence in depth against the same desync from
     * any other source). This manipulates the fake flash directly rather
     * than through the store API -- nothing in the public API can produce
     * this state on its own, which is exactly why the store must not trust
     * that it can't happen. */
    memset(g_flash + ASSET_HEADER_BYTES + index * ASSET_RECORD_BYTES, 0xFF,
           ASSET_RECORD_BYTES);

    assert(asset_store_commit(&store, index) == ASSET_STORE_ERR_CORRUPT);
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
    test_committed_record_is_findable_by_digest();
    test_compaction_plan_drops_unreferenced_and_packs();
    test_compaction_plan_reports_capacity_exhaustion();
    test_reserve_does_not_reuse_dead_but_uncompacted_space();
    test_stats_free_blob_bytes_matches_what_reserve_will_grant();
    test_boot_reclaim_marks_orphaned_uncommitted_dead();
    test_boot_reclaim_leaves_free_and_committed_slots_alone();
    test_compaction_skips_corrupt_kept_record_instead_of_failing();
    test_commit_rejects_a_record_wiped_out_from_under_it();
    return 0;
}
