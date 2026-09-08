# Runtime Asset Store Implementation Plan (Plugins, Stage 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A font pushed from the host is stored in the device's 6 MB `assets`
partition, survives reboot and OTA, and renders text at any pixel size — lifting the
four-baked-size ceiling.

**Architecture:** A content-addressed, append-only store in the existing `assets`
partition. Commit is a single flash bit-flip (`0xFF` → `0x01`), so an interrupted
transfer is reclaimable rather than corrupting. All store logic lives in
`firmware/main/core/` behind a flash-IO vtable, so it is host-testable as plain C with
a RAM-backed fake. `esp_partition_mmap()` hands the stored TTF bytes to
`lv_tiny_ttf_create_data_ex()` in place, so font data costs no PSRAM.

**Tech Stack:** ESP-IDF 5.x / C11, LVGL 9 (`lv_tiny_ttf`, vendored), TinyCBOR,
Rust (companion workspace: `protocol`, `app-core`, `server`, `lvgl-sim`).

**Spec:** `docs/superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md`

This plan covers **stage 1 only**. Stage 2 (the scene renderer) gets its own plan,
written at this stage's exit, per the spec's §7.

## Global Constraints

- **Every allocation on this path comes from PSRAM heap.** No new static/`.bss`
  objects in firmware. Spec §9: ~105 bytes of static internal DRAM once broke OTA
  downloads entirely with every test green.
- **`.bss` delta is measured and recorded.** Task 1 takes the baseline; Task 13
  compares.
- **An OTA download must be verified on the physical board before this stage exits.**
  Green tests are not evidence. Spec §9.
- `firmware/main/core/` stays free of ESP-IDF includes and compiles as plain C11 under
  `-Wall -Wextra -Werror -std=c11`.
- All bytes from the host are untrusted: bound every length and count, reject malformed
  input, never reboot on bad input.
- Protocol stays **v1**, additive only. `PROTOCOL_CURRENT_CAPABILITIES` becomes
  **`235`** (today's `203` plus bit 5, `+32`), pinned by a test in both languages.
- Guard LVGL calls made outside LVGL callbacks with
  `lvgl_port_lock()`/`lvgl_port_unlock()`.
- Conventional commit prefixes (`feat:`, `fix:`, `test:`, `docs:`, `chore:`).
- Do not touch the working tree's unrelated in-progress files (`firmware/version.txt`,
  `docs/hardware/webcam-harness.md`, `tools/hwcam/record.sh`, `firmware/build-diag/`).

## File Structure

| File | Responsibility |
| --- | --- |
| `firmware/main/core/asset_store.h/.c` | Directory format, record codec, lookup, allocation, GC planning. No ESP-IDF. |
| `firmware/main/core/asset_transfer.h/.c` | Resumable chunk state machine. No ESP-IDF. |
| `firmware/main/link/asset_flash.c` | Thin `esp_partition` + `esp_partition_mmap` implementation of the flash-IO vtable. |
| `firmware/main/ui/font_registry.h/.c` | digest+size → `lv_font_t *`, LRU of open faces, PSRAM allocation. |
| `firmware/main/core/protocol_message.h/.c` | Message types 15–18, `Ack` registry, capability bit 5. |
| `firmware/host_tests/test_asset_store.c` | Store unit tests against a RAM-backed fake flash. |
| `firmware/host_tests/test_asset_transfer.c` | Transfer state machine unit tests. |
| `companion/crates/protocol/src/message.rs` | Rust encode/decode for 15–18; `CURRENT_CAPABILITIES` = 235. |
| `companion/crates/app-core/src/config.rs` | Schema v5 asset variants, validation, migration. |
| `companion/crates/server/src/asset_sync.rs` | Server-side transfer driver over the `RuntimeDevice` seam. |
| `companion/crates/lvgl-sim/src/assets.rs` | Simulator asset shim — resolves a digest to the same bytes the device gets. |

The core/link split is deliberate and load-bearing: it is what makes the store
host-testable, and it matches the repo's existing rule that hardware-independent
firmware logic lives under `core/`.

---

### Task 1: Baseline measurement (no code)

This task exists because a `.bss` delta with no baseline is not a measurement, and
because spec §9 requires proving OTA worked *before* the change in order to attribute
any later breakage.

**Files:**
- Modify: `docs/hardware/board-notes.md` (append one section)

**Interfaces:**
- Consumes: nothing.
- Produces: a recorded baseline `.bss`/DIRAM figure and a confirmed-working OTA
  download on the current build, referenced by Task 13.

- [ ] **Step 1: Build clean and capture the size report**

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware fullclean
idf.py -C firmware build
idf.py -C firmware size > /tmp/size-baseline.txt
idf.py -C firmware size-components > /tmp/size-components-baseline.txt
cat /tmp/size-baseline.txt
```

Record the DIRAM `.bss` total and the "remaining" figure verbatim. The spec cites
219307/341760 and IRAM 100% full from an earlier build; confirm the current numbers
rather than reusing those.

- [ ] **Step 2: Flash and verify an OTA download on the board**

Flash the current build, then publish a build under a *new* version string and confirm
the device downloads and installs it. Note the two standing traps: a flashed build is
reverted within a minute unless `DESKMATE_FIRMWARE_VERSION` matches the catalog, and a
version string that has already failed is refused forever.

Expected: `ota` transitions `checking` → `downloading` → installs → reboots, and
`last_ota_error` is absent in `GET /v1/devices/{id}`.

- [ ] **Step 3: Record the baseline in board notes**

Append a section titled "Asset store baseline — 2026-08-22" with the `.bss` figure, the
IRAM figure, the firmware version string used, and the observed OTA result. State
plainly that this is a pre-change baseline.

- [ ] **Step 4: Commit**

```bash
git add docs/hardware/board-notes.md
git commit -m "docs: record pre-asset-store size and OTA baseline"
```

---

### Task 2: Asset store — header and record codec

**Files:**
- Create: `firmware/main/core/asset_store.h`, `firmware/main/core/asset_store.c`
- Create: `firmware/host_tests/test_asset_store.c`
- Modify: `firmware/host_tests/Makefile`, `firmware/main/CMakeLists.txt`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `asset_flash_io_t` — the vtable later tasks implement against.
  - `ASSET_DIGEST_BYTES` (32), `ASSET_RECORD_BYTES` (64), `ASSET_HEADER_BYTES` (32).
  - `asset_store_result_t`, `asset_record_t`.
  - `asset_store_format()`, `asset_store_open()`, `asset_store_record_decode()`,
    `asset_store_record_encode()`.

**On-flash layout.** A 32-byte header, then a fixed record array, then the blob region.
`state` uses erased flash (`0xFF`) as "uncommitted" and `0x01` as "committed", so commit
is a 1→0 bit write needing no sector erase. An interrupted transfer therefore leaves an
uncommitted record that GC reclaims, and can never present as a valid asset.

There is **no refcount on flash.** Reference counting is the host's job; the device
holds a set and is told which digests survive (`AssetRelease`, Task 5). This removes
in-place record mutation entirely, which is what makes the format crash-safe.

- [ ] **Step 1: Write the failing test**

Create `firmware/host_tests/test_asset_store.c`:

```c
#include <assert.h>
#include <string.h>

#include "core/asset_store.h"

/* RAM-backed fake flash: NOR semantics -- writes may only clear bits, and
 * erase restores 0xFF. Modelling that faithfully is the point; a plain memcpy
 * fake would hide the commit-bit trick this format depends on. */
#define FAKE_SIZE (64U * 1024U)
static uint8_t g_flash[FAKE_SIZE];

static int fake_read(void *ctx, uint32_t offset, void *out, size_t length)
{
    (void)ctx;
    if ((size_t)offset + length > FAKE_SIZE) return -1;
    memcpy(out, g_flash + offset, length);
    return 0;
}

static int fake_write(void *ctx, uint32_t offset, const void *data, size_t length)
{
    (void)ctx;
    if ((size_t)offset + length > FAKE_SIZE) return -1;
    const uint8_t *src = data;
    for (size_t i = 0; i < length; i++) {
        g_flash[offset + i] &= src[i]; /* NOR: bits only go 1 -> 0 */
    }
    return 0;
}

static int fake_erase(void *ctx, uint32_t offset, size_t length)
{
    (void)ctx;
    if ((size_t)offset + length > FAKE_SIZE) return -1;
    memset(g_flash + offset, 0xFF, length);
    return 0;
}

static asset_flash_io_t fake_io(void)
{
    asset_flash_io_t io = {
        .read = fake_read, .write = fake_write, .erase = fake_erase, .ctx = NULL,
    };
    return io;
}

static void test_format_then_open_roundtrips(void)
{
    memset(g_flash, 0x00, sizeof g_flash); /* deliberately not erased */
    asset_flash_io_t io = fake_io();
    asset_store_t store;

    assert(asset_store_format(&io, FAKE_SIZE, 64U) == ASSET_STORE_OK);
    assert(asset_store_open(&store, &io, FAKE_SIZE) == ASSET_STORE_OK);
    assert(store.record_capacity == 64U);
    assert(store.blob_region_size > 0U);
}

static void test_open_rejects_bad_magic(void)
{
    memset(g_flash, 0xFF, sizeof g_flash);
    asset_flash_io_t io = fake_io();
    asset_store_t store;

    assert(asset_store_open(&store, &io, FAKE_SIZE) == ASSET_STORE_ERR_NOT_FORMATTED);
}

static void test_record_encode_decode_roundtrips(void)
{
    asset_record_t in = { .offset = 4096U, .length = 1234U, .kind = ASSET_KIND_FONT,
                          .state = ASSET_STATE_COMMITTED };
    memset(in.digest, 0xAB, ASSET_DIGEST_BYTES);
    uint8_t bytes[ASSET_RECORD_BYTES];
    asset_record_t out;

    asset_store_record_encode(&in, bytes);
    assert(asset_store_record_decode(bytes, &out) == ASSET_STORE_OK);
    assert(out.offset == in.offset);
    assert(out.length == in.length);
    assert(out.kind == in.kind);
    assert(out.state == in.state);
    assert(memcmp(out.digest, in.digest, ASSET_DIGEST_BYTES) == 0);
}

static void test_record_decode_rejects_unknown_kind(void)
{
    uint8_t bytes[ASSET_RECORD_BYTES];
    memset(bytes, 0, sizeof bytes);
    bytes[ASSET_DIGEST_BYTES + 8U] = 0x7FU; /* kind byte, not a defined kind */
    bytes[ASSET_DIGEST_BYTES + 9U] = ASSET_STATE_COMMITTED;
    asset_record_t out;

    assert(asset_store_record_decode(bytes, &out) == ASSET_STORE_ERR_CORRUPT);
}

int main(void)
{
    test_format_then_open_roundtrips();
    test_open_rejects_bad_magic();
    test_record_encode_decode_roundtrips();
    test_record_decode_rejects_unknown_kind();
    return 0;
}
```

- [ ] **Step 2: Run it to confirm it fails**

```sh
make -C firmware/host_tests test_asset_store
```

Expected: FAIL — `core/asset_store.h: No such file or directory`.

- [ ] **Step 3: Write the header**

Create `firmware/main/core/asset_store.h`:

```c
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define ASSET_DIGEST_BYTES 32U
#define ASSET_HEADER_BYTES 32U
#define ASSET_RECORD_BYTES 64U
#define ASSET_STORE_MAGIC "DMAS"
#define ASSET_STORE_FORMAT_VERSION 1U

/* One asset may not exceed 1 MiB. Defined here, in the module both
 * asset_transfer.h and protocol_message.h already depend on, so the wire
 * bound and the transfer bound cannot drift apart. */
#define ASSET_MAX_BYTES (1024U * 1024U)

/* Erased NOR flash reads 0xFF. Commit clears bits to 0x01, which needs no
 * sector erase, so an interrupted transfer leaves an uncommitted record
 * rather than a corrupt one. */
#define ASSET_STATE_UNCOMMITTED 0xFFU
#define ASSET_STATE_COMMITTED 0x01U
#define ASSET_STATE_DEAD 0x00U

typedef enum {
    ASSET_KIND_FONT = 1,
    ASSET_KIND_ICON_FONT = 2,
    ASSET_KIND_IMAGE = 3,
} asset_kind_t;

typedef enum {
    ASSET_STORE_OK = 0,
    ASSET_STORE_ERR_ARGUMENT,
    ASSET_STORE_ERR_IO,
    ASSET_STORE_ERR_NOT_FORMATTED,
    ASSET_STORE_ERR_CORRUPT,
    ASSET_STORE_ERR_FULL,
    ASSET_STORE_ERR_NOT_FOUND,
} asset_store_result_t;

/* Flash abstraction. `write` must have NOR semantics (bits clear only);
 * `erase` restores 0xFF over whole sectors. Keeping this a vtable is what
 * lets the whole store be host-tested without ESP-IDF. */
typedef struct {
    int (*read)(void *ctx, uint32_t offset, void *out, size_t length);
    int (*write)(void *ctx, uint32_t offset, const void *data, size_t length);
    int (*erase)(void *ctx, uint32_t offset, size_t length);
    void *ctx;
} asset_flash_io_t;

typedef struct {
    uint8_t digest[ASSET_DIGEST_BYTES];
    uint32_t offset; /* relative to blob_region_offset */
    uint32_t length;
    uint8_t kind;
    uint8_t state;
} asset_record_t;

typedef struct {
    const asset_flash_io_t *io;
    uint32_t partition_size;
    uint32_t record_capacity;
    uint32_t blob_region_offset;
    uint32_t blob_region_size;
} asset_store_t;

asset_store_result_t asset_store_format(const asset_flash_io_t *io,
                                        uint32_t partition_size,
                                        uint32_t record_capacity);
asset_store_result_t asset_store_open(asset_store_t *store,
                                      const asset_flash_io_t *io,
                                      uint32_t partition_size);
void asset_store_record_encode(const asset_record_t *record, uint8_t *out);
asset_store_result_t asset_store_record_decode(const uint8_t *bytes,
                                               asset_record_t *out);
```

- [ ] **Step 4: Write the implementation**

Create `firmware/main/core/asset_store.c` implementing exactly those four functions.
Required behaviour, all covered by Step 1's tests:

- `asset_store_format` erases the whole partition, then writes the header:
  magic `"DMAS"` (4), format version LE `u32`, record capacity LE `u32`,
  blob region offset LE `u32` (= `ASSET_HEADER_BYTES + record_capacity *
  ASSET_RECORD_BYTES`, rounded up to 4096), blob region size LE `u32`, then
  reserved bytes left erased.
- `asset_store_open` reads the header, rejects a wrong magic or format version with
  `ASSET_STORE_ERR_NOT_FORMATTED`, rejects a blob region that does not fit inside
  `partition_size` with `ASSET_STORE_ERR_CORRUPT`, and otherwise populates `store`.
- `asset_store_record_encode` writes digest (32), offset LE `u32`, length LE `u32`,
  kind (1), state (1), and leaves the remaining 22 bytes untouched by writing `0xFF`.
- `asset_store_record_decode` rejects any `kind` outside `asset_kind_t` and any `state`
  outside the three defined values with `ASSET_STORE_ERR_CORRUPT`.

Use explicit little-endian byte assembly, as `dev_capture.c`'s `write_u32_le` does — do
not memcpy structs.

- [ ] **Step 5: Register the module in both build systems**

In `firmware/host_tests/Makefile`, add `test_asset_store` to the `test:` dependency
list and to the run list, and add the rule:

```make
test_asset_store: test_asset_store.c ../main/core/asset_store.c
	$(CC) $(CFLAGS) -I../main -o $@ $^
```

In `firmware/main/CMakeLists.txt`, add `"core/asset_store.c"` to `SRCS`, keeping the
existing alphabetical grouping.

- [ ] **Step 6: Run the tests**

```sh
make -C firmware/host_tests clean test
```

Expected: PASS, including every pre-existing test.

- [ ] **Step 7: Commit**

```bash
git add firmware/main/core/asset_store.h firmware/main/core/asset_store.c \
        firmware/host_tests/test_asset_store.c firmware/host_tests/Makefile \
        firmware/main/CMakeLists.txt
git commit -m "feat: add the asset store's on-flash header and record codec"
```

---

### Task 3: Asset store — lookup, reservation, commit, and compaction planning

**Files:**
- Modify: `firmware/main/core/asset_store.h`, `firmware/main/core/asset_store.c`
- Modify: `firmware/host_tests/test_asset_store.c`

**Interfaces:**
- Consumes: Task 2's `asset_store_t`, `asset_record_t`, `asset_flash_io_t`.
- Produces:
  - `asset_store_find(store, digest, out_record, out_index)`
  - `asset_store_reserve(store, digest, kind, length, out_index, out_blob_offset)`
  - `asset_store_commit(store, index)`
  - `asset_store_mark_dead(store, index)`
  - `asset_store_stats(store, out_stats)` → `asset_store_stats_t`
  - `asset_store_plan_compaction(store, keep, keep_count, moves, moves_capacity, out_move_count)`
    → `asset_move_t`

Compaction is split deliberately: **planning is a pure function in `core/`** (host
tested here) and **byte-moving lives in the flash layer** (Task 7). That keeps the
policy testable without ESP-IDF and leaves the link layer with no decisions to make.

- [ ] **Step 1: Write the failing tests**

Append to `firmware/host_tests/test_asset_store.c`:

```c
static asset_store_t formatted_store(asset_flash_io_t *io)
{
    memset(g_flash, 0x00, sizeof g_flash);
    *io = fake_io();
    asset_store_t store;
    assert(asset_store_format(io, FAKE_SIZE, 64U) == ASSET_STORE_OK);
    assert(asset_store_open(&store, io, FAKE_SIZE) == ASSET_STORE_OK);
    return store;
}

static void digest_of(uint8_t seed, uint8_t *out)
{
    memset(out, seed, ASSET_DIGEST_BYTES);
}

static void test_uncommitted_reservation_is_not_findable(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x11, digest);
    uint32_t index = 0U, blob = 0U;

    assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT, 100U, &index, &blob)
           == ASSET_STORE_OK);
    /* This is the crash-safety property: a transfer interrupted before commit
     * must never surface as a usable asset. */
    assert(asset_store_find(&store, digest, NULL, NULL) == ASSET_STORE_ERR_NOT_FOUND);

    assert(asset_store_commit(&store, index) == ASSET_STORE_OK);
    assert(asset_store_find(&store, digest, NULL, NULL) == ASSET_STORE_OK);
}

static void test_reserve_rejects_blob_overflow(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x22, digest);
    uint32_t index = 0U, blob = 0U;

    assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT,
                               store.blob_region_size + 1U, &index, &blob)
           == ASSET_STORE_ERR_FULL);
}

static void test_committed_record_is_findable_by_digest(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x33, digest);
    uint32_t index = 0U, blob = 0U;

    assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT, 64U, &index, &blob)
           == ASSET_STORE_OK);
    assert(asset_store_commit(&store, index) == ASSET_STORE_OK);
    /* `reserve` does NOT deduplicate; the inventory protocol lives in Task 5's
     * AssetBegin -> already_present handler. This only pins that a committed
     * record is findable by its digest. */
    assert(asset_store_find(&store, digest, NULL, NULL) == ASSET_STORE_OK);
}

static void test_compaction_plan_drops_unreferenced_and_packs(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t keep_digest[ASSET_DIGEST_BYTES];
    uint8_t drop_digest[ASSET_DIGEST_BYTES];
    digest_of(0xAA, keep_digest);
    digest_of(0xBB, drop_digest);
    uint32_t index = 0U, blob = 0U;

    assert(asset_store_reserve(&store, drop_digest, ASSET_KIND_FONT, 4096U,
                               &index, &blob) == ASSET_STORE_OK);
    assert(asset_store_commit(&store, index) == ASSET_STORE_OK);
    assert(asset_store_reserve(&store, keep_digest, ASSET_KIND_FONT, 512U,
                               &index, &blob) == ASSET_STORE_OK);
    assert(asset_store_commit(&store, index) == ASSET_STORE_OK);

    const uint8_t *keep[1] = { keep_digest };
    asset_move_t moves[8];
    size_t move_count = 0U;
    assert(asset_store_plan_compaction(&store, keep, 1U, moves, 8U, &move_count)
           == ASSET_STORE_OK);

    assert(move_count == 1U);
    assert(moves[0].length == 512U);
    assert(moves[0].to_offset == 0U); /* survivor packs down to the region start */
    assert(moves[0].from_offset == 4096U);
}

static void test_compaction_plan_reports_capacity_exhaustion(void)
{
    asset_flash_io_t io;
    asset_store_t store = formatted_store(&io);
    uint8_t digest[ASSET_DIGEST_BYTES];
    const uint8_t *keep[2];
    uint8_t d0[ASSET_DIGEST_BYTES], d1[ASSET_DIGEST_BYTES];
    uint32_t index = 0U, blob = 0U;

    digest_of(0xC0, d0);
    digest_of(0xC1, d1);
    keep[0] = d0;
    keep[1] = d1;
    for (uint8_t i = 0; i < 2; i++) {
        digest_of((uint8_t)(0xC0 + i), digest);
        assert(asset_store_reserve(&store, digest, ASSET_KIND_FONT, 256U,
                                   &index, &blob) == ASSET_STORE_OK);
        assert(asset_store_commit(&store, index) == ASSET_STORE_OK);
    }

    asset_move_t moves[1];
    size_t move_count = 0U;
    /* A caller-supplied buffer that is too small must be an error, never a
     * silent truncation that would delete surviving assets. */
    assert(asset_store_plan_compaction(&store, keep, 2U, moves, 1U, &move_count)
           == ASSET_STORE_ERR_FULL);
}
```

Add all five to `main()`.

- [ ] **Step 2: Run to confirm failure**

```sh
make -C firmware/host_tests test_asset_store
```

Expected: FAIL — `asset_store_find` and friends undeclared.

- [ ] **Step 3: Extend the header**

Append to `firmware/main/core/asset_store.h`:

```c
typedef struct {
    uint32_t committed_count;
    uint32_t used_blob_bytes;
    uint32_t free_blob_bytes;
    uint32_t reclaimable_blob_bytes;
} asset_store_stats_t;

typedef struct {
    uint32_t from_offset;
    uint32_t to_offset;
    uint32_t length;
    uint32_t record_index;
} asset_move_t;

/* `out_record` and `out_index` may be NULL when the caller only needs presence. */
asset_store_result_t asset_store_find(const asset_store_t *store,
                                      const uint8_t *digest,
                                      asset_record_t *out_record,
                                      uint32_t *out_index);
asset_store_result_t asset_store_reserve(const asset_store_t *store,
                                         const uint8_t *digest, uint8_t kind,
                                         uint32_t length, uint32_t *out_index,
                                         uint32_t *out_blob_offset);
asset_store_result_t asset_store_commit(const asset_store_t *store, uint32_t index);
asset_store_result_t asset_store_mark_dead(const asset_store_t *store, uint32_t index);
asset_store_result_t asset_store_stats(const asset_store_t *store,
                                       asset_store_stats_t *out_stats);
asset_store_result_t asset_store_plan_compaction(const asset_store_t *store,
                                                 const uint8_t *const *keep,
                                                 size_t keep_count,
                                                 asset_move_t *moves,
                                                 size_t moves_capacity,
                                                 size_t *out_move_count);
```

- [ ] **Step 4: Implement**

Required behaviour:

- `asset_store_find` scans records, matching only `ASSET_STATE_COMMITTED` entries whose
  digest compares equal. Returns `ASSET_STORE_ERR_NOT_FOUND` otherwise.
- `asset_store_reserve` places the blob at the current high-water mark — the maximum
  `offset + length` across **committed, uncommitted, and dead** records. Dead records
  must count: `asset_store_mark_dead` only clears a state byte, leaving every blob byte
  physically present and un-erased, and NOR writes clear bits only. Excluding them lets
  the next reservation land on a dead blob and AND itself into garbage, which commits
  and reads back as a valid asset. Space is reclaimable only by compaction. It takes the
  first record
  slot whose state is `ASSET_STATE_UNCOMMITTED` and whose digest bytes are all `0xFF`,
  and writes the record with `state` left erased. Returns `ASSET_STORE_ERR_FULL` when
  either the blob region or the record array cannot fit the request.
- `asset_store_commit` writes the single byte `ASSET_STATE_COMMITTED` at the record's
  state offset. No erase, no read-modify-write.
- `asset_store_mark_dead` writes `ASSET_STATE_DEAD` (`0x00`) — also a pure bit-clear.
- `asset_store_stats` sums committed lengths for `used_blob_bytes` and dead lengths for
  `reclaimable_blob_bytes`. **`free_blob_bytes` must come from the same high-water
  helper `asset_store_reserve` uses**, so the figure equals what a reservation will
  actually grant. Computing it as a plain region remainder makes an in-flight
  (uncommitted) record invisible, and Task 9 ships these numbers to the host in
  `StatusResponse` key 31 — an interrupted transfer would then advertise space the
  device refuses to allocate. used + reclaimable + free need not sum to the region
  while a transfer is in flight; that is deliberate.
- `asset_store_plan_compaction` walks committed records in ascending `offset`, emits a
  move for each digest present in `keep`, assigning `to_offset` as a running total from
  zero. Returns `ASSET_STORE_ERR_FULL` if `moves_capacity` is insufficient — never
  truncates. A record whose `from_offset` equals its `to_offset` is still emitted, so
  the caller can treat the plan as authoritative.

- [ ] **Step 5: Run the tests**

```sh
make -C firmware/host_tests clean test
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add firmware/main/core/asset_store.h firmware/main/core/asset_store.c \
        firmware/host_tests/test_asset_store.c
git commit -m "feat: add asset lookup, reservation, commit, and compaction planning"
```

---

### Task 4: Resumable transfer state machine

**Files:**
- Create: `firmware/main/core/asset_transfer.h`, `firmware/main/core/asset_transfer.c`
- Create: `firmware/host_tests/test_asset_transfer.c`
- Modify: `firmware/host_tests/Makefile`, `firmware/main/CMakeLists.txt`

**Interfaces:**
- Consumes: Task 2's `ASSET_DIGEST_BYTES`, `asset_kind_t`.
- Produces: `asset_transfer_t`, `asset_transfer_result_t`, `asset_transfer_begin()`,
  `asset_transfer_accept_chunk()`, `asset_transfer_is_complete()`,
  `asset_transfer_abort()`, `asset_transfer_resume_offset()`.

This is what makes spec §4's "resumable and preemptible" real. Chunks are strictly
in-order, so resume state is a single offset. A **repeat of the chunk just accepted is
a no-op, not an error** — that is precisely the lost-`Ack` case, and treating it as an
error would make every dropped acknowledgement fatal.

- [ ] **Step 1: Write the failing test**

Create `firmware/host_tests/test_asset_transfer.c`:

```c
#include <assert.h>
#include <string.h>

#include "core/asset_transfer.h"

static void digest_of(uint8_t seed, uint8_t *out)
{
    memset(out, seed, ASSET_DIGEST_BYTES);
}

static void test_in_order_chunks_complete_the_transfer(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x01, digest);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 300U, false)
           == ASSET_TRANSFER_OK);
    assert(!asset_transfer_is_complete(&t));
    assert(asset_transfer_accept_chunk(&t, digest, 0U, 200U) == ASSET_TRANSFER_OK);
    assert(asset_transfer_resume_offset(&t) == 200U);
    assert(asset_transfer_accept_chunk(&t, digest, 200U, 100U) == ASSET_TRANSFER_OK);
    assert(asset_transfer_is_complete(&t));
}

static void test_repeated_chunk_is_idempotent(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x02, digest);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 300U, false)
           == ASSET_TRANSFER_OK);
    assert(asset_transfer_accept_chunk(&t, digest, 0U, 200U) == ASSET_TRANSFER_OK);
    /* The host resent because our Ack was lost. Advancing twice would corrupt
     * the blob; erroring would make a dropped Ack fatal. Neither is acceptable. */
    assert(asset_transfer_accept_chunk(&t, digest, 0U, 200U)
           == ASSET_TRANSFER_DUPLICATE);
    assert(asset_transfer_resume_offset(&t) == 200U);
}

static void test_out_of_order_chunk_is_rejected(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x03, digest);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 300U, false)
           == ASSET_TRANSFER_OK);
    assert(asset_transfer_accept_chunk(&t, digest, 100U, 50U)
           == ASSET_TRANSFER_ERR_OFFSET);
}

static void test_chunk_for_another_digest_is_rejected(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES], other[ASSET_DIGEST_BYTES];
    digest_of(0x04, digest);
    digest_of(0x05, other);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 300U, false)
           == ASSET_TRANSFER_OK);
    assert(asset_transfer_accept_chunk(&t, other, 0U, 10U)
           == ASSET_TRANSFER_ERR_DIGEST);
}

static void test_overflow_past_total_length_is_rejected(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x06, digest);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 100U, false)
           == ASSET_TRANSFER_OK);
    assert(asset_transfer_accept_chunk(&t, digest, 0U, 101U)
           == ASSET_TRANSFER_ERR_LENGTH);
}

static void test_chunk_without_begin_is_rejected(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x07, digest);
    memset(&t, 0, sizeof t);

    assert(asset_transfer_accept_chunk(&t, digest, 0U, 10U)
           == ASSET_TRANSFER_ERR_INACTIVE);
}

static void test_begin_rejects_zero_and_oversize_length(void)
{
    asset_transfer_t t;
    uint8_t digest[ASSET_DIGEST_BYTES];
    digest_of(0x08, digest);

    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT, 0U, false)
           == ASSET_TRANSFER_ERR_LENGTH);
    assert(asset_transfer_begin(&t, digest, ASSET_KIND_FONT,
                                ASSET_MAX_BYTES + 1U, false)
           == ASSET_TRANSFER_ERR_LENGTH);
}

int main(void)
{
    test_in_order_chunks_complete_the_transfer();
    test_repeated_chunk_is_idempotent();
    test_out_of_order_chunk_is_rejected();
    test_chunk_for_another_digest_is_rejected();
    test_overflow_past_total_length_is_rejected();
    test_chunk_without_begin_is_rejected();
    test_begin_rejects_zero_and_oversize_length();
    return 0;
}
```

- [ ] **Step 2: Run to confirm failure**

```sh
make -C firmware/host_tests test_asset_transfer
```

Expected: FAIL — missing header.

- [ ] **Step 3: Write the header**

Create `firmware/main/core/asset_transfer.h`:

```c
#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "core/asset_store.h"

typedef enum {
    ASSET_TRANSFER_OK = 0,
    ASSET_TRANSFER_DUPLICATE, /* chunk already accepted; not an error */
    ASSET_TRANSFER_ERR_ARGUMENT,
    ASSET_TRANSFER_ERR_INACTIVE,
    ASSET_TRANSFER_ERR_DIGEST,
    ASSET_TRANSFER_ERR_OFFSET,
    ASSET_TRANSFER_ERR_LENGTH,
} asset_transfer_result_t;

typedef struct {
    uint8_t digest[ASSET_DIGEST_BYTES];
    uint32_t total_length;
    uint32_t committed_offset;
    uint32_t last_chunk_offset;
    uint32_t last_chunk_length;
    uint8_t kind;
    bool volatile_tier;
    bool active;
} asset_transfer_t;

asset_transfer_result_t asset_transfer_begin(asset_transfer_t *transfer,
                                             const uint8_t *digest, uint8_t kind,
                                             uint32_t total_length,
                                             bool volatile_tier);
asset_transfer_result_t asset_transfer_accept_chunk(asset_transfer_t *transfer,
                                                    const uint8_t *digest,
                                                    uint32_t offset,
                                                    uint32_t length);
bool asset_transfer_is_complete(const asset_transfer_t *transfer);
uint32_t asset_transfer_resume_offset(const asset_transfer_t *transfer);
void asset_transfer_abort(asset_transfer_t *transfer);
```

- [ ] **Step 4: Implement**

- `asset_transfer_begin` validates `kind` against `asset_kind_t`, rejects
  `total_length == 0` or `> ASSET_MAX_BYTES` with
  `ASSET_TRANSFER_ERR_LENGTH`, then zeroes the struct, copies the digest, and sets
  `active = true`.
- `asset_transfer_accept_chunk` checks, in order: `active` (else `ERR_INACTIVE`),
  digest equality (else `ERR_DIGEST`), the duplicate case
  (`offset == last_chunk_offset && length == last_chunk_length && offset <
  committed_offset` → `DUPLICATE`), `offset == committed_offset` (else `ERR_OFFSET`),
  `length > 0 && offset + length <= total_length` with overflow-safe arithmetic (else
  `ERR_LENGTH`). On success it records `last_chunk_*` and advances `committed_offset`.
- `asset_transfer_is_complete` is `active && committed_offset == total_length`.
- `asset_transfer_abort` zeroes the struct.

- [ ] **Step 5: Register in both build systems**

Add `test_asset_transfer` to the Makefile's dependency and run lists plus:

```make
test_asset_transfer: test_asset_transfer.c ../main/core/asset_transfer.c \
	../main/core/asset_store.c
	$(CC) $(CFLAGS) -I../main -o $@ $^
```

Add `"core/asset_transfer.c"` to `SRCS` in `firmware/main/CMakeLists.txt`.

- [ ] **Step 6: Run and commit**

```sh
make -C firmware/host_tests clean test
```

```bash
git add firmware/main/core/asset_transfer.h firmware/main/core/asset_transfer.c \
        firmware/host_tests/test_asset_transfer.c firmware/host_tests/Makefile \
        firmware/main/CMakeLists.txt
git commit -m "feat: add the resumable asset transfer state machine"
```

---

### Task 5: Wire — device-side decode for types 15–18 and capability bit 5

**Files:**
- Modify: `firmware/main/core/protocol_message.h`, `firmware/main/core/protocol_message.c`
- Modify: `firmware/host_tests/test_protocol.c`
- Modify: `docs/protocol/v1.md`

**Interfaces:**
- Consumes: Task 2's `ASSET_DIGEST_BYTES`.
- Produces: `PROTOCOL_TYPE_ASSET_BEGIN` (15), `PROTOCOL_TYPE_ASSET_CHUNK` (16),
  `PROTOCOL_TYPE_ASSET_COMMIT` (17), `PROTOCOL_TYPE_ASSET_RELEASE` (18);
  `protocol_asset_begin_t`, `protocol_asset_chunk_t`, `protocol_asset_commit_t`,
  `protocol_asset_release_t`; `PROTOCOL_MAX_ASSET_CHUNK_BYTES` (1920),
  `PROTOCOL_MAX_ASSET_DIGESTS` (32); `protocol_ack_t.has_already_present` /
  `.already_present`.

**Payload shapes.** All keys are unsigned integers in deterministic order.

| Type | Payload |
| --- | --- |
| 15 `AssetBegin` | `{0: bstr(32) digest, 1: uint kind, 2: uint total_length, 3: bool volatile}` |
| 16 `AssetChunk` | `{0: bstr(32) digest, 1: uint offset, 2: bstr data}` |
| 17 `AssetCommit` | `{0: bstr(32) digest}` |
| 18 `AssetRelease` | `{0: [bstr(32) …]}` — 0..32 digests |

`Ack` gains optional key `2: already_present` (bool), present **only** when
`acknowledged_type` is 15. That is the entire inventory protocol: the host offers a
digest and is told whether to send the bytes. Content addressing means no separate
query message, no list to keep in sync, and no staleness.

`PROTOCOL_MAX_ASSET_CHUNK_BYTES` is 1920, not 2034: the envelope's payload cap must
also cover the CBOR map header, the 32-byte digest with its `bstr` header, the offset,
and the data `bstr` header — roughly 46 bytes. 1920 leaves deliberate margin.

- [ ] **Step 1: Write the failing tests**

Append to `firmware/host_tests/test_protocol.c`, following the file's existing
encode-then-decode helper style:

```c
static void test_asset_begin_roundtrips(void)
{
    uint8_t digest[ASSET_DIGEST_BYTES];
    memset(digest, 0x5A, sizeof digest);
    protocol_message_t message;

    assert(decode_asset_begin_fixture(digest, ASSET_KIND_FONT, 4096U, false,
                                      &message) == PROTOCOL_MESSAGE_OK);
    assert(message.type == PROTOCOL_TYPE_ASSET_BEGIN);
    assert(message.value.asset_begin.kind == ASSET_KIND_FONT);
    assert(message.value.asset_begin.total_length == 4096U);
    assert(message.value.asset_begin.volatile_tier == false);
    assert(memcmp(message.value.asset_begin.digest, digest, ASSET_DIGEST_BYTES) == 0);
}

static void test_asset_begin_rejects_short_digest(void)
{
    protocol_message_t message;
    /* A 31-byte digest must be rejected outright: a truncated digest would
     * silently address a different asset. */
    assert(decode_asset_begin_short_digest(&message)
           == PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
}

static void test_asset_chunk_rejects_oversize_data(void)
{
    protocol_message_t message;
    assert(decode_asset_chunk_with_data_length(PROTOCOL_MAX_ASSET_CHUNK_BYTES + 1U,
                                               &message)
           == PROTOCOL_MESSAGE_ERR_TOO_LARGE);
}

static void test_asset_release_rejects_too_many_digests(void)
{
    protocol_message_t message;
    assert(decode_asset_release_with_count(PROTOCOL_MAX_ASSET_DIGESTS + 1U, &message)
           == PROTOCOL_MESSAGE_ERR_TOO_LARGE);
}

static void test_ack_carries_already_present_only_for_asset_begin(void)
{
    protocol_ack_t ack;
    assert(decode_ack_with_already_present(PROTOCOL_TYPE_ASSET_BEGIN, true, &ack)
           == PROTOCOL_MESSAGE_OK);
    assert(ack.has_already_present && ack.already_present);

    /* Key 2 on any other acknowledged type is a contract violation. */
    assert(decode_ack_with_already_present(PROTOCOL_TYPE_ASSET_COMMIT, true, &ack)
           == PROTOCOL_MESSAGE_ERR_INVALID_VALUE);
}

static void test_current_capabilities_is_235(void)
{
    /* Bit 7 sat defined-but-dark for most of V2 and a conforming host could
     * not provision the device. Pin the number, not the expression. */
    assert(PROTOCOL_CURRENT_CAPABILITIES == UINT64_C(235));
}
```

Write the `decode_*_fixture` helpers in the same file, building the CBOR by hand as the
existing tests in `test_protocol.c` do.

- [ ] **Step 2: Run to confirm failure**

```sh
make -C firmware/host_tests test_protocol
```

Expected: FAIL — `PROTOCOL_TYPE_ASSET_BEGIN` undeclared.

- [ ] **Step 3: Extend the header**

Add the four enum values to `protocol_message_type_t`, the four payload structs, the
two new bounds, and the `Ack` fields. Add `PROTOCOL_CAPABILITY_ASSET_TRANSFER` to
`PROTOCOL_CURRENT_CAPABILITIES`:

```c
#define PROTOCOL_MAX_ASSET_CHUNK_BYTES 1920U
#define PROTOCOL_MAX_ASSET_DIGESTS 32U

#define PROTOCOL_CURRENT_CAPABILITIES                            \
    (PROTOCOL_CAPABILITY_CORE_WIDGETS |                          \
     PROTOCOL_CAPABILITY_CONFIG_ROTATION |                       \
     PROTOCOL_CAPABILITY_EXTENDED_TEMPLATES |                    \
     PROTOCOL_CAPABILITY_ASSET_TRANSFER |                        \
     PROTOCOL_CAPABILITY_FIRMWARE_UPDATE |                       \
     PROTOCOL_CAPABILITY_NETWORKING)
```

Add the four payload types to the `protocol_message_t` union.

- [ ] **Step 4: Implement the decoders**

Follow `decode_network_config`'s structure exactly: iterate map entries, track required
keys with `REQUIRED_BIT`, `skip_value()` unknown integer keys, reject duplicates,
and validate after the loop. Specifically:

- Reject any digest `bstr` whose length is not exactly `ASSET_DIGEST_BYTES` with
  `PROTOCOL_MESSAGE_ERR_INVALID_VALUE`.
- Reject `kind` outside `asset_kind_t` with `PROTOCOL_MESSAGE_ERR_INVALID_VALUE`.
- Reject `total_length` of zero or above `ASSET_MAX_BYTES`, and chunk data
  above `PROTOCOL_MAX_ASSET_CHUNK_BYTES`, with `PROTOCOL_MESSAGE_ERR_TOO_LARGE`.
- Reject a release array longer than `PROTOCOL_MAX_ASSET_DIGESTS` with
  `PROTOCOL_MESSAGE_ERR_TOO_LARGE`.

Extend the `Ack` validator (`protocol_message.c` around line 985): add the four new
types to the accepted-type list with `revision_required = false`, and enforce that
`has_already_present` is true **iff** `acknowledged_type == PROTOCOL_TYPE_ASSET_BEGIN`.

- [ ] **Step 5: Fix the host-test include path**

`protocol_message.h` now includes `core/asset_store.h`, which
`firmware/host_tests/Makefile`'s `test_protocol` rule cannot find — it compiles with
`-I$(CBOR_DIR)` only. Add `-I../main` to that rule:

```make
test_protocol: test_protocol.c $(PROTOCOL_SRCS)
	$(CC) $(CFLAGS) -I$(CBOR_DIR) -I../main -o $@ $^
```

Check every other rule that compiles `$(PROTOCOL_SRCS)` and add `-I../main` there too.

- [ ] **Step 6: Update the protocol document**

In `docs/protocol/v1.md`, add rows 15–18 to the registry table, document the four
payloads and the new `Ack` key in the CBOR section, and note that all four are gated on
capability bit 5. State that the current capability value is `235`.

- [ ] **Step 7: Run the tests**

```sh
make -C firmware/host_tests clean test
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add firmware/main/core/protocol_message.h firmware/main/core/protocol_message.c \
        firmware/host_tests/test_protocol.c firmware/host_tests/Makefile docs/protocol/v1.md
git commit -m "feat: add asset transfer message types and switch on capability bit 5"
```

---

### Task 6: Wire — host-side encode/decode parity in Rust

**Files:**
- Modify: `companion/crates/protocol/src/message.rs`
- Modify: `companion/crates/protocol/src/lib.rs` (re-exports)

**Interfaces:**
- Consumes: Task 5's payload shapes and bounds — these must match byte for byte.
- Produces: `Message::AssetBegin(AssetBegin)`, `Message::AssetChunk(AssetChunk)`,
  `Message::AssetCommit(AssetCommit)`, `Message::AssetRelease(AssetRelease)`;
  `Ack.already_present: Option<bool>`; `MAX_ASSET_CHUNK_BYTES`, `MAX_ASSET_DIGESTS`;
  `CURRENT_CAPABILITIES == 235`.

- [ ] **Step 1: Write the failing tests**

Append to `companion/crates/protocol/src/message.rs`'s test module:

```rust
#[test]
fn current_capabilities_is_235() {
    // Bit 7 was defined and never set for most of V2; the constant read 75
    // instead of 203 and a conforming host could not have provisioned the
    // device. Pin the number so the same omission cannot recur.
    assert_eq!(CURRENT_CAPABILITIES, 235);
}

#[test]
fn asset_begin_roundtrips() {
    let message = Message::AssetBegin(AssetBegin {
        digest: [0x5a; 32],
        kind: AssetKind::Font,
        total_length: 4096,
        volatile: false,
    });
    let frame = message.encode(7).expect("encode");
    let decoded = Message::decode(&frame).expect("decode");
    assert_eq!(decoded, message);
}

#[test]
fn asset_chunk_rejects_oversize_data() {
    let message = Message::AssetChunk(AssetChunk {
        digest: [0x01; 32],
        offset: 0,
        data: vec![0u8; MAX_ASSET_CHUNK_BYTES + 1],
    });
    assert!(matches!(
        message.encode(8),
        Err(MessageError::InvalidValue("asset chunk data too large"))
    ));
}

#[test]
fn asset_release_rejects_too_many_digests() {
    let message = Message::AssetRelease(AssetRelease {
        digests: vec![[0u8; 32]; MAX_ASSET_DIGESTS + 1],
    });
    assert!(matches!(
        message.encode(9),
        Err(MessageError::InvalidValue("too many asset digests"))
    ));
}

#[test]
fn already_present_is_rejected_on_non_asset_begin_acks() {
    let message = Message::Ack(Ack {
        acknowledged_type: TYPE_ASSET_COMMIT,
        revision: None,
        already_present: Some(true),
    });
    assert!(message.encode(10).is_err());
}
```

- [ ] **Step 2: Run to confirm failure**

```sh
cd companion && cargo test -p protocol
```

Expected: FAIL — `AssetBegin` not found; `current_capabilities_is_235` fails with 203.

- [ ] **Step 3: Implement**

Add `TYPE_ASSET_BEGIN = 15` … `TYPE_ASSET_COMMIT = 17`, `TYPE_ASSET_RELEASE = 18`
alongside the existing type constants; add `AssetKind`, the four payload structs, the
four `Message` variants (including `type_id()` arms), encode and decode following
`NetworkConfig`'s existing implementation, and `already_present: Option<bool>` on
`Ack`. Add `CAPABILITY_ASSET_TRANSFER` to `CURRENT_CAPABILITIES`.

Every bound must equal Task 5's: `MAX_ASSET_CHUNK_BYTES = 1920`,
`MAX_ASSET_DIGESTS = 32`, digests exactly 32 bytes, `total_length` in
`1..=1_048_576`.

- [ ] **Step 4: Run the gates**

```sh
cd companion && cargo fmt --all --check \
  && cargo clippy --workspace --all-targets -- -D warnings \
  && cargo test --workspace
```

Expected: PASS. Existing tests that construct `Ack` need the new field added.

- [ ] **Step 5: Add the new types to the shared fixture corpus**

`protocol/fixtures/v1/` holds 49 `.bin` files that are this project's real cross-language
contract. `companion/crates/protocol/examples/generate-fixtures.rs` writes them, and
**both** `companion/crates/protocol/tests/fixtures.rs` and
`firmware/host_tests/test_protocol.c` read them — the latter via `FIXTURE_DIRECTORY`,
whose `assert_valid_fixture()` re-encodes each decoded message and `memcmp`s the result
against the file. That round-trip is the only mechanism proving the two implementations
agree byte for byte; unit tests on either side cannot.

Add fixtures for `asset_begin`, `asset_chunk`, `asset_commit`, `asset_release`, and an
`ack` carrying `already_present` — that last one is the only fixture that can pin the
`Some`-iff-type-15 rule across languages. Firmware's list is explicit rather than a
directory walk, so add matching `assert_valid_fixture()` calls to `test_valid_fixtures()`
as well.

Choose payloads that can actually catch a divergence: a non-trivial digest, a
`total_length` and `offset` large enough to exercise multi-byte CBOR integer encoding,
non-empty chunk data, and a release array with more than one digest. An all-minimal
fixture passes even when the two encoders disagree about everything interesting.

- [ ] **Step 6: Run both suites and commit**

```sh
make -C firmware/host_tests clean test
```

If firmware's `memcmp` fails, the two encoders genuinely disagree — report it, do not
adjust the fixture to make it pass.

```bash
git add companion/crates/protocol/src/message.rs companion/crates/protocol/src/lib.rs \
        companion/crates/protocol/examples/generate-fixtures.rs \
        companion/crates/protocol/tests/fixtures.rs \
        firmware/host_tests/test_protocol.c protocol/fixtures/v1/
git commit -m "feat: encode and decode asset transfer messages on the host"
```

---

### Task 7: Flash backing and memory mapping

**Files:**
- Create: `firmware/main/link/asset_flash.h`, `firmware/main/link/asset_flash.c`
- Modify: `firmware/main/CMakeLists.txt`

**Interfaces:**
- Consumes: Task 2's `asset_flash_io_t`; Task 3's `asset_move_t`.
- Produces: `asset_flash_init()`, `asset_flash_io()`, `asset_flash_store()`,
  `asset_flash_map(record, &ptr)`, `asset_flash_write_blob()`,
  `asset_flash_execute_compaction(moves, count)`.

This file is deliberately thin. It contains no policy — it reads, writes, erases, maps,
and executes a move list that `core/` planned. It is not host-testable (it needs
ESP-IDF), which is exactly why the decision-making lives in `core/`.

`esp_partition_mmap()` returns a pointer into the flash cache; that pointer is what
Task 8 hands to `lv_tiny_ttf_create_data_ex()`. **Font bytes are therefore never copied
into RAM.**

- [ ] **Step 1: Write the header**

```c
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
```

- [ ] **Step 2: Implement**

- `asset_flash_init` uses `esp_partition_find_first(ESP_PARTITION_TYPE_DATA, 0x40,
  NULL)`. If `asset_store_open()` returns `ASSET_STORE_ERR_NOT_FORMATTED`, call
  `asset_store_format()` and reopen. Any other error is returned, not papered over.
- The `asset_flash_io_t` implementation wraps `esp_partition_read`,
  `esp_partition_write`, and `esp_partition_erase_range`.
- `asset_flash_map` calls `esp_partition_mmap(partition, blob_region_offset +
  record->offset, record->length, ESP_PARTITION_MMAP_DATA, out_ptr, &handle)`.
  Mmap handles are held in a PSRAM-allocated table, not a static array.
- `asset_flash_execute_compaction` reads every surviving record and blob into a PSRAM
  staging buffer **before any erase** — a per-move flash-to-flash copy is unsafe, because
  destination space can hold a dead record's un-erased bytes and NOR writes only clear
  bits. It then erases, and writes in this order: **blobs, then the record array, then
  the header last**.

  **The header write is the commit point, and the ordering is load-bearing.** Writing the
  record array (carrying each survivor's new packed `offset`) before the blob bytes leaves
  a window where power loss yields a store that opens perfectly — valid magic, every
  record `COMMITTED` — whose offsets point into the *pre-compaction* blob layout, i.e. at
  another asset's bytes. `asset_store_open` does not cross-check record offsets against
  blob content, so nothing detects it and a font simply renders garbage. Writing the
  header last instead means any interruption leaves the magic erased,
  `asset_store_open` returns `ASSET_STORE_ERR_NOT_FORMATTED`, `asset_flash_init`
  reformats, and the host re-syncs. Compaction is deliberately not atomic — the host holds
  every asset — but it must fail **closed**.

**Memory rule for this file:** the partition handle and the mmap table are the only
module state. Keep `.bss` to pointers — a handful of bytes, not buffers. Every buffer
comes from `heap_caps_malloc(..., MALLOC_CAP_SPIRAM)`. Spec §9.

- [ ] **Step 3: Register and build**

Add `"link/asset_flash.c"` to `SRCS`, and add `spi_flash` to `REQUIRES` if the build
reports it missing.

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

Expected: builds clean with no new warnings.

- [ ] **Step 4: Record the size delta so far**

```sh
idf.py -C firmware size > /tmp/size-task7.txt
diff /tmp/size-baseline.txt /tmp/size-task7.txt
```

Note the `.bss` delta in the commit message. If DIRAM `.bss` has grown by more than a
few dozen bytes, find out why before continuing — do not defer this to Task 13.

- [ ] **Step 5: Commit**

```bash
git add firmware/main/link/asset_flash.h firmware/main/link/asset_flash.c \
        firmware/main/CMakeLists.txt
git commit -m "feat: back the asset store with the assets partition and mmap"
```

---

### Task 8: Font registry — arbitrary pixel sizes via tiny_ttf

**Files:**
- Create: `firmware/main/ui/font_registry.h`, `firmware/main/ui/font_registry.c`
- Modify: `firmware/lv_conf.h:1052`, `firmware/main/CMakeLists.txt`

**Interfaces:**
- Consumes: nothing at compile time. The digest→bytes resolver is injected, so this
  task does **not** depend on Task 7.
- Produces: `asset_resolver_fn`, `font_registry_result_t`, `font_registry_init(resolver)`,
  `font_registry_acquire(digest, pixel_size)` → `lv_font_t *`,
  `font_registry_release(font)`, `font_registry_reset()`, `font_registry_warm()`.

> **Controller ruling (pre-flight scan).** This file must contain **no ESP-IDF
> include and no `esp_err_t`**, because Task 12 compiles it into `lvgl-sim`, which
> builds firmware C on the host with no ESP-IDF present. As originally drafted Task 8
> and Task 12 could not both hold. The resolver callback is the fix, and it also honours
> the repo's standing rule that hardware-independent logic stays free of ESP-IDF.

This is the task that lifts the four-size ceiling. `lv_tiny_ttf_create_data_ex()` takes
the mmap'd TTF pointer and any pixel size, so `deskmate_font_18/28/56/96` stop being
the available set.

- [ ] **Step 1: Enable tiny_ttf**

In `firmware/lv_conf.h`, change line 1052 from `#define LV_USE_TINY_TTF 0` to
`#define LV_USE_TINY_TTF 1`. Leave `LV_TINY_TTF_FILE_SUPPORT` at 0 — assets come from
memory, and file support would pull in an LVGL filesystem driver we do not want.

- [ ] **Step 2: Confirm the IRAM story before writing code**

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build && idf.py -C firmware size
```

Expected: builds clean; `stb_truetype` lands in flash `.text`. **IRAM is reported 100%
full on this board**, so if IRAM overflows, stop and report rather than trimming
something else to make room.

- [ ] **Step 3: Write the header**

```c
#pragma once

#include "core/asset_store.h"
#include "lvgl.h"

/* Resolves a digest to mapped asset bytes. Firmware passes an
 * asset_flash-backed resolver (wired in Task 9); lvgl-sim passes a RAM-backed
 * one (Task 12). This indirection is why this file carries no ESP-IDF include
 * and can therefore be compiled into the simulator like every other ui/ file. */
typedef bool (*asset_resolver_fn)(const uint8_t *digest, const void **out_ptr,
                                  uint32_t *out_len, uint8_t *out_kind);

typedef enum {
    FONT_REGISTRY_OK = 0,
    FONT_REGISTRY_ERR_ARGUMENT,
    FONT_REGISTRY_ERR_MEMORY,
} font_registry_result_t;

font_registry_result_t font_registry_init(asset_resolver_fn resolver);

/* Returns a font for `digest` rendered at `pixel_size`, or NULL if the asset
 * is absent or not a font. Call under lvgl_port_lock(). */
lv_font_t *font_registry_acquire(const uint8_t *digest, int32_t pixel_size);

void font_registry_release(lv_font_t *font);

/* Destroys every open face. Call before compaction invalidates mmap pointers. */
void font_registry_reset(void);

/* Rasterizes `glyphs` into the face's cache ahead of first paint, so a cold
 * 96px digit does not rasterize inline on the LVGL task mid-render. */
void font_registry_warm(lv_font_t *font, const char *glyphs);
```

- [ ] **Step 4: Implement**

An LRU keyed by `(digest, pixel_size)` over at most 8 open faces. On a miss: call the
injected `asset_resolver_fn` → accept `ASSET_KIND_FONT` **and** `ASSET_KIND_ICON_FONT`,
reject `ASSET_KIND_IMAGE` →
`lv_tiny_ttf_create_data_ex(ptr, record.length, pixel_size, LV_FONT_KERNING_NORMAL,
cache_size)`. On eviction, `lv_tiny_ttf_destroy()`.

Three requirements that are easy to miss:

1. **The LRU table is `heap_caps_malloc(..., MALLOC_CAP_SPIRAM)`.** Only the table
   pointer is module state. Keep this file's `.bss` at pointer size, following
   `5699f1d`'s precedent of packing state rather than growing `.bss`.
2. **`font_registry_reset()` must run before any compaction**, because compaction
   moves blobs and invalidates every mmap'd pointer a live `lv_font_t` holds. A font
   surviving compaction would read moved bytes.
3. **Warm the cache at card activation.** A miss rasterizes inline on the LVGL task, so
   the first render of each glyph at 96 px can show as a hitch. Provide
   `font_registry_warm(font, "0123456789:")` and call it when a card becomes active.

- [ ] **Step 5: Add to the build and verify a size render**

Add `"ui/font_registry.c"` to `SRCS`, build, and confirm no new warnings.

- [ ] **Step 6: Commit**

```bash
git add firmware/lv_conf.h firmware/main/ui/font_registry.h \
        firmware/main/ui/font_registry.c firmware/main/CMakeLists.txt
git commit -m "feat: render text at any pixel size from stored TTF assets"
```

---

### Task 9: Protocol task wiring

**Files:**
- Modify: `firmware/main/link/protocol_task.c`
- Modify: `firmware/main/core/protocol_message.c` (StatusResponse key 31)
- Modify: `docs/protocol/v1.md`

**Interfaces:**
- Consumes: Task 4's `asset_transfer_t`; Task 5's message types; Task 7's
  `asset_flash_*`.
- Produces: device-side handling for types 15–18, and `StatusResponse` key 31
  (`asset_store_used_bytes`, `asset_store_free_bytes`, `asset_count`).

- [ ] **Step 1: Wire the four handlers**

In `protocol_task.c`'s dispatch, add cases for the four types:

- `AssetBegin` — **reject `volatile: true` with `PROTOCOL_ERROR_UNSUPPORTED_MESSAGE`.**
  The volatile (PSRAM) tier exists for rasterized frames and arrives in stage 4; stage 1
  implements only the durable tier. Storing a volatile asset to flash instead would burn
  the partition's write endurance on a 30-second refresh cycle — precisely the thing the
  tier exists to prevent — so this must be an explicit typed refusal, never a silent
  downgrade. The wire already carries the flag (Task 5) so that stage 4 is purely
  additive.
- `AssetBegin` (durable) — if `asset_store_find()` succeeds, reply
  `Ack{type: 15, already_present: true}` and do **not** start a transfer. Otherwise
  `asset_store_reserve()`, `asset_transfer_begin()`, and reply
  `Ack{type: 15, already_present: false}`. A second `AssetBegin` while a transfer is
  active aborts the old one first — the host may legitimately give up and retry.
- `AssetChunk` — `asset_transfer_accept_chunk()`. On `OK`, `asset_flash_write_blob()` at
  the reserved blob offset plus the chunk offset, then `Ack`. On `DUPLICATE`, `Ack`
  **without** rewriting — that is the lost-`Ack` path. Any error replies `Error` with
  `PROTOCOL_ERROR_INVALID_PAYLOAD` and aborts the transfer.
- `AssetCommit` — reject unless `asset_transfer_is_complete()`, then
  `asset_store_commit()` and `Ack`.
- `AssetRelease` — `font_registry_reset()`, then `asset_store_mark_dead()` for every
  committed record absent from the digest list, then `asset_store_plan_compaction()`
  and `asset_flash_execute_compaction()`. `Ack` last.

The transfer struct is a single `asset_transfer_t` owned by the protocol task, not a
static — allocate it with the task's other state.

- [ ] **Step 2: Add StatusResponse key 31**

Encode `{31: {0: used, 1: free, 2: count}}` from `asset_store_stats()`. Follow keys
29–30's precedent: omit the key entirely when the store is unformatted, and document
that a reader which does not see it treats the device as having no asset store.

- [ ] **Step 3: Wire the flash-backed font resolver**

Task 8 left `font_registry_init()` taking an `asset_resolver_fn`. Implement that
resolver here, over `asset_flash_store()` + `asset_flash_map()`, and call
`font_registry_init()` once at boot after `asset_flash_init()`. This is the only place
the two halves meet.

- [ ] **Step 4: Add the capability gate**

Reject all four types with `PROTOCOL_ERROR_UNSUPPORTED_MESSAGE` when
`PROTOCOL_CAPABILITY_ASSET_TRANSFER` is not advertised. This mirrors how types 13–14
gate on bit 7 and keeps `docs/protocol/v1.md` truthful.

- [ ] **Step 5: Build and run every host test**

```sh
make -C firmware/host_tests clean test
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware build
```

- [ ] **Step 6: Update the protocol document and commit**

Add key 31 to the `StatusResponse` table in `docs/protocol/v1.md`.

```bash
git add firmware/main/link/protocol_task.c firmware/main/core/protocol_message.c \
        docs/protocol/v1.md
git commit -m "feat: handle asset transfer messages on the device"
```

---

### Task 10: Config schema v5 — asset variants

**Files:**
- Modify: `companion/crates/app-core/src/config.rs`, `companion/crates/app-core/src/store.rs`
- Create: `docs/config/v5.md`

**Interfaces:**
- Consumes: Task 6's `AssetKind`.
- Produces: `AssetSettings` with `font`/`icon-font`/`image` variants,
  `SCHEMA_VERSION = 5`, and removal of the `RequiresCapability` rejection at
  `config.rs:819`.

v4's `font { pixel_size, glyph_ranges }` and `icon { width, height }` encode the
pre-`tiny_ttf` design where glyphs were baked at a fixed size. **Because
`config.rs:819` has always rejected a non-empty `assets` array, no saved config
contains one**, so migration is a version bump with no data transformation. Say so in
`docs/config/v5.md` rather than writing a migration that can never run.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn v4_config_migrates_to_v5_unchanged() {
    let v4 = sample_v4_document();
    let migrated = load_from_str(&v4).expect("migrate");
    assert_eq!(migrated.schema_version, 5);
    assert!(migrated.assets.is_empty());
}

#[test]
fn font_asset_no_longer_carries_a_pixel_size() {
    // The four-size ceiling is gone: size is chosen per text node at render
    // time, so pinning one in the config would be meaningless.
    let json = r#"{"kind":"font","id":"inter","source":{"kind":"file","value":"/f.ttf"},
                   "maximum_bytes":262144,"pixel_size":48}"#;
    assert!(serde_json::from_str::<AssetSettings>(json).is_err());
}

#[test]
fn a_font_asset_compiles_and_requires_capability_bit_5() {
    let config = config_with_one_font_asset();
    let compiled = config.compile(1).expect("assets must compile now");
    assert!(config.required_capabilities() & protocol::CAPABILITY_ASSET_TRANSFER != 0);
    assert_eq!(compiled.assets.len(), 1);
}

#[test]
fn asset_budget_total_is_enforced() {
    let config = config_with_assets_totalling(1_048_577);
    let issues = config.validate().expect_err("over budget");
    assert!(issues.issues.iter().any(|i| i.path == "assets"));
}
```

- [ ] **Step 2: Run to confirm failure**

```sh
cd companion && cargo test -p app-core
```

Expected: FAIL — `a_font_asset_compiles_...` fails with `RequiresCapability`.

- [ ] **Step 3: Implement**

Reshape `AssetSettings` to the three v5 variants, bump `SCHEMA_VERSION` to 5, add the
v4→v5 step in `store.rs` alongside the existing v0–v3 migrations, and **delete** the
`compatibility_issues.push(... "asset transfer is not implemented by this build")`
block at `config.rs:819`.

Keep every existing bound: at most 16 assets, `maximum_bytes` in `1..=262_144`, and a
total budget of 1,048,576 bytes. Keep `#[serde(deny_unknown_fields)]` on every variant
and remember that it is **a silent no-op on internally tagged enums** — v4's
"Unknown-field rejection" section documents the workaround this schema already uses.

- [ ] **Step 4: Write `docs/config/v5.md`**

Copy v4's structure. State that v5 differs from v4 only in the asset variants, that the
wire and firmware are unchanged, and that no saved config has ever contained an asset.
Mark `docs/config/v4.md` superseded in-file, as v3 was.

- [ ] **Step 5: Run the gates and commit**

```sh
cd companion && cargo fmt --all --check \
  && cargo clippy --workspace --all-targets -- -D warnings \
  && cargo test --workspace
```

```bash
git add companion/crates/app-core/src/config.rs companion/crates/app-core/src/store.rs \
        docs/config/v5.md docs/config/v4.md
git commit -m "feat: reshape config assets for runtime fonts as schema v5"
```

---

### Task 11: Server-side transfer driver

**Files:**
- Create: `companion/crates/server/src/asset_sync.rs`
- Modify: `companion/crates/server/src/lib.rs`, `companion/crates/app-core/src/runtime.rs`

**Interfaces:**
- Consumes: Task 6's message types; Task 10's `AssetSettings`.
- Produces: `AssetSync::reconcile(device, desired) -> Result<AssetSyncReport>`, and
  `RuntimeDevice::send_asset_*` methods.

Unlike `provision` and `factory_reset` — which return a typed
unsupported-on-this-transport error because provisioning is a cable operation by design
— **asset transfer must work over both transports.** The server owning the device is
the entire point of networked tier.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn reconcile_skips_assets_the_device_already_holds() {
    let device = FakeDevice::new().with_already_present([0xaa; 32]);
    let desired = vec![asset_blob([0xaa; 32], 4096), asset_blob([0xbb; 32], 2048)];

    let report = AssetSync::reconcile(&device, &desired).await.expect("reconcile");

    // Content addressing is the inventory protocol: AssetBegin answers
    // already_present and we send no chunks at all for that digest.
    assert_eq!(report.skipped, 1);
    assert_eq!(report.uploaded, 1);
    assert_eq!(device.chunks_sent_for(&[0xaa; 32]), 0);
}

#[tokio::test]
async fn reconcile_resumes_from_the_last_committed_offset() {
    let device = FakeDevice::new().failing_after_chunks(2);
    let desired = vec![asset_blob([0xcc; 32], MAX_ASSET_CHUNK_BYTES * 5)];

    let _ = AssetSync::reconcile(&device, &desired).await;
    let report = AssetSync::reconcile(&device.recovered(), &desired).await.expect("resume");

    assert_eq!(report.resumed_from_offset, MAX_ASSET_CHUNK_BYTES as u32 * 2);
}

#[tokio::test]
async fn reconcile_releases_digests_no_longer_desired() {
    let device = FakeDevice::new().holding([[0xaa; 32], [0xdd; 32]]);
    let desired = vec![asset_blob([0xaa; 32], 128)];

    AssetSync::reconcile(&device, &desired).await.expect("reconcile");

    assert_eq!(device.last_release(), Some(vec![[0xaa; 32]]));
}
```

- [ ] **Step 2: Run to confirm failure, then implement**

```sh
cd companion && cargo test -p server asset_sync
```

`reconcile` computes the desired digest set, sends `AssetBegin` per asset with
`volatile: false` (stage 1 has no volatile tier — see Task 9), skips
chunking when the `Ack` reports `already_present`, otherwise streams
`MAX_ASSET_CHUNK_BYTES` chunks in order and sends `AssetCommit`, then sends one
`AssetRelease` carrying the full desired set.

**Yield between chunks.** Spec §4 chose one-outstanding-request plus preemption over
windowing; that choice is only real if this driver actually lets interactive traffic
through. A long transfer must not starve taps or heartbeats.

- [ ] **Step 3: Run the gates and commit**

```sh
cd companion && cargo fmt --all --check \
  && cargo clippy --workspace --all-targets -- -D warnings \
  && cargo test --workspace
```

```bash
git add companion/crates/server/src/asset_sync.rs companion/crates/server/src/lib.rs \
        companion/crates/app-core/src/runtime.rs
git commit -m "feat: reconcile device assets from the server"
```

---

### Task 12: Simulator asset shim and a non-baked-size golden

**Files:**
- Create: `companion/crates/lvgl-sim/src/assets.rs`
- Create: `companion/crates/lvgl-sim/assets/Inter-subset.ttf`, `.../OFL.txt`
- Modify: `companion/crates/lvgl-sim/build.rs`, `companion/crates/lvgl-sim/src/cases.rs`,
  `companion/crates/lvgl-sim/csrc/sim_shim.c`

**Interfaces:**
- Consumes: Task 8's `font_registry_acquire`.
- Produces: `sim_asset_register(digest, bytes, len)` — the shim that resolves a digest
  to bytes without ESP-IDF.

**This task BEGINS discharging the parity obligation the spec records in §6 — it does
not complete it.** What the golden proves is that the simulator, built from the firmware's
own `asset_store.c`, `font_registry.c` and LVGL sources, can resolve a digest to bytes and
rasterize a runtime font correctly. It does **not** prove simulator-vs-firmware agreement,
because `cases::golden_cases()` is also driven onto hardware by
`device/examples/framebuffer_diff.rs` via `TemplateKind`/`PushData`, and no wire message
exists for "register this font blob" — so an asset-font case cannot ride that path. The
obligation is closed only by Task 13's on-device probe plus Task 14's hardware comparison.
Do not cite this task alone as closing §6. Today the
simulator compiles the firmware's own font `.c` files, so both hosts rasterize
identically by construction. Once fonts are runtime assets that stops being automatic:
the simulator must resolve **the same digest to the same bytes** as the device. If it
does not, the framebuffer diff silently stops meaning anything.

- [ ] **Step 1: Vendor a subset TTF**

Subset an OFL-licensed face to the glyphs the golden needs (`0-9`, `:`, `A-Z`, space)
and commit it with its `OFL.txt`. Keep it under 64 KB. Record the SHA-256 in
`cases.rs` — the digest is the identity, so it must be written down, not computed at
test time from a file that could be replaced.

- [ ] **Step 2: Write the failing golden case**

Add to `cases.rs` a case named `asset-font--72px-digits` that registers the subset TTF,
acquires it at **72 px** — deliberately not 18, 28, 56, or 96 — and renders `12:34`
centred on the 448×368 canvas.

```sh
cd companion && cargo test -p lvgl-sim asset_font
```

Expected: FAIL — no such case.

- [ ] **Step 3: Implement the shim**

`sim_shim.c` gains a RAM-backed `asset_flash_io_t` and a `sim_asset_register()` that
inserts a committed record plus its blob, so `asset_store_find()` and a shimmed
`asset_flash_map()` behave exactly as on device. `build.rs` adds
`core/asset_store.c`, `core/asset_transfer.c`, and `ui/font_registry.c` to its source
list — the same firmware sources, as it already does for templates.

- [ ] **Step 4: Generate and review the golden**

Regenerate goldens, then **look at the PNG**. A golden nobody looked at pins whatever
bug shipped with it.

- [ ] **Step 5: Run the gates and commit**

```sh
cd companion && cargo fmt --all --check \
  && cargo clippy --workspace --all-targets -- -D warnings \
  && cargo test --workspace
```

```bash
git add companion/crates/lvgl-sim/
git commit -m "test: render a 72px asset font in the simulator"
```

---

### Task 13: On-device render probe

**Files:**
- Modify: `firmware/main/link/dev_capture.c`, `firmware/main/link/dev_capture.h`

**Interfaces:**
- Consumes: Task 8's `font_registry_acquire`; Task 12's golden case name.
- Produces: a `DESKMATE_DEV_DIAG`-only command that renders the same case on the panel
  so `framebuffer_diff` can compare it.

Stage 1 has no scene renderer yet, so nothing in the shipping UI uses an asset font.
This probe is what makes the stage physically verifiable rather than merely tested —
and it follows `dev_capture.c`'s existing pattern exactly: always compiled, entirely
`#ifdef DESKMATE_DEV_DIAG`-guarded, absent from release binaries.

- [ ] **Step 1: Add the probe command**

Add a dev-only message id alongside the existing 0x7E/0x7F capture ids that renders
`12:34` at 72 px from a given digest onto the active screen, under
`lvgl_port_lock()`/`lvgl_port_unlock()`.

- [ ] **Step 2: Verify it is absent from a release build**

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware fullclean && idf.py -C firmware build
strings firmware/build/deskmate.elf | grep -c asset_probe
```

Expected: `0`. This is the same check the Task 10 report used for `dev_capture`.

- [ ] **Step 3: Commit**

```bash
git add firmware/main/link/dev_capture.c firmware/main/link/dev_capture.h
git commit -m "test: add a dev-only asset font render probe"
```

---

### Task 14: Hardware gates and documentation

**Files:**
- Modify: `docs/hardware/board-notes.md`, `CLAUDE.md`
- Modify: `docs/superpowers/plans/2026-08-22-deskmate-asset-store.md` (results)

**Interfaces:**
- Consumes: Task 1's baseline; every preceding task.
- Produces: the recorded evidence that licenses stage 2.

**No step here may be marked complete on software grounds.** Every one is a physical
observation.

- [ ] **Step 1: Record the `.bss` delta against Task 1's baseline**

```sh
. "$HOME/esp/esp-idf/export.sh"
idf.py -C firmware fullclean && idf.py -C firmware build
idf.py -C firmware size > /tmp/size-final.txt
diff /tmp/size-baseline.txt /tmp/size-final.txt
```

Record the DIRAM `.bss` and IRAM figures. Spec §9: `3f2aa03` broke OTA with ~105 bytes.

- [ ] **Step 2: Verify an OTA download on the board — the blocking gate**

Publish a build under a new version string and confirm the device downloads and
installs it. Remember the standing traps: move `DESKMATE_FIRMWARE_VERSION` to match or
the build is reverted within a minute; a version string that already failed is refused
forever; and `firmware/version.txt` pins the version — do not rely on `git describe`.

**If the download fails, stop.** Do not proceed to stage 2 and do not attempt a fix by
inspection. Bisect against Task 1's baseline build, as `3f2aa03` was bisected: the
cause will be memory layout, and it will not be visible in any test.

- [x] **Step 3: Push a font and verify an asset-backed scene**

The temporary Task 13 0x7D probe and its special host renderer were retired after the
scene renderer made their premise obsolete. The normal path now uploads the Task 12
subset TTF with `AssetBegin`/`AssetChunk`/`AssetCommit`, renders it from a 72 px
asset-backed `SceneText`, and captures through the retained 0x7E path. Stage 3b Task 9
observed that path at both orientations on 2026-08-30; the simulator retains one
landscape SceneText golden and derives flipped output from the renderer's reversal.

- [ ] **Step 4: Measure the glyph-cache-miss hitch**

Time `font_registry_acquire()` plus a first render at 96 px on the LVGL task, cold.
Record the number. If it is visible, note whether `font_registry_warm()` removes it.
This is the risk the spec flags as appearing only on hardware.

- [ ] **Step 5: Verify persistence across reboot and OTA**

Power-cycle: the asset must still be present and render. Then run an OTA: assets live
in a separate partition and **must** survive. If they do not, the partition layout is
wrong and stage 2 depends on this working.

- [ ] **Step 6: Record everything in board notes**

Append "Asset store — verified YYYY-MM-DD" with the observed results of steps 1–5,
each stated as observed or not observed. Do not describe anything unobserved as
verified.

- [ ] **Step 7: Update `CLAUDE.md`**

Add a "Current state" entry: schema is now **v5**, `CURRENT_CAPABILITIES` is now
**235**, fonts are runtime assets in the 6 MB partition, `LV_USE_TINY_TTF` is on, and
the four-baked-size and eleven-icon ceilings are lifted. Note that `weather_icon.c` and
the four `deskmate_font_*.c` files still exist and are still used by the six C
templates — they retire in stage 3, not here.

- [ ] **Step 8: Commit**

```bash
git add docs/hardware/board-notes.md CLAUDE.md \
        docs/superpowers/plans/2026-08-22-deskmate-asset-store.md
git commit -m "docs: record asset store hardware verification"
```

---

## Exit criteria

Stage 1 is complete when all of the following are **observed**, not inferred:

1. `make -C firmware/host_tests clean test` passes.
2. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   and `cargo test --workspace` pass from `companion/`.
3. `idf.py -C firmware build` is clean and the `.bss` delta against Task 1 is recorded.
4. **An OTA download completes on the physical board.**
5. A pushed font renders at 72 px on the panel, byte-identical to the simulator golden,
   at both orientations.
6. The asset survives a power cycle and an OTA.

Stage 2's plan is written after this, using what stage 1 taught — per the spec's §7 and
the repo's working agreement on writing the next plan at the current one's exit.
