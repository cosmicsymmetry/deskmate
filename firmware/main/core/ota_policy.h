#pragma once

#include <stdbool.h>
#include <stddef.h>

#include "protocol_message.h"

#define OTA_POLICY_MAX_URL_LENGTH 256U
#define OTA_POLICY_MAX_METADATA_LENGTH 256U

typedef struct {
    char version[PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH + 1U];
    char path[OTA_POLICY_MAX_URL_LENGTH + 1U];
} ota_policy_metadata_t;

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
