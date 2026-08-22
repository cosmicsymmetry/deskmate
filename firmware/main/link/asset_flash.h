#pragma once

#include "core/asset_store.h"
#include "esp_err.h"

/* Finds the `assets` partition, formats it if unformatted, and opens the
 * store. Call once at boot, before the UI needs a font. */
esp_err_t asset_flash_init(void);

const asset_flash_io_t *asset_flash_io(void);
const asset_store_t *asset_flash_store(void);

/* Maps a committed record's blob region read-only. `out_ptr` stays valid
 * until the next compaction. */
esp_err_t asset_flash_map(const asset_record_t *record, const void **out_ptr);

esp_err_t asset_flash_write_blob(uint32_t blob_offset, const void *data,
                                 size_t length);
esp_err_t asset_flash_execute_compaction(const asset_move_t *moves, size_t count);
