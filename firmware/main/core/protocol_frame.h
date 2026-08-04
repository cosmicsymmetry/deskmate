#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define PROTOCOL_VERSION 1U
#define PROTOCOL_MAX_DECODED_FRAME 2048U
#define PROTOCOL_HEADER_SIZE 10U
#define PROTOCOL_CHECKSUM_SIZE 4U
#define PROTOCOL_MAX_PAYLOAD_SIZE \
    (PROTOCOL_MAX_DECODED_FRAME - PROTOCOL_HEADER_SIZE - PROTOCOL_CHECKSUM_SIZE)
#define PROTOCOL_MAX_WIRE_FRAME 2058U
#define PROTOCOL_MAX_ENCODED_FRAME (PROTOCOL_MAX_WIRE_FRAME - 1U)

typedef enum {
    PROTOCOL_FRAME_OK = 0,
    PROTOCOL_FRAME_ERR_ARGUMENT,
    PROTOCOL_FRAME_ERR_MISSING_DELIMITER,
    PROTOCOL_FRAME_ERR_EMBEDDED_DELIMITER,
    PROTOCOL_FRAME_ERR_OVERLONG,
    PROTOCOL_FRAME_ERR_COBS,
    PROTOCOL_FRAME_ERR_TOO_SHORT,
    PROTOCOL_FRAME_ERR_LENGTH,
    PROTOCOL_FRAME_ERR_CHECKSUM,
    PROTOCOL_FRAME_ERR_FLAGS,
} protocol_frame_result_t;

typedef struct {
    uint8_t version;
    uint8_t message_type;
    uint16_t flags;
    uint32_t request_id;
    uint16_t payload_length;
    uint8_t payload[PROTOCOL_MAX_PAYLOAD_SIZE];
} protocol_frame_t;

typedef struct {
    // Keep incremental wire and decoded-frame scratch out of the protocol
    // task's stack. The callback may use frame only until it returns.
    uint8_t encoded[PROTOCOL_MAX_WIRE_FRAME];
    protocol_frame_t frame;
    size_t encoded_length;
    bool discarding;
} protocol_decoder_t;

typedef void (*protocol_frame_callback_t)(protocol_frame_result_t result,
                                          const protocol_frame_t *frame,
                                          void *context);

uint32_t protocol_crc32c(const uint8_t *bytes, size_t length);

protocol_frame_result_t protocol_frame_encode(const protocol_frame_t *frame,
                                               uint8_t *wire,
                                               size_t wire_capacity,
                                               size_t *wire_length);

protocol_frame_result_t protocol_frame_decode(const uint8_t *wire,
                                               size_t wire_length,
                                               protocol_frame_t *frame);

void protocol_decoder_init(protocol_decoder_t *decoder);

void protocol_decoder_feed(protocol_decoder_t *decoder,
                           const uint8_t *bytes,
                           size_t length,
                           protocol_frame_callback_t callback,
                           void *context);
