#include "ota_policy.h"

#include <stdint.h>
#include <string.h>

#define CHECK_PATH "/v1/device/firmware?current="
#define FIRMWARE_PATH_PREFIX "/v1/firmware/"
#define FIRMWARE_PATH_SUFFIX ".bin"

typedef struct {
    const char *bytes;
    size_t length;
    size_t position;
} json_cursor_t;

static bool bounded_length(const char *text, size_t maximum, size_t *length)
{
    if (text == NULL) {
        return false;
    }
    const char *end = memchr(text, '\0', maximum + 1U);
    if (end == NULL) {
        return false;
    }
    *length = (size_t)(end - text);
    return true;
}

static bool safe_authority_character(unsigned char byte)
{
    return byte > 0x20U && byte < 0x7fU && byte != (unsigned char)'@' &&
           byte != (unsigned char)'?' && byte != (unsigned char)'#' &&
           byte != (unsigned char)'\\';
}

static bool write_origin(const char *server_url,
                         char *out,
                         size_t out_capacity,
                         size_t *written)
{
    size_t server_length = 0U;
    if (out == NULL || written == NULL || out_capacity == 0U ||
        !bounded_length(server_url, PROTOCOL_MAX_SERVER_URL_LENGTH,
                        &server_length) ||
        server_length <= sizeof("wss://") - 1U ||
        memcmp(server_url, "wss://", sizeof("wss://") - 1U) != 0) {
        return false;
    }

    size_t authority_start = sizeof("wss://") - 1U;
    size_t authority_end = authority_start;
    while (authority_end < server_length && server_url[authority_end] != '/') {
        if (!safe_authority_character(
                (unsigned char)server_url[authority_end])) {
            return false;
        }
        ++authority_end;
    }
    if (authority_end == authority_start) {
        return false;
    }

    size_t authority_length = authority_end - authority_start;
    size_t origin_length = (sizeof("https://") - 1U) + authority_length;
    if (origin_length >= out_capacity) {
        return false;
    }
    memcpy(out, "https://", sizeof("https://") - 1U);
    memcpy(out + sizeof("https://") - 1U,
           server_url + authority_start, authority_length);
    out[origin_length] = '\0';
    *written = origin_length;
    return true;
}

static bool unreserved(unsigned char byte)
{
    return (byte >= (unsigned char)'a' && byte <= (unsigned char)'z') ||
           (byte >= (unsigned char)'A' && byte <= (unsigned char)'Z') ||
           (byte >= (unsigned char)'0' && byte <= (unsigned char)'9') ||
           byte == (unsigned char)'-' || byte == (unsigned char)'.' ||
           byte == (unsigned char)'_' || byte == (unsigned char)'~';
}

bool ota_policy_build_check_url(const char *server_url,
                                const char *current_version,
                                char *out,
                                size_t out_capacity)
{
    size_t version_length = 0U;
    size_t used = 0U;
    if (!bounded_length(current_version,
                        PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH,
                        &version_length) ||
        version_length == 0U ||
        !write_origin(server_url, out, out_capacity, &used)) {
        return false;
    }
    if (sizeof(CHECK_PATH) - 1U > out_capacity - used - 1U) {
        return false;
    }
    memcpy(out + used, CHECK_PATH, sizeof(CHECK_PATH) - 1U);
    used += sizeof(CHECK_PATH) - 1U;

    static const char hex[] = "0123456789ABCDEF";
    for (size_t index = 0U; index < version_length; ++index) {
        unsigned char byte = (unsigned char)current_version[index];
        size_t needed = unreserved(byte) ? 1U : 3U;
        if (needed > out_capacity - used - 1U) {
            return false;
        }
        if (needed == 1U) {
            out[used++] = (char)byte;
        } else {
            out[used++] = '%';
            out[used++] = hex[byte >> 4U];
            out[used++] = hex[byte & UINT8_C(0x0f)];
        }
    }
    out[used] = '\0';
    return true;
}

static bool safe_version(const char *version, size_t length)
{
    if (length == 0U || length > PROTOCOL_MAX_FIRMWARE_VERSION_LENGTH) {
        return false;
    }
    for (size_t index = 0U; index < length; ++index) {
        unsigned char byte = (unsigned char)version[index];
        if (!((byte >= (unsigned char)'a' && byte <= (unsigned char)'z') ||
              (byte >= (unsigned char)'A' && byte <= (unsigned char)'Z') ||
              (byte >= (unsigned char)'0' && byte <= (unsigned char)'9') ||
              byte == (unsigned char)'.' || byte == (unsigned char)'-' ||
              byte == (unsigned char)'_')) {
            return false;
        }
        if (index != 0U && byte == (unsigned char)'.' &&
            version[index - 1U] == '.') {
            return false;
        }
    }
    return true;
}

static bool path_matches_version(const char *path,
                                 size_t path_length,
                                 const char *version,
                                 size_t version_length)
{
    size_t expected_length = (sizeof(FIRMWARE_PATH_PREFIX) - 1U) +
                             version_length +
                             (sizeof(FIRMWARE_PATH_SUFFIX) - 1U);
    return path_length == expected_length &&
           memcmp(path, FIRMWARE_PATH_PREFIX,
                  sizeof(FIRMWARE_PATH_PREFIX) - 1U) == 0 &&
           memcmp(path + sizeof(FIRMWARE_PATH_PREFIX) - 1U,
                  version, version_length) == 0 &&
           memcmp(path + expected_length -
                      (sizeof(FIRMWARE_PATH_SUFFIX) - 1U),
                  FIRMWARE_PATH_SUFFIX,
                  sizeof(FIRMWARE_PATH_SUFFIX) - 1U) == 0;
}

bool ota_policy_build_download_url(const char *server_url,
                                   const char *path,
                                   char *out,
                                   size_t out_capacity)
{
    size_t path_length = 0U;
    size_t used = 0U;
    if (!bounded_length(path, OTA_POLICY_MAX_URL_LENGTH, &path_length) ||
        path_length == 0U || path[0] != '/' ||
        !write_origin(server_url, out, out_capacity, &used) ||
        path_length > out_capacity - used - 1U) {
        return false;
    }
    memcpy(out + used, path, path_length + 1U);
    return true;
}

static void skip_space(json_cursor_t *cursor)
{
    while (cursor->position < cursor->length) {
        char byte = cursor->bytes[cursor->position];
        if (byte != ' ' && byte != '\t' && byte != '\r' && byte != '\n') {
            return;
        }
        ++cursor->position;
    }
}

static bool consume(json_cursor_t *cursor, char expected)
{
    skip_space(cursor);
    if (cursor->position >= cursor->length ||
        cursor->bytes[cursor->position] != expected) {
        return false;
    }
    ++cursor->position;
    return true;
}

static bool parse_string(json_cursor_t *cursor,
                         char *out,
                         size_t out_capacity,
                         size_t *out_length)
{
    if (!consume(cursor, '"') || out == NULL || out_capacity == 0U) {
        return false;
    }
    size_t written = 0U;
    while (cursor->position < cursor->length) {
        unsigned char byte = (unsigned char)cursor->bytes[cursor->position++];
        if (byte == (unsigned char)'"') {
            out[written] = '\0';
            if (out_length != NULL) {
                *out_length = written;
            }
            return true;
        }
        /* The server's values are ASCII and never require JSON escaping. */
        if (byte < 0x20U || byte >= 0x7fU || byte == (unsigned char)'\\' ||
            written >= out_capacity - 1U) {
            return false;
        }
        out[written++] = (char)byte;
    }
    return false;
}

bool ota_policy_parse_metadata(const char *json,
                               size_t length,
                               ota_policy_metadata_t *out)
{
    if (json == NULL || out == NULL || length == 0U ||
        length > OTA_POLICY_MAX_METADATA_LENGTH) {
        return false;
    }
    memset(out, 0, sizeof(*out));
    json_cursor_t cursor = {.bytes = json, .length = length};
    if (!consume(&cursor, '{')) {
        return false;
    }

    bool have_version = false;
    bool have_path = false;
    size_t version_length = 0U;
    size_t path_length = 0U;
    for (size_t pair = 0U; pair < 2U; ++pair) {
        char key[8];
        size_t key_length = 0U;
        if (!parse_string(&cursor, key, sizeof(key), &key_length) ||
            !consume(&cursor, ':')) {
            return false;
        }
        if (key_length == sizeof("version") - 1U &&
            memcmp(key, "version", sizeof("version") - 1U) == 0 &&
            !have_version) {
            if (!parse_string(&cursor, out->version,
                              sizeof(out->version), &version_length)) {
                return false;
            }
            have_version = true;
        } else if (key_length == sizeof("url") - 1U &&
                   memcmp(key, "url", sizeof("url") - 1U) == 0 &&
                   !have_path) {
            if (!parse_string(&cursor, out->path, sizeof(out->path),
                              &path_length)) {
                return false;
            }
            have_path = true;
        } else {
            return false;
        }
        if (pair == 0U && !consume(&cursor, ',')) {
            return false;
        }
    }
    if (!consume(&cursor, '}')) {
        return false;
    }
    skip_space(&cursor);
    if (cursor.position != cursor.length || !have_version || !have_path ||
        !safe_version(out->version, version_length) ||
        !path_matches_version(out->path, path_length, out->version,
                              version_length)) {
        memset(out, 0, sizeof(*out));
        return false;
    }
    return true;
}

bool ota_policy_update_deferred(bool interrupt_live,
                                bool progress_timer_running)
{
    return interrupt_live || progress_timer_running;
}
