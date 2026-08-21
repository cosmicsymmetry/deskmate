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
    assert(!ota_policy_build_download_url("wss://host/link", "not/absolute",
                                          url, sizeof(url)));
    assert(!ota_policy_build_download_url("wss://host/link", "//evil.test/x",
                                          url, sizeof(url)));
    assert(!ota_policy_build_download_url(
        "wss://host/link", "/v1/firmware/1.1.0.bin", url,
        sizeof("https://host")));
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
    memcpy(oversized, valid, sizeof(valid) - 1U);
    memset(oversized + sizeof(valid) - 1U, ' ',
           sizeof(oversized) - (sizeof(valid) - 1U));
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

static void test_owner_wait_is_bounded_and_readiness_wins(void)
{
    assert(ota_policy_should_wait_for_owner(false, 0U, 60000U));
    assert(ota_policy_should_wait_for_owner(false, 59999U, 60000U));
    assert(!ota_policy_should_wait_for_owner(false, 60000U, 60000U));
    assert(!ota_policy_should_wait_for_owner(false, 60001U, 60000U));
    assert(!ota_policy_should_wait_for_owner(true, 0U, 60000U));
}

static void test_day_scale_delays_convert_without_32_bit_overflow(void)
{
    assert(ota_policy_delay_ticks(82800000U, 100U) == 8280000U);
    assert(ota_policy_delay_ticks(90000000U, 100U) == 9000000U);
    assert(ota_policy_delay_ticks(UINT32_MAX, UINT32_MAX) == UINT32_MAX);
}

static void test_download_deadlines_bound_stalls_and_slow_trickles(void)
{
    assert(!ota_policy_download_timed_out(
        OTA_POLICY_TOTAL_TIMEOUT_US - 1U,
        OTA_POLICY_NO_PROGRESS_TIMEOUT_US - 1U));
    assert(ota_policy_download_timed_out(
        OTA_POLICY_TOTAL_TIMEOUT_US,
        OTA_POLICY_NO_PROGRESS_TIMEOUT_US - 1U));
    assert(ota_policy_download_timed_out(
        OTA_POLICY_TOTAL_TIMEOUT_US - 1U,
        OTA_POLICY_NO_PROGRESS_TIMEOUT_US));
}

static void test_failure_reason_is_actionable_and_truncated_at_wire_bound(void)
{
    char failure[PROTOCOL_MAX_DIAGNOSTIC_LENGTH + 2U];
    memset(failure, '!', sizeof(failure));
    ota_policy_format_failure(failure,
                              PROTOCOL_MAX_DIAGNOSTIC_LENGTH + 1U,
                              "download", "ESP_ERR_NO_MEM");
    assert(strcmp(failure, "download: ESP_ERR_NO_MEM") == 0);
    assert(failure[PROTOCOL_MAX_DIAGNOSTIC_LENGTH + 1U] == '!');

    char detail[PROTOCOL_MAX_DIAGNOSTIC_LENGTH + 2U];
    memset(detail, 'x', sizeof(detail) - 1U);
    detail[sizeof(detail) - 1U] = '\0';
    ota_policy_format_failure(failure,
                              PROTOCOL_MAX_DIAGNOSTIC_LENGTH + 1U,
                              "verify", detail);
    assert(strlen(failure) == PROTOCOL_MAX_DIAGNOSTIC_LENGTH);
    assert(strncmp(failure, "verify: ", sizeof("verify: ") - 1U) == 0);
    assert(failure[PROTOCOL_MAX_DIAGNOSTIC_LENGTH] == '\0');
    assert(failure[PROTOCOL_MAX_DIAGNOSTIC_LENGTH + 1U] == '!');

    ota_policy_format_http_failure(failure, sizeof(failure), "check", 500U);
    assert(strcmp(failure, "check: HTTP 500") == 0);

    ota_policy_format_failure(failure, sizeof(failure), "begin", "ESP_FAIL");
    assert(strcmp(failure, "begin: ESP_FAIL") == 0);
}

static void test_failure_pack_is_lossless_at_field_boundaries(void)
{
    ota_failure_t failure = ota_policy_unpack_failure(
        ota_policy_pack_failure(OTA_FAILURE_STAGE_DOWNLOAD,
                                INT32_MIN, UINT16_MAX));
    assert(failure.stage == OTA_FAILURE_STAGE_DOWNLOAD);
    assert(failure.error == INT32_MIN);
    assert(failure.http_status == UINT16_MAX);

    failure = ota_policy_unpack_failure(
        ota_policy_pack_failure(OTA_FAILURE_STAGE_VERIFY,
                                INT32_MAX, 0U));
    assert(failure.stage == OTA_FAILURE_STAGE_VERIFY);
    assert(failure.error == INT32_MAX);
    assert(failure.http_status == 0U);

    failure = ota_policy_unpack_failure(0U);
    assert(failure.stage == OTA_FAILURE_STAGE_NONE);
    assert(failure.error == 0);
    assert(failure.http_status == 0U);

    assert(strcmp(ota_policy_failure_stage_name(OTA_FAILURE_STAGE_TASK),
                  "task") == 0);
    assert(strcmp(ota_policy_failure_stage_name(OTA_FAILURE_STAGE_CHECK),
                  "check") == 0);
    assert(strcmp(ota_policy_failure_stage_name(OTA_FAILURE_STAGE_BEGIN),
                  "begin") == 0);
    assert(strcmp(ota_policy_failure_stage_name(OTA_FAILURE_STAGE_DOWNLOAD),
                  "download") == 0);
    assert(strcmp(ota_policy_failure_stage_name(OTA_FAILURE_STAGE_VERIFY),
                  "verify") == 0);
}

int main(void)
{
    test_urls_use_the_server_origin_and_https();
    test_urls_reject_unsafe_or_truncated_inputs();
    test_metadata_is_bounded_and_tied_to_its_version();
    test_either_live_focus_state_defers_update();
    test_owner_wait_is_bounded_and_readiness_wins();
    test_day_scale_delays_convert_without_32_bit_overflow();
    test_download_deadlines_bound_stalls_and_slow_trickles();
    test_failure_reason_is_actionable_and_truncated_at_wire_bound();
    test_failure_pack_is_lossless_at_field_boundaries();
    puts("test_ota_policy: OK");
    return 0;
}
