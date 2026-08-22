#include "link/asset_flash.h"

#include <string.h>

#include "esp_check.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_partition.h"

/* 64 committed records is the capacity core/asset_store.c's own host tests
 * format and open against (docs/superpowers/plans/2026-08-22-deskmate-asset-
 * store.md's Task 2 fixtures assert record_capacity == 64). Deskmate ships a
 * handful of fonts and icon fonts, so this is ample headroom; it is a format
 * choice for a fresh partition, not something asset_flash.c decides per
 * asset, so it lives here rather than in core/. */
#define ASSET_FLASH_RECORD_CAPACITY 64U

/* firmware/partitions.csv: "assets, data, 0x40, , 6M,". 0x40 is the first
 * subtype value ESP-IDF's partition table format reserves for
 * application-defined data partitions (0x40-0xFE). */
#define ASSET_FLASH_PARTITION_SUBTYPE ((esp_partition_subtype_t)0x40)

static const char *TAG = "asset_flash";

/* Module state is pointers and a handful of small fixed-size fields, never
 * a buffer -- this repo lost OTA downloads once to ~105 bytes of static
 * internal DRAM (see firmware/version.txt history / board-notes), so every
 * buffer this file needs is allocated from PSRAM at the point of use
 * instead of living here. */
static const esp_partition_t *s_partition;
static asset_store_t s_store;
static esp_partition_mmap_handle_t *s_mmap_handles;
static uint32_t s_mmap_handle_count;
static uint32_t s_mmap_handle_capacity;

static int io_read(void *ctx, uint32_t offset, void *out, size_t length)
{
    (void)ctx;
    return esp_partition_read(s_partition, offset, out, length) == ESP_OK ? 0 : -1;
}

static int io_write(void *ctx, uint32_t offset, const void *data, size_t length)
{
    (void)ctx;
    /* esp_partition_write only clears bits -- it never erases -- which is
     * exactly the NOR semantics asset_store.h's asset_flash_io_t documents
     * its callers rely on for crash-safe commits and chunk writes. Do not
     * add read-modify-write or an implicit erase here; that would defeat
     * the property the whole on-flash format rests on. */
    return esp_partition_write(s_partition, offset, data, length) == ESP_OK ? 0 : -1;
}

static int io_erase(void *ctx, uint32_t offset, size_t length)
{
    (void)ctx;
    return esp_partition_erase_range(s_partition, offset, length) == ESP_OK ? 0 : -1;
}

/* Compile-time constant -- this lives in .rodata, not .bss. */
static const asset_flash_io_t s_io = {
    .read = io_read,
    .write = io_write,
    .erase = io_erase,
    .ctx = NULL,
};

static esp_err_t map_store_result(asset_store_result_t result)
{
    switch (result) {
    case ASSET_STORE_OK:
        return ESP_OK;
    case ASSET_STORE_ERR_ARGUMENT:
        return ESP_ERR_INVALID_ARG;
    case ASSET_STORE_ERR_IO:
        return ESP_FAIL;
    case ASSET_STORE_ERR_NOT_FORMATTED:
        return ESP_ERR_INVALID_STATE;
    case ASSET_STORE_ERR_CORRUPT:
        return ESP_ERR_INVALID_STATE;
    case ASSET_STORE_ERR_FULL:
        return ESP_ERR_NO_MEM;
    case ASSET_STORE_ERR_NOT_FOUND:
        return ESP_ERR_NOT_FOUND;
    default:
        return ESP_FAIL;
    }
}

esp_err_t asset_flash_init(void)
{
    ESP_RETURN_ON_FALSE(s_partition == NULL, ESP_ERR_INVALID_STATE, TAG,
                        "already initialized");

    const esp_partition_t *partition = esp_partition_find_first(
        ESP_PARTITION_TYPE_DATA, ASSET_FLASH_PARTITION_SUBTYPE, NULL);
    ESP_RETURN_ON_FALSE(partition != NULL, ESP_ERR_NOT_FOUND, TAG,
                        "assets partition not found");
    s_partition = partition;

    asset_store_result_t result =
        asset_store_open(&s_store, &s_io, (uint32_t)partition->size);
    if (result == ASSET_STORE_ERR_NOT_FORMATTED) {
        ESP_LOGI(TAG, "assets partition unformatted; formatting (%u records)",
                (unsigned)ASSET_FLASH_RECORD_CAPACITY);
        result = asset_store_format(&s_io, (uint32_t)partition->size,
                                    ASSET_FLASH_RECORD_CAPACITY);
        if (result != ASSET_STORE_OK) {
            s_partition = NULL;
            return map_store_result(result);
        }
        result = asset_store_open(&s_store, &s_io, (uint32_t)partition->size);
    }
    if (result != ASSET_STORE_OK) {
        s_partition = NULL;
        return map_store_result(result);
    }

    /* Sized to record_capacity: at most one outstanding mmap per record slot
     * can ever exist, since every mapped record came from a distinct
     * committed slot. That bounds the table without guessing at how many
     * faces font_registry happens to keep open. */
    s_mmap_handle_capacity = s_store.record_capacity;
    s_mmap_handles = heap_caps_malloc(
        (size_t)s_mmap_handle_capacity * sizeof(*s_mmap_handles), MALLOC_CAP_SPIRAM);
    if (s_mmap_handles == NULL) {
        s_partition = NULL;
        return ESP_ERR_NO_MEM;
    }
    s_mmap_handle_count = 0U;

    ESP_LOGI(TAG, "asset store open: %u records, %u blob bytes",
            (unsigned)s_store.record_capacity, (unsigned)s_store.blob_region_size);
    return ESP_OK;
}

const asset_flash_io_t *asset_flash_io(void)
{
    return &s_io;
}

const asset_store_t *asset_flash_store(void)
{
    return &s_store;
}

esp_err_t asset_flash_map(const asset_record_t *record, const void **out_ptr)
{
    ESP_RETURN_ON_FALSE(s_partition != NULL, ESP_ERR_INVALID_STATE, TAG,
                        "not initialized");
    ESP_RETURN_ON_FALSE(record != NULL && out_ptr != NULL, ESP_ERR_INVALID_ARG, TAG,
                        "bad argument");
    ESP_RETURN_ON_FALSE(record->length <= s_store.blob_region_size &&
                            record->offset <= s_store.blob_region_size - record->length,
                        ESP_ERR_INVALID_ARG, TAG, "record out of range");
    ESP_RETURN_ON_FALSE(s_mmap_handle_count < s_mmap_handle_capacity, ESP_ERR_NO_MEM,
                        TAG, "mmap handle table full");

    esp_partition_mmap_handle_t handle;
    esp_err_t err = esp_partition_mmap(
        s_partition, s_store.blob_region_offset + record->offset, record->length,
        ESP_PARTITION_MMAP_DATA, out_ptr, &handle);
    if (err != ESP_OK) {
        return err;
    }
    s_mmap_handles[s_mmap_handle_count++] = handle;
    return ESP_OK;
}

esp_err_t asset_flash_write_blob(uint32_t blob_offset, const void *data, size_t length)
{
    ESP_RETURN_ON_FALSE(s_partition != NULL, ESP_ERR_INVALID_STATE, TAG,
                        "not initialized");
    ESP_RETURN_ON_FALSE(data != NULL || length == 0U, ESP_ERR_INVALID_ARG, TAG,
                        "bad argument");
    ESP_RETURN_ON_FALSE(length <= s_store.blob_region_size &&
                            blob_offset <= s_store.blob_region_size - length,
                        ESP_ERR_INVALID_ARG, TAG, "write out of range");

    if (io_write(NULL, s_store.blob_region_offset + blob_offset, data, length) != 0) {
        return ESP_FAIL;
    }
    return ESP_OK;
}

/* Releases every mmap handle this module has outstanding. Called at the
 * start of compaction: asset_flash_map's own contract says a mapped pointer
 * "stays valid until the next compaction", and protocol_task's AssetRelease
 * handler already destroys the lv_font_t objects built on top of those
 * pointers (font_registry_reset()) before calling here -- but font_registry
 * has no ESP-IDF include and so cannot itself release the underlying
 * esp_partition_mmap handles. That is this file's job. */
static void unmap_all(void)
{
    for (uint32_t i = 0U; i < s_mmap_handle_count; i++) {
        esp_partition_munmap(s_mmap_handles[i]);
    }
    s_mmap_handle_count = 0U;
}

esp_err_t asset_flash_execute_compaction(const asset_move_t *moves, size_t count)
{
    ESP_RETURN_ON_FALSE(s_partition != NULL, ESP_ERR_INVALID_STATE, TAG,
                        "not initialized");
    ESP_RETURN_ON_FALSE(moves != NULL || count == 0U, ESP_ERR_INVALID_ARG, TAG,
                        "bad argument");

    unmap_all();

    /* core/asset_store.c's asset_store_plan_compaction visits committed
     * records in ascending blob offset and packs survivors forward starting
     * at 0, so for every emitted move to_offset <= from_offset, and a move's
     * destination interval never reaches into a later move's still-unread
     * source (each move's [to_offset, to_offset+length) ends at or before
     * the next move's to_offset, which is itself <= that move's from_offset).
     * That means moves[] is already sorted ascending by both from_offset and
     * to_offset in emission order -- verified by reading that loop, not
     * assumed here. This loop still checks the ascending-to_offset half of
     * that invariant rather than trusting it silently, since a violation
     * would mean this function's forward-pack copy is unsafe. */
    uint32_t total_length = 0U;
    uint32_t previous_to_offset = 0U;
    for (size_t i = 0U; i < count; i++) {
        const asset_move_t *move = &moves[i];
        if (move->record_index >= s_store.record_capacity) {
            return ESP_ERR_INVALID_ARG;
        }
        if (move->to_offset > move->from_offset) {
            return ESP_ERR_INVALID_STATE;
        }
        if (i > 0U && move->to_offset < previous_to_offset) {
            return ESP_ERR_INVALID_STATE;
        }
        if (move->length > s_store.blob_region_size ||
            move->from_offset > s_store.blob_region_size - move->length ||
            move->to_offset > s_store.blob_region_size - move->length) {
            return ESP_ERR_INVALID_ARG;
        }
        previous_to_offset = move->to_offset;
        total_length = move->to_offset + move->length;
    }

    esp_err_t err = ESP_OK;
    asset_record_t *kept = NULL;
    uint8_t *blob_buffer = NULL;

    /* Read every surviving record's current bytes, and every surviving
     * blob's current bytes, into PSRAM before erasing anything. Flash
     * writes here only ever clear bits (io_write), so packing a record's
     * offset down or reusing a byte range that used to belong to a dead
     * record requires an erase first -- and an erase must not run until
     * everything it would destroy has already been read. */
    if (count > 0U) {
        kept = heap_caps_malloc(count * sizeof(*kept), MALLOC_CAP_SPIRAM);
        if (kept == NULL) {
            return ESP_ERR_NO_MEM;
        }
        for (size_t i = 0U; i < count; i++) {
            uint8_t bytes[ASSET_RECORD_BYTES];
            uint32_t record_offset =
                ASSET_HEADER_BYTES + moves[i].record_index * ASSET_RECORD_BYTES;
            if (io_read(NULL, record_offset, bytes, sizeof bytes) != 0) {
                err = ESP_FAIL;
                goto done;
            }
            if (asset_store_record_decode(bytes, &kept[i]) != ASSET_STORE_OK) {
                err = ESP_ERR_INVALID_STATE;
                goto done;
            }
            kept[i].offset = moves[i].to_offset;
            kept[i].length = moves[i].length;
            kept[i].state = ASSET_STATE_COMMITTED;
        }
    }

    /* The staging buffer is exactly `total_length` -- the compacted blob
     * data is contiguous with no gaps between moves (each to_offset picks
     * up exactly where the previous move's ended), so one PSRAM buffer,
     * filled from each move's current location and written back as a
     * single run, replaces what would otherwise be a per-sector shuffle. */
    if (total_length > 0U) {
        blob_buffer = heap_caps_malloc(total_length, MALLOC_CAP_SPIRAM);
        if (blob_buffer == NULL) {
            err = ESP_ERR_NO_MEM;
            goto done;
        }
        for (size_t i = 0U; i < count; i++) {
            if (io_read(NULL, s_store.blob_region_offset + moves[i].from_offset,
                       blob_buffer + moves[i].to_offset, moves[i].length) != 0) {
                err = ESP_FAIL;
                goto done;
            }
        }
    }

    /* Header + record array: back up the header verbatim (this file has no
     * business knowing its field layout -- that is core/asset_store.c's
     * private encoding), erase the whole sector-aligned region ahead of the
     * blob region (blob_region_offset is always a round_up_to_sector()
     * result), restore the header byte-for-byte, then write only the
     * surviving records. Every other slot -- dead, still-uncommitted, or
     * simply unused -- stays erased, which is exactly "free" in this
     * format. */
    {
        uint8_t header_backup[ASSET_HEADER_BYTES];
        if (io_read(NULL, 0U, header_backup, sizeof header_backup) != 0) {
            err = ESP_FAIL;
            goto done;
        }
        if (io_erase(NULL, 0U, s_store.blob_region_offset) != 0) {
            err = ESP_FAIL;
            goto done;
        }
        if (io_write(NULL, 0U, header_backup, sizeof header_backup) != 0) {
            err = ESP_FAIL;
            goto done;
        }
        for (size_t i = 0U; i < count; i++) {
            uint8_t bytes[ASSET_RECORD_BYTES];
            asset_store_record_encode(&kept[i], bytes);
            uint32_t record_offset =
                ASSET_HEADER_BYTES + moves[i].record_index * ASSET_RECORD_BYTES;
            if (io_write(NULL, record_offset, bytes, sizeof bytes) != 0) {
                err = ESP_FAIL;
                goto done;
            }
        }
    }

    /* Blob region: erase the whole thing -- this reclaims both the dead
     * space between surviving assets and the true tail past the new,
     * smaller high-water mark in one aligned call -- then write the
     * compacted buffer back as a single run starting at offset 0. */
    if (io_erase(NULL, s_store.blob_region_offset, s_store.blob_region_size) != 0) {
        err = ESP_FAIL;
        goto done;
    }
    if (total_length > 0U) {
        if (io_write(NULL, s_store.blob_region_offset, blob_buffer, total_length) != 0) {
            err = ESP_FAIL;
            goto done;
        }
    }

done:
    heap_caps_free(kept);
    heap_caps_free(blob_buffer);
    return err;
}
