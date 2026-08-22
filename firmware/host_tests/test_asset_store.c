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

int main(void)
{
    test_format_then_open_roundtrips();
    test_open_rejects_bad_magic();
    test_record_encode_decode_roundtrips();
    test_record_decode_rejects_unknown_kind();
    return 0;
}
