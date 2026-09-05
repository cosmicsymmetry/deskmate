#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "protocol_message.h"

#define OTA_POLICY_MAX_URL_LENGTH 256U
#define OTA_POLICY_MAX_METADATA_LENGTH 256U
#define OTA_POLICY_FIRMWARE_PATH_PREFIX "/v1/firmware/"
#define OTA_POLICY_FIRMWARE_PATH_SUFFIX ".bin"
#define OTA_POLICY_MAX_FIRMWARE_PATH_LENGTH                              \
    ((sizeof(OTA_POLICY_FIRMWARE_PATH_PREFIX) - 1U) +                    \
     PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH +                              \
     (sizeof(OTA_POLICY_FIRMWARE_PATH_SUFFIX) - 1U))
#define OTA_POLICY_FIRMWARE_PATH_CAPACITY \
    (OTA_POLICY_MAX_FIRMWARE_PATH_LENGTH + 1U)
#define OTA_POLICY_NO_PROGRESS_TIMEOUT_US UINT64_C(120000000)
#define OTA_POLICY_TOTAL_TIMEOUT_US UINT64_C(1800000000)

typedef struct {
    char version[PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH + 1U];
    char path[OTA_POLICY_FIRMWARE_PATH_CAPACITY];
} ota_policy_metadata_t;

typedef enum {
    OTA_FAILURE_STAGE_NONE = 0,
    OTA_FAILURE_STAGE_TASK = 1,
    OTA_FAILURE_STAGE_CHECK = 2,
    OTA_FAILURE_STAGE_BEGIN = 3,
    OTA_FAILURE_STAGE_DOWNLOAD = 4,
    OTA_FAILURE_STAGE_VERIFY = 5,
} ota_failure_stage_t;

typedef struct {
    ota_failure_stage_t stage;
    int32_t error;
    uint16_t http_status;
} ota_failure_t;

/** Build the authenticated update-check URL from a stored WSS link URL. */
bool ota_policy_build_check_url(
    const char *server_url,
    const char *current_version,
    char *out,
    size_t out_capacity);

/** Build an HTTPS image URL from a validated relative metadata path. */
bool ota_policy_build_download_url(
    const char *server_url,
    const char *path,
    char *out,
    size_t out_capacity);

/**
 * Parse the server's deliberately tiny metadata object.
 *
 * The accepted form has exactly one `version` and one `url` string. Escapes,
 * duplicate/unknown fields, unsafe versions, and paths other than
 * `/v1/firmware/<version>.bin` are rejected.
 */
bool ota_policy_parse_metadata(
    const char *json,
    size_t length,
    ota_policy_metadata_t *out);

/** A live interrupt and a running focus timer independently defer OTA. */
bool ota_policy_update_deferred(bool interrupt_live,
                                bool progress_timer_running);

/** Wait for host state only until the bounded owner-readiness deadline. */
bool ota_policy_should_wait_for_owner(bool owner_state_ready,
                                      uint32_t waited_ms,
                                      uint32_t limit_ms);

/** Convert a millisecond interval without overflowing a 32-bit multiply. */
uint32_t ota_policy_delay_ticks(uint32_t milliseconds,
                                uint32_t ticks_per_second);

/** Bound both a stalled transfer and a pathologically slow trickle. */
bool ota_policy_download_timed_out(uint64_t total_elapsed_us,
                                   uint64_t no_progress_elapsed_us);

/**
 * Pack a complete OTA failure into one 64-bit value.
 *
 * Bits 0..2 are the stage, 3..34 are the exact two's-complement esp_err_t
 * bits, and 35..50 are the HTTP status (zero when absent). Bits 51..63 are
 * reserved as zero.
 */
uint64_t ota_policy_pack_failure(ota_failure_stage_t stage,
                                 int32_t error,
                                 uint16_t http_status);

/** Recover every packed field without narrowing or sign loss. */
ota_failure_t ota_policy_unpack_failure(uint64_t packed);

/** Return the stable wire spelling for a failure stage. */
const char *ota_policy_failure_stage_name(ota_failure_stage_t stage);

/** Format a human-readable OTA failure and truncate it to the wire bound. */
void ota_policy_format_failure(char *out,
                               size_t out_capacity,
                               const char *stage,
                               const char *detail);

/** Format an HTTP failure directly into the caller's bounded buffer. */
void ota_policy_format_http_failure(char *out,
                                    size_t out_capacity,
                                    const char *stage,
                                    uint16_t http_status);
