#include <assert.h>
#include <string.h>

#include "core/asset_transfer.h"

static void digest_of(uint8_t seed, uint8_t *out)
{
    memset(out, seed, ASSET_DIGEST_BYTES);
}

static void test_in_order_chunks_complete_the_transfer(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x01, digest);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 300U)
           == ASSET_TRANSFER_OK);
    assert(!asset_transfer_is_complete(&t));
    assert(asset_transfer_accept_chunk(&t, digest, 0U, 200U) == ASSET_TRANSFER_OK);
    assert(asset_transfer_accept_chunk(&t, digest, 200U, 100U) == ASSET_TRANSFER_OK);
    assert(asset_transfer_is_complete(&t));
}

static void test_repeated_chunk_is_idempotent(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x02, digest);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 300U)
           == ASSET_TRANSFER_OK);
    assert(asset_transfer_accept_chunk(&t, digest, 0U, 200U) == ASSET_TRANSFER_OK);
    /* The host resent because our Ack was lost. Advancing twice would corrupt
     * the blob; erroring would make a dropped Ack fatal. Neither is acceptable. */
    assert(asset_transfer_accept_chunk(&t, digest, 0U, 200U)
           == ASSET_TRANSFER_DUPLICATE);
    assert(asset_transfer_accept_chunk(&t, digest, 200U, 100U)
           == ASSET_TRANSFER_OK);
    assert(asset_transfer_is_complete(&t));
}

static void test_out_of_order_chunk_is_rejected(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x03, digest);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 300U)
           == ASSET_TRANSFER_OK);
    assert(asset_transfer_accept_chunk(&t, digest, 100U, 50U)
           == ASSET_TRANSFER_ERR_OFFSET);
}

static void test_chunk_for_another_digest_is_rejected(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES], other[ASSET_DIGEST_BYTES];
    digest_of(0x04, digest);
    digest_of(0x05, other);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 300U)
           == ASSET_TRANSFER_OK);
    assert(asset_transfer_accept_chunk(&t, other, 0U, 10U)
           == ASSET_TRANSFER_ERR_DIGEST);
}

static void test_overflow_past_total_length_is_rejected(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x06, digest);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 100U)
           == ASSET_TRANSFER_OK);
    assert(asset_transfer_accept_chunk(&t, digest, 0U, 101U)
           == ASSET_TRANSFER_ERR_LENGTH);
}

static void test_chunk_without_begin_is_rejected(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x07, digest);
    memset(&t, 0, sizeof t);

    assert(asset_transfer_accept_chunk(&t, digest, 0U, 10U)
           == ASSET_TRANSFER_ERR_INACTIVE);
}

static void test_begin_rejects_zero_and_oversize_length(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x08, digest);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 0U)
           == ASSET_TRANSFER_ERR_LENGTH);
    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT,
                                ASSET_MAX_BYTES + 1U)
           == ASSET_TRANSFER_ERR_LENGTH);
}

int main(void)
{
    test_in_order_chunks_complete_the_transfer();
    test_repeated_chunk_is_idempotent();
    test_out_of_order_chunk_is_rejected();
    test_chunk_for_another_digest_is_rejected();
    test_overflow_past_total_length_is_rejected();
    test_chunk_without_begin_is_rejected();
    test_begin_rejects_zero_and_oversize_length();
    return 0;
}
