#include <assert.h>
#include <stdio.h>
#include <string.h>

#include "../main/core/ota_policy.h"

static void test_urls_use_the_server_origin_and_https(void)
{
    char url[OTA_POLICY_MAX_URL_LENGTH + 1U];
    assert(ota_policy_build_check_url(
        "wss://deskmate.example/v1/device/link", "1.0 beta", url,
        sizeof(url)));
    assert(strcmp(url,
                  "https://deskmate.example/v1/device/firmware?current="
                  "1.0%20beta") == 0);

    assert(ota_policy_build_download_url(
        "wss://deskmate.example/v1/device/link",
        "/v1/firmware/1.1.0.bin", url, sizeof(url)));
    assert(strcmp(url,
                  "https://deskmate.example/v1/firmware/1.1.0.bin") == 0);
}

static void test_urls_reject_unsafe_or_truncated_inputs(void)
{
    char url[OTA_POLICY_MAX_URL_LENGTH + 1U];
    assert(!ota_policy_build_check_url("ws://insecure/link", "1.0.0", url,
                                       sizeof(url)));
    assert(!ota_policy_build_check_url("wss://token@host/link", "1.0.0",
                                       url, sizeof(url)));
    assert(!ota_policy_build_check_url("wss:///missing-host", "1.0.0", url,
                                       sizeof(url)));
    assert(!ota_policy_build_download_url("wss://host/link", "//evil.test/x",
                                          url, 8U));
}

static void test_metadata_is_bounded_and_tied_to_its_version(void)
{
    static const char valid[] =
        "{\"version\":\"1.1.0\",\"url\":\"/v1/firmware/1.1.0.bin\"}";
    ota_policy_metadata_t metadata;
    assert(ota_policy_parse_metadata(valid, sizeof(valid) - 1U, &metadata));
    assert(strcmp(metadata.version, "1.1.0") == 0);
    assert(strcmp(metadata.path, "/v1/firmware/1.1.0.bin") == 0);

    static const char reversed[] =
        " { \"url\" : \"/v1/firmware/v2.bin\", "
        "\"version\" : \"v2\" } ";
    assert(ota_policy_parse_metadata(reversed, sizeof(reversed) - 1U,
                                     &metadata));

    static const char mismatch[] =
        "{\"version\":\"1.1.0\",\"url\":\"/v1/firmware/9.9.9.bin\"}";
    assert(!ota_policy_parse_metadata(mismatch, sizeof(mismatch) - 1U,
                                      &metadata));
    static const char duplicate[] =
        "{\"version\":\"1.1.0\",\"version\":\"1.1.0\"}";
    assert(!ota_policy_parse_metadata(duplicate, sizeof(duplicate) - 1U,
                                      &metadata));
    static const char escaped[] =
        "{\"version\":\"1.1\\u002e0\","
        "\"url\":\"/v1/firmware/1.1.0.bin\"}";
    assert(!ota_policy_parse_metadata(escaped, sizeof(escaped) - 1U,
                                      &metadata));
    static const char traversal[] =
        "{\"version\":\"1..0\",\"url\":\"/v1/firmware/1..0.bin\"}";
    assert(!ota_policy_parse_metadata(traversal, sizeof(traversal) - 1U,
                                      &metadata));
    char oversized[OTA_POLICY_MAX_METADATA_LENGTH + 1U];
    memset(oversized, 'x', sizeof(oversized));
    assert(!ota_policy_parse_metadata(oversized, sizeof(oversized),
                                      &metadata));
}

static void test_either_live_focus_state_defers_update(void)
{
    assert(!ota_policy_update_deferred(false, false));
    assert(ota_policy_update_deferred(true, false));
    assert(ota_policy_update_deferred(false, true));
    assert(ota_policy_update_deferred(true, true));
}

int main(void)
{
    test_urls_use_the_server_origin_and_https();
    test_urls_reject_unsafe_or_truncated_inputs();
    test_metadata_is_bounded_and_tied_to_its_version();
    test_either_live_focus_state_defers_update();
    puts("test_ota_policy: OK");
    return 0;
}
