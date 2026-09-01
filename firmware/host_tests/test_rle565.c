#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "core/rle565.h"

static const uint8_t ENCODED[] = {
    3, 0, 0x34, 0x12, 1, 0, 0xcd, 0xab, 2, 0, 0xff, 0x00,
};
static const uint8_t DECODED[] = {
    0x34, 0x12, 0x34, 0x12, 0x34, 0x12,
    0xcd, 0xab, 0xff, 0x00, 0xff, 0x00,
};

static void decode_with_chunk_size(size_t chunk_size)
{
    uint8_t guarded[sizeof DECODED + 2U];
    memset(guarded, 0xa5, sizeof guarded);
    rle565_decoder_t decoder;
    assert(rle565_decoder_init(&decoder, guarded + 1U, sizeof DECODED) ==
           RLE565_OK);
    size_t offset = 0U;
    while (offset < sizeof ENCODED) {
        size_t length = sizeof ENCODED - offset;
        if (length > chunk_size) length = chunk_size;
        assert(rle565_decoder_feed(&decoder, ENCODED + offset, length) ==
               RLE565_OK);
        offset += length;
    }
    assert(rle565_decoder_finish(&decoder) == RLE565_OK);
    assert(decoder.output_length == sizeof DECODED);
    assert(memcmp(guarded + 1U, DECODED, sizeof DECODED) == 0);
    assert(guarded[0] == 0xa5U);
    assert(guarded[sizeof guarded - 1U] == 0xa5U);
}

static void test_every_split_boundary(void)
{
    for (size_t split = 1U; split <= sizeof ENCODED; ++split) {
        uint8_t output[sizeof DECODED];
        rle565_decoder_t decoder;
        assert(rle565_decoder_init(&decoder, output, sizeof output) ==
               RLE565_OK);
        assert(rle565_decoder_feed(&decoder, ENCODED, split) == RLE565_OK);
        assert(rle565_decoder_feed(&decoder, ENCODED + split,
                                   sizeof ENCODED - split) == RLE565_OK);
        assert(rle565_decoder_finish(&decoder) == RLE565_OK);
        assert(memcmp(output, DECODED, sizeof output) == 0);
    }
}

static void test_one_byte_at_a_time(void) { decode_with_chunk_size(1U); }

static void test_zero_count_is_rejected(void)
{
    uint8_t output[2];
    const uint8_t input[] = {0, 0, 0x34, 0x12};
    rle565_decoder_t decoder;
    assert(rle565_decoder_init(&decoder, output, sizeof output) == RLE565_OK);
    assert(rle565_decoder_feed(&decoder, input, sizeof input) ==
           RLE565_ERR_ZERO_COUNT);
}

static void test_decoded_overrun_is_rejected_without_writing(void)
{
    uint8_t guarded[] = {0xa5, 0xa5, 0xa5, 0xa5};
    const uint8_t input[] = {2, 0, 0x34, 0x12};
    rle565_decoder_t decoder;
    assert(rle565_decoder_init(&decoder, guarded + 1U, 2U) == RLE565_OK);
    assert(rle565_decoder_feed(&decoder, input, sizeof input) ==
           RLE565_ERR_OUTPUT_OVERRUN);
    assert(guarded[0] == 0xa5U && guarded[3] == 0xa5U);
}

static void test_trailing_garbage_is_rejected(void)
{
    uint8_t output[2];
    const uint8_t exact[] = {1, 0, 0x34, 0x12};
    const uint8_t garbage = 0xff;
    rle565_decoder_t decoder;
    assert(rle565_decoder_init(&decoder, output, sizeof output) == RLE565_OK);
    assert(rle565_decoder_feed(&decoder, exact, sizeof exact) == RLE565_OK);
    assert(rle565_decoder_feed(&decoder, &garbage, 1U) ==
           RLE565_ERR_TRAILING_DATA);
}

static void test_truncated_run_is_rejected_at_finish(void)
{
    uint8_t output[2];
    const uint8_t input[] = {1, 0, 0x34};
    rle565_decoder_t decoder;
    assert(rle565_decoder_init(&decoder, output, sizeof output) == RLE565_OK);
    assert(rle565_decoder_feed(&decoder, input, sizeof input) == RLE565_OK);
    assert(rle565_decoder_finish(&decoder) == RLE565_ERR_TRUNCATED_STREAM);
}

static void test_short_output_is_rejected_at_finish(void)
{
    uint8_t output[4];
    const uint8_t input[] = {1, 0, 0x34, 0x12};
    rle565_decoder_t decoder;
    assert(rle565_decoder_init(&decoder, output, sizeof output) == RLE565_OK);
    assert(rle565_decoder_feed(&decoder, input, sizeof input) == RLE565_OK);
    assert(rle565_decoder_finish(&decoder) == RLE565_ERR_OUTPUT_SHORT);
}

static void test_invalid_output_capacity_is_rejected(void)
{
    uint8_t output[3];
    rle565_decoder_t decoder;
    assert(rle565_decoder_init(&decoder, output, sizeof output) ==
           RLE565_ERR_ARGUMENT);
}

int main(void)
{
    test_every_split_boundary();
    test_one_byte_at_a_time();
    test_zero_count_is_rejected();
    test_decoded_overrun_is_rejected_without_writing();
    test_trailing_garbage_is_rejected();
    test_truncated_run_is_rejected_at_finish();
    test_short_output_is_rejected_at_finish();
    test_invalid_output_capacity_is_rejected();
    puts("test_rle565: OK (8 tests)");
    return 0;
}
