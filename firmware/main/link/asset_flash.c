#include "link/asset_flash.h"

#include <string.h>

#include "esp_check.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_partition.h"

// FONT_REGISTRY_MAX_OPEN_FACES: this file's only caller of asset_flash_map()
// is font_registry.c's resolver (via protocol_asset_resolver), one call per
// (digest, pixel_size) miss -- see ASSET_FLASH_MMAP_CAPACITY below for why
// that makes this the right bound, and why it is not the exact bound.
#include "ui/font_registry.h"

/* 64 committed records is the capacity core/asset_store.c's own host tests
 * format and open against (docs/superpowers/plans/2026-08-22-deskmate-asset-
 * store.md's Task 2 fixtures assert record_capacity == 64). Deskmate ships a
 * handful of fonts and icon fonts, so this is ample headroom; it is a format
 * choice for a fresh partition, not something asset_flash.c decides per
 * asset, so it lives here rather than in core/. */
#define ASSET_FLASH_RECORD_CAPACITY 64U

/* Outstanding esp_partition_mmap() handles this file will hold at once.
 * FONT_REGISTRY_MAX_OPEN_FACES, plus ONE. The +1 is not slack: it is the
 * transient overlap inside an evicting font_registry_acquire(), which
 * resolves -- and therefore maps -- before claim_slot() destroys its LRU
 * victim and unmaps that victim's blob. For the length of that window the
 * new mapping and the doomed one coexist, so a table sized to exactly
 * FONT_REGISTRY_MAX_OPEN_FACES refuses the ninth map with ESP_ERR_NO_MEM
 * before eviction can ever run. That is what a full registry did on this
 * device until 2026-08-23: acquire returned NULL with every slot unpinned,
 * LRU eviction was unreachable, and font_registry.h documented a contract
 * the hardware could not honour. Resolving before claiming is deliberate
 * (font_registry.c: "so an absent or non-font digest never costs a live
 * face"), so the table gives way, not the ordering. */
#define ASSET_FLASH_MMAP_CAPACITY (FONT_REGISTRY_MAX_OPEN_FACES + 1U)

_Static_assert(ASSET_FLASH_MMAP_CAPACITY > FONT_REGISTRY_MAX_OPEN_FACES,
               "the mmap table must exceed the registry's open-face bound: an "
               "evicting acquire maps its new face before unmapping its LRU "
               "victim, so the two coexist and an exact fit makes eviction "
               "unreachable");

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

// One outstanding esp_partition_mmap() call, and the pointer it returned --
// the pointer is what asset_flash_unmap() is given back, since that is the
// only handle its caller (font_registry.c, via the injected asset_release_fn)
// ever sees.
typedef struct {
    const void *ptr;
    esp_partition_mmap_handle_t handle;
} mmap_entry_t;

static mmap_entry_t *s_mmap_entries;
static uint32_t s_mmap_entry_count;
static uint32_t s_mmap_entry_capacity;

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

    /* No in-RAM asset_transfer_t survives a reboot, so any record still
     * UNCOMMITTED with a real digest at this point is a reservation
     * abandoned by a transfer that was interrupted before AssetCommit --
     * link drop, host crash, or power loss. Reclaim it now, or it is
     * counted as spoken-for by every future asset_store_reserve() forever,
     * with no path back to free space short of a reflash. */
    uint32_t reclaimed_count = 0U;
    result = asset_store_reclaim_boot_orphans(&s_store, &reclaimed_count);
    if (result != ASSET_STORE_OK) {
        s_partition = NULL;
        return map_store_result(result);
    }
    if (reclaimed_count > 0U) {
        ESP_LOGW(TAG, "reclaimed %u abandoned asset reservation(s) at boot",
                (unsigned)reclaimed_count);
    }

    /* NOT sized to record_capacity. That was this file's original reasoning
     * ("at most one outstanding mmap per record slot can ever exist") and it
     * is false: font_registry_acquire() misses per (digest, pixel_size), not
     * per record, so one committed record can be mapped again for every
     * distinct pixel size it is rendered at, and again after every LRU
     * eviction (whole-branch review finding 2). asset_flash_map() is only
     * ever called from font_registry.c's injected resolver, and
     * font_registry.c bounds its own open-face count to
     * FONT_REGISTRY_MAX_OPEN_FACES with LRU eviction -- so *that*, plus the
     * one transient mapping ASSET_FLASH_MMAP_CAPACITY explains, is the true
     * bound on outstanding mappings, as long as every eviction and every
     * font_registry_reset() releases its mapping via asset_flash_unmap()
     * (the asset_release_fn font_registry_init() is given below). It is not
     * record_capacity coincidentally being larger that keeps this safe; it
     * is font_registry.c's own LRU bound. */
    s_mmap_entry_capacity = ASSET_FLASH_MMAP_CAPACITY;
    s_mmap_entries = heap_caps_malloc(
        (size_t)s_mmap_entry_capacity * sizeof(*s_mmap_entries), MALLOC_CAP_SPIRAM);
    if (s_mmap_entries == NULL) {
        s_partition = NULL;
        return ESP_ERR_NO_MEM;
    }
    s_mmap_entry_count = 0U;

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
    ESP_RETURN_ON_FALSE(s_mmap_entry_count < s_mmap_entry_capacity, ESP_ERR_NO_MEM,
                        TAG, "mmap handle table full");

    esp_partition_mmap_handle_t handle;
    esp_err_t err = esp_partition_mmap(
        s_partition, s_store.blob_region_offset + record->offset, record->length,
        ESP_PARTITION_MMAP_DATA, out_ptr, &handle);
    if (err != ESP_OK) {
        return err;
    }
    s_mmap_entries[s_mmap_entry_count].ptr = *out_ptr;
    s_mmap_entries[s_mmap_entry_count].handle = handle;
    s_mmap_entry_count++;
    return ESP_OK;
}

void asset_flash_unmap(const void *ptr)
{
    if (ptr == NULL) {
        return;
    }
    for (uint32_t i = 0U; i < s_mmap_entry_count; i++) {
        if (s_mmap_entries[i].ptr != ptr) {
            continue;
        }
        esp_partition_munmap(s_mmap_entries[i].handle);
        // Swap-remove: mapping order carries no meaning once a mapping is
        // outstanding, so moving the last live entry into the freed slot
        // keeps this O(1) instead of shifting the whole array down.
        s_mmap_entry_count--;
        s_mmap_entries[i] = s_mmap_entries[s_mmap_entry_count];
        return;
    }
    // Not found: either already released, or invalidated out from under the
    // caller by a compaction's unmap_all(). Both are legitimate per this
    // function's documented contract, so this is not logged as an error.
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

/* Releases every mmap handle this module has outstanding, regardless of
 * whether font_registry.c already released its own via asset_flash_unmap().
 * Called at the start of compaction: asset_flash_map's own contract says a
 * mapped pointer "stays valid until the next compaction [or
 * asset_flash_unmap()]", and protocol_task's AssetRelease handler already
 * destroys the lv_font_t objects built on top of those pointers
 * (font_registry_reset()) before calling here -- but font_registry has no
 * ESP-IDF include and so cannot itself release the underlying
 * esp_partition_mmap handles. That is this file's job. */
static void unmap_all(void)
{
    for (uint32_t i = 0U; i < s_mmap_entry_count; i++) {
        esp_partition_munmap(s_mmap_entries[i].handle);
    }
    s_mmap_entry_count = 0U;
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
     * at 0, so for every emitted move to_offset <= from_offset, and moves
     * are gapless: to_offset picks up exactly where the previous move's
     * destination ended, with no unused byte in between. That means
     * moves[] is already sorted ascending by both from_offset and to_offset
     * in emission order -- verified by reading that loop, not assumed here.
     * This loop checks the full invariant (gapless, non-overlapping,
     * to_offset <= from_offset) rather than trusting it silently: a gap
     * would leave a hole in blob_buffer sized from uninitialized PSRAM
     * below, and any other violation would make the forward-pack copy
     * unsafe. */
    uint32_t total_length = 0U;
    for (size_t i = 0U; i < count; i++) {
        const asset_move_t *move = &moves[i];
        if (move->record_index >= s_store.record_capacity) {
            return ESP_ERR_INVALID_ARG;
        }
        if (move->to_offset > move->from_offset) {
            return ESP_ERR_INVALID_STATE;
        }
        if (move->to_offset != total_length) {
            /* Must equal the running end of every move packed so far --
             * catches both a gap and an overlap/out-of-order entry. */
            return ESP_ERR_INVALID_STATE;
        }
        if (move->length > s_store.blob_region_size ||
            move->from_offset > s_store.blob_region_size - move->length ||
            move->to_offset > s_store.blob_region_size - move->length) {
            return ESP_ERR_INVALID_ARG;
        }
        total_length = move->to_offset + move->length;
    }

    esp_err_t err = ESP_OK;
    asset_record_t *kept = NULL;
    uint8_t *blob_buffer = NULL;
    // Set true immediately before the first erase call below (not after it
    // succeeds -- esp_partition_erase_range can fail partway through and
    // still have physically erased the header's sector before the error).
    // Any failure from that point on leaves flash reading NOT_FORMATTED
    // (offset 0 erased, not "DMAS") until this function's own final header
    // write -- which is the crash-safety property the comment below the
    // erase calls documents -- but a live in-RAM s_store does not
    // automatically agree once that happens. Without this flag, a caller
    // that keeps going after this function returns an error would still see
    // committed()/find()/reserve() all succeed against a store the next
    // reboot's asset_flash_init() will discover has no header and silently
    // reformat, discarding anything committed since.
    bool past_point_of_no_return = false;

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

    /* Back up the header verbatim before erasing anything -- this file has
     * no business knowing its field layout, that is core/asset_store.c's
     * private encoding, so this is a byte-for-byte copy, not a re-derive. */
    uint8_t header_backup[ASSET_HEADER_BYTES];
    if (io_read(NULL, 0U, header_backup, sizeof header_backup) != 0) {
        err = ESP_FAIL;
        goto done;
    }

    /* Erase both regions before writing either one. Everything either erase
     * would destroy has already been read into PSRAM above (kept[] and
     * blob_buffer), so nothing here can lose unread data. The header's
     * sector is included in this erase -- blob_region_offset is always a
     * round_up_to_sector() result, so [0, blob_region_offset) is exactly
     * the header + record array's sector-aligned span -- which is *why*
     * the header write below has to be the last thing this function does:
     * from this point until that final write, offset 0 reads erased
     * (0xFF), not "DMAS", so asset_store_open() cannot mistake a partially
     * written compaction for a valid, committed store. */
    past_point_of_no_return = true;
    if (io_erase(NULL, 0U, s_store.blob_region_offset) != 0) {
        err = ESP_FAIL;
        goto done;
    }
    if (io_erase(NULL, s_store.blob_region_offset, s_store.blob_region_size) != 0) {
        err = ESP_FAIL;
        goto done;
    }

    /* Write the blob bytes, then the record array (offset ASSET_HEADER_BYTES
     * onward -- this never touches offset 0..ASSET_HEADER_BYTES, so it
     * cannot race the header's commit role below), then -- last -- the
     * header.
     *
     * That ordering is the whole crash-safety argument: a power loss at any
     * point before the final header write leaves offset 0 erased, so
     * asset_store_open() returns ASSET_STORE_ERR_NOT_FORMATTED and
     * asset_flash_init() reformats to an empty, self-consistent store that
     * the host re-syncs from scratch. The alternative this replaced --
     * writing the record array (with offsets already repointed at the new,
     * not-yet-written blob layout) before the blob region was rewritten --
     * left a *valid-looking* store whose committed records pointed at
     * stale, pre-compaction bytes: asset_store_open() has no way to
     * cross-check a record's offset against blob content, so nothing would
     * ever detect it. Do not reorder the header write earlier than last;
     * doing so reintroduces that silent corruption. */
    if (total_length > 0U) {
        if (io_write(NULL, s_store.blob_region_offset, blob_buffer, total_length) != 0) {
            err = ESP_FAIL;
            goto done;
        }
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
    /* Commit point. esp_partition_write has no alignment requirement for an
     * unencrypted partition (the 16-byte offset/length rule only applies
     * when the partition's `encrypted` flag is set -- `assets` in
     * partitions.csv carries no such flag), so this 32-byte write at
     * offset 0 needs no special handling. */
    if (io_write(NULL, 0U, header_backup, sizeof header_backup) != 0) {
        err = ESP_FAIL;
        goto done;
    }

done:
    heap_caps_free(kept);
    heap_caps_free(blob_buffer);
    if (err != ESP_OK && past_point_of_no_return) {
        // The header's sector is gone from flash and this function is not
        // going to restore it. Degrade exactly the way a failed
        // asset_flash_init() already degrades (see protocol_task.c's own
        // comment on that): zeroing s_store makes every asset_store_*() call
        // fail cleanly with ASSET_STORE_ERR_ARGUMENT (they all check
        // store->io == NULL first), and clearing s_partition makes this
        // file's own ESP_RETURN_ON_FALSE(s_partition != NULL, ...) guards
        // refuse every later asset_flash_map()/write_blob()/
        // execute_compaction() call with ESP_ERR_INVALID_STATE instead of
        // touching a store that flash no longer agrees exists. The loss is
        // then visible immediately -- every asset operation starts failing
        // -- rather than silently discovered (and silently reformatted
        // away) only at the next boot.
        ESP_LOGE(TAG, "compaction failed past the point of no return (%s); "
                "asset store invalidated until reboot",
                esp_err_to_name(err));
        memset(&s_store, 0, sizeof s_store);
        s_partition = NULL;
    }
    return err;
}
