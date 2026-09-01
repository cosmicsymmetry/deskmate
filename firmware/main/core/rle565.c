#include "core/rle565.h"

#include <string.h>

rle565_result_t rle565_decoder_init(rle565_decoder_t *decoder,
                                    uint8_t *output,
                                    size_t output_capacity)
{
    if (decoder == NULL || (output_capacity > 0U && output == NULL) ||
        (output_capacity % 2U) != 0U) {
        return RLE565_ERR_ARGUMENT;
    }
    memset(decoder, 0, sizeof *decoder);
    decoder->output = output;
    decoder->output_capacity = output_capacity;
    return RLE565_OK;
}

static rle565_result_t decode_pending_run(rle565_decoder_t *decoder)
{
    uint16_t count = (uint16_t)((uint16_t)decoder->pending[0] |
                                ((uint16_t)decoder->pending[1] << 8U));
    if (count == 0U) {
        return RLE565_ERR_ZERO_COUNT;
    }
    size_t remaining = decoder->output_capacity - decoder->output_length;
    if ((size_t)count > remaining / 2U) {
        return RLE565_ERR_OUTPUT_OVERRUN;
    }
    for (uint16_t i = 0U; i < count; ++i) {
        decoder->output[decoder->output_length++] = decoder->pending[2];
        decoder->output[decoder->output_length++] = decoder->pending[3];
    }
    decoder->pending_length = 0U;
    return RLE565_OK;
}

rle565_result_t rle565_decoder_feed(rle565_decoder_t *decoder,
                                    const uint8_t *input,
                                    size_t input_length)
{
    if (decoder == NULL || (input_length > 0U && input == NULL)) {
        return RLE565_ERR_ARGUMENT;
    }
    if (decoder->result != RLE565_OK) {
        return decoder->result;
    }
    if (input_length > 0U &&
        decoder->output_length == decoder->output_capacity) {
        decoder->result = RLE565_ERR_TRAILING_DATA;
        return decoder->result;
    }
    for (size_t i = 0U; i < input_length; ++i) {
        if (decoder->output_length == decoder->output_capacity &&
            decoder->pending_length == 0U) {
            decoder->result = RLE565_ERR_TRAILING_DATA;
            return decoder->result;
        }
        decoder->pending[decoder->pending_length++] = input[i];
        if (decoder->pending_length == sizeof decoder->pending) {
            decoder->result = decode_pending_run(decoder);
            if (decoder->result != RLE565_OK) {
                return decoder->result;
            }
        }
    }
    return RLE565_OK;
}

rle565_result_t rle565_decoder_finish(rle565_decoder_t *decoder)
{
    if (decoder == NULL) {
        return RLE565_ERR_ARGUMENT;
    }
    if (decoder->result != RLE565_OK) {
        return decoder->result;
    }
    if (decoder->pending_length != 0U) {
        decoder->result = RLE565_ERR_TRUNCATED_STREAM;
    } else if (decoder->output_length != decoder->output_capacity) {
        decoder->result = RLE565_ERR_OUTPUT_SHORT;
    }
    return decoder->result;
}

