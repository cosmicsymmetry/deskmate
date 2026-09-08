#pragma once

#include <stddef.h>
#include <stdint.h>

typedef enum {
    RLE565_OK = 0,
    RLE565_ERR_ARGUMENT,
    RLE565_ERR_ZERO_COUNT,
    RLE565_ERR_OUTPUT_OVERRUN,
    RLE565_ERR_TRAILING_DATA,
    RLE565_ERR_TRUNCATED_STREAM,
    RLE565_ERR_OUTPUT_SHORT,
} rle565_result_t;

/* Bounded streaming decoder for `(u16 count LE, u16 pixel LE)` runs.
 * All mutable state belongs to the caller. `output_capacity` is the exact
 * decoded byte length promised by the enclosing AssetBegin. */
typedef struct {
    uint8_t *output;
    size_t output_capacity;
    size_t output_length;
    uint8_t pending[4];
    uint8_t pending_length;
    rle565_result_t result;
} rle565_decoder_t;

rle565_result_t rle565_decoder_init(rle565_decoder_t *decoder,
                                    uint8_t *output,
                                    size_t output_capacity);
rle565_result_t rle565_decoder_feed(rle565_decoder_t *decoder,
                                    const uint8_t *input,
                                    size_t input_length);

/* Distinguishes a partial four-byte run from a valid stream whose decoded
 * output ended before the promised capacity. */
rle565_result_t rle565_decoder_finish(rle565_decoder_t *decoder);

