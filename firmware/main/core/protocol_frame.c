#include "protocol_frame.h"

#include <string.h>

static uint16_t read_u16_le(const uint8_t *bytes)
{
    return (uint16_t)bytes[0] | ((uint16_t)bytes[1] << 8U);
}

static uint32_t read_u32_le(const uint8_t *bytes)
{
    return (uint32_t)bytes[0] |
           ((uint32_t)bytes[1] << 8U) |
           ((uint32_t)bytes[2] << 16U) |
           ((uint32_t)bytes[3] << 24U);
}

static void write_u16_le(uint8_t *bytes, uint16_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8U);
}

static void write_u32_le(uint8_t *bytes, uint32_t value)
{
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8U);
    bytes[2] = (uint8_t)(value >> 16U);
    bytes[3] = (uint8_t)(value >> 24U);
}

uint32_t protocol_crc32c(const uint8_t *bytes, size_t length)
{
    uint32_t crc = UINT32_MAX;
    if (bytes == NULL && length != 0U) {
        return 0U;
    }
    for (size_t i = 0; i < length; ++i) {
        crc ^= bytes[i];
        for (unsigned bit = 0; bit < 8U; ++bit) {
            crc = (crc >> 1U) ^
                  (0x82f63b78U & (0U - (crc & 1U)));
        }
    }
    return ~crc;
}

static protocol_frame_result_t cobs_encode(const uint8_t *decoded,
                                           size_t decoded_length,
                                           uint8_t *encoded,
                                           size_t encoded_capacity,
                                           size_t *encoded_length)
{
    if (decoded == NULL || encoded == NULL || encoded_length == NULL ||
        encoded_capacity == 0U) {
        return PROTOCOL_FRAME_ERR_ARGUMENT;
    }

    size_t write = 1U;
    size_t code_index = 0U;
    uint8_t code = 1U;
    for (size_t read = 0; read < decoded_length; ++read) {
        if (decoded[read] == 0U) {
            encoded[code_index] = code;
            if (write >= encoded_capacity) {
                return PROTOCOL_FRAME_ERR_OVERLONG;
            }
            code_index = write++;
            code = 1U;
        } else {
            if (write >= encoded_capacity) {
                return PROTOCOL_FRAME_ERR_OVERLONG;
            }
            encoded[write++] = decoded[read];
            ++code;
            if (code == UINT8_MAX) {
                encoded[code_index] = code;
                if (write >= encoded_capacity) {
                    return PROTOCOL_FRAME_ERR_OVERLONG;
                }
                code_index = write++;
                code = 1U;
            }
        }
    }
    encoded[code_index] = code;
    *encoded_length = write;
    return PROTOCOL_FRAME_OK;
}

static protocol_frame_result_t cobs_decode(const uint8_t *encoded,
                                           size_t encoded_length,
                                           uint8_t *decoded,
                                           size_t decoded_capacity,
                                           size_t *decoded_length)
{
    if (encoded == NULL || decoded == NULL || decoded_length == NULL ||
        encoded_length == 0U) {
        return PROTOCOL_FRAME_ERR_COBS;
    }
    size_t read = 0U;
    size_t write = 0U;
    while (read < encoded_length) {
        uint8_t code = encoded[read++];
        if (code == 0U) {
            return PROTOCOL_FRAME_ERR_COBS;
        }
        size_t count = (size_t)code - 1U;
        if (count > encoded_length - read || count > decoded_capacity - write) {
            return count > decoded_capacity - write
                       ? PROTOCOL_FRAME_ERR_OVERLONG
                       : PROTOCOL_FRAME_ERR_COBS;
        }
        memcpy(decoded + write, encoded + read, count);
        read += count;
        write += count;
        if (code != UINT8_MAX && read < encoded_length) {
            if (write == decoded_capacity) {
                return PROTOCOL_FRAME_ERR_OVERLONG;
            }
            decoded[write++] = 0U;
        }
    }
    *decoded_length = write;
    return PROTOCOL_FRAME_OK;
}

protocol_frame_result_t protocol_frame_encode(const protocol_frame_t *frame,
                                               uint8_t *wire,
                                               size_t wire_capacity,
                                               size_t *wire_length)
{
    if (frame == NULL || wire == NULL || wire_length == NULL) {
        return PROTOCOL_FRAME_ERR_ARGUMENT;
    }
    if (frame->flags != 0U) {
        return PROTOCOL_FRAME_ERR_FLAGS;
    }
    if (frame->payload_length > PROTOCOL_MAX_PAYLOAD_SIZE) {
        return PROTOCOL_FRAME_ERR_OVERLONG;
    }

    uint8_t decoded[PROTOCOL_MAX_DECODED_FRAME];
    size_t body_length = PROTOCOL_HEADER_SIZE + frame->payload_length;
    decoded[0] = frame->version;
    decoded[1] = frame->message_type;
    write_u16_le(decoded + 2U, frame->flags);
    write_u32_le(decoded + 4U, frame->request_id);
    write_u16_le(decoded + 8U, frame->payload_length);
    memcpy(decoded + PROTOCOL_HEADER_SIZE, frame->payload,
           frame->payload_length);
    write_u32_le(decoded + body_length,
                 protocol_crc32c(decoded, body_length));
    size_t decoded_length = body_length + PROTOCOL_CHECKSUM_SIZE;

    if (wire_capacity < 2U) {
        return PROTOCOL_FRAME_ERR_OVERLONG;
    }
    size_t encoded_length = 0U;
    protocol_frame_result_t result =
        cobs_encode(decoded, decoded_length, wire, wire_capacity - 1U,
                    &encoded_length);
    if (result != PROTOCOL_FRAME_OK) {
        return result;
    }
    wire[encoded_length] = 0U;
    *wire_length = encoded_length + 1U;
    return PROTOCOL_FRAME_OK;
}

protocol_frame_result_t protocol_frame_decode(const uint8_t *wire,
                                               size_t wire_length,
                                               protocol_frame_t *frame)
{
    if (wire == NULL || frame == NULL) {
        return PROTOCOL_FRAME_ERR_ARGUMENT;
    }
    if (wire_length == 0U || wire[wire_length - 1U] != 0U) {
        return PROTOCOL_FRAME_ERR_MISSING_DELIMITER;
    }
    if (wire_length > PROTOCOL_MAX_WIRE_FRAME) {
        return PROTOCOL_FRAME_ERR_OVERLONG;
    }
    for (size_t i = 0; i + 1U < wire_length; ++i) {
        if (wire[i] == 0U) {
            return PROTOCOL_FRAME_ERR_EMBEDDED_DELIMITER;
        }
    }

    uint8_t decoded[PROTOCOL_MAX_DECODED_FRAME];
    size_t decoded_length = 0U;
    protocol_frame_result_t result =
        cobs_decode(wire, wire_length - 1U, decoded, sizeof(decoded),
                    &decoded_length);
    if (result != PROTOCOL_FRAME_OK) {
        return result;
    }
    if (decoded_length < PROTOCOL_HEADER_SIZE + PROTOCOL_CHECKSUM_SIZE) {
        return PROTOCOL_FRAME_ERR_TOO_SHORT;
    }
    uint16_t payload_length = read_u16_le(decoded + 8U);
    if (decoded_length != PROTOCOL_HEADER_SIZE + payload_length +
                              PROTOCOL_CHECKSUM_SIZE) {
        return PROTOCOL_FRAME_ERR_LENGTH;
    }
    size_t checksum_offset = decoded_length - PROTOCOL_CHECKSUM_SIZE;
    if (read_u32_le(decoded + checksum_offset) !=
        protocol_crc32c(decoded, checksum_offset)) {
        return PROTOCOL_FRAME_ERR_CHECKSUM;
    }

    frame->version = decoded[0];
    frame->message_type = decoded[1];
    frame->flags = read_u16_le(decoded + 2U);
    frame->request_id = read_u32_le(decoded + 4U);
    frame->payload_length = payload_length;
    if (frame->flags != 0U) {
        return PROTOCOL_FRAME_ERR_FLAGS;
    }
    memcpy(frame->payload, decoded + PROTOCOL_HEADER_SIZE, payload_length);
    return PROTOCOL_FRAME_OK;
}

void protocol_decoder_init(protocol_decoder_t *decoder)
{
    if (decoder != NULL) {
        memset(decoder, 0, sizeof(*decoder));
    }
}

void protocol_decoder_feed(protocol_decoder_t *decoder,
                           const uint8_t *bytes,
                           size_t length,
                           protocol_frame_callback_t callback,
                           void *context)
{
    if (decoder == NULL || (bytes == NULL && length != 0U) || callback == NULL) {
        return;
    }
    for (size_t i = 0; i < length; ++i) {
        uint8_t byte = bytes[i];
        if (decoder->discarding) {
            if (byte == 0U) {
                decoder->discarding = false;
                callback(PROTOCOL_FRAME_ERR_OVERLONG, NULL, context);
            }
            continue;
        }
        if (byte == 0U) {
            if (decoder->encoded_length != 0U) {
                decoder->encoded[decoder->encoded_length] = 0U;
                protocol_frame_result_t result = protocol_frame_decode(
                    decoder->encoded, decoder->encoded_length + 1U,
                    &decoder->frame);
                callback(result,
                         result == PROTOCOL_FRAME_OK ? &decoder->frame : NULL,
                         context);
                decoder->encoded_length = 0U;
            }
        } else if (decoder->encoded_length == PROTOCOL_MAX_ENCODED_FRAME) {
            decoder->encoded_length = 0U;
            decoder->discarding = true;
        } else {
            decoder->encoded[decoder->encoded_length++] = byte;
        }
    }
}
