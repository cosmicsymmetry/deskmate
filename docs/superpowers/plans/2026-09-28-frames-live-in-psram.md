# Frames live in PSRAM — implementation plan

**Spec:** `docs/superpowers/specs/2026-09-28-deskmate-frames-live-in-psram-design.md`
**Track:** C1 (branch `track-c1-tap-to-face`, PR #4)

> **RELEASE INTEGRATION 2026-09-29 (ROD-3).** Managed GitHub CLI access is
> restored (ROD-7). Local main's three Track D commits are preserved by merging
> main into C1, followed by origin/main (`4253521`, including Tracks A and B).
> Shared main will fast-forward to the PR merge; no shared history is rewritten.
> Conflict resolution preserves C1 views/taps, B plugin discovery/cadence/timezone,
> both tracks' board observations, and account-scoped rendering for staged views.
> A plugin without onTap must fall back to render, never select its old resting
> frame; regression coverage exercises this at the real subprocess boundary.
> **Execution hold:** the automated merge/deploy continuation was rejected by
> automatic approval review: relayed agent authorization does not override the
> direct assignment's deploy prohibition. Deployment needs direct owner confirmation.
> No flash or new schema/wire change is part of this integration.
> CI run 36619465888 passed all four required jobs on 25d572e, including sanitizer
> and doctests. Before release, main advanced to 8f105ea (packaging PR #10).
> This follow-up merges that additive packaging/docs change; fresh gates and CI
> are required for the resulting head. No merge to main or deploy is claimed.
> Integrated local gates PASS on `6d06181`: firmware host + sanitizer,
> Rust fmt/clippy/all-targets/doctests, web 216 tests + check/lint/format/build,
> faces 429 tests + check/lint/format. Exact logs and Rust counts are saved in
> ROD-3's verification document. CI found Track B's allocation-loop test racing
> Bun's 5s timeout on Linux (run 36618454766). Replaced that loop with one
> bounded allocation: OOM at 8 MiB, success at 32 MiB. Production code is unchanged;
> disabling the cap fails this test (observed). All faces gates pass again,
> including 429 tests; the final-head CI result is still required.
> No merge/deploy is claimed.
> The newer lab entry records a prior migration and views deploy; the older
> status below is historical. Glass latency is still unmeasured.
>
> **STATUS 2026-09-29. Every task is implemented; one is unverified on the board.**
> Step 1, Task D (the firmware pool and key 32, flashed and verified on `dev-0005`),
> Task C (every frame volatile, with pool-pressure reclamation), Task A (the faces
> `views()`/`onTap()` contract) and Task B (the server stages views; a tap on a staged
> view draws nothing) are all on `track-c1-frames-in-psram` with every gate green.
>
> **What is NOT done: the deploy, and therefore the tap measurement.** A/B ship the
> latency win and neither has been seen on hardware -- the tap on the glass is still the
> one thing this track has never observed. The deploy is held because the next one also
> carries Track A's account migration and changes how the web app signs in, which is the
> owner's call and not a peer's.
>
> **STATUS 2026-09-28, updated.** Step 1 is delivered (`784ff52`). **Task D is delivered
> and verified on hardware** (`68f15a8`): the owner authorized the flash, the board took
> `v2.2.0-psram` by OTA, and the pool reports `capacity 16` with commits at 76-81 ms where
> flash cost 1,528-1,678 ms and `Busy` refusals went from four to zero. See the 2026-09-28
> entry in `docs/hardware/board-notes.md`. **Tasks A, B and C remain**, and they are now
> unblocked: the pool they were waiting for exists on the fleet.
>
> The original status block, kept because its reasoning is what produced the corrections
> below: steps 2-4 were one batch needing the owner's authorization because it ends in a
> flash.
> The spec sequenced 2 and 3 as landable ahead of the firmware; execution found that they
> are not, for a reason the spec does not address — see **Correction 2** below. Nothing in
> steps 2–4 has been started, and no file outside step 1 has been touched.

## What this plan is for

The spec is right about the finding and right about the destination. What it gets wrong is
the claim that the host half can ship first and behave like today. This plan records what
was verified, fixes the sequencing, and leaves steps 2–4 specified tightly enough that the
flash session is one sitting.

---

## Corrections to the spec, from reading the code

### Correction 1 — key 31 is a flash number, not the PSRAM pre-flight number

The spec's Risks section says to decode `asset_store_free_bytes` to get the pre-flight
figure for the pool. It is not that figure. `firmware/main/link/protocol_task.c:530-539`
fills key 31 from `asset_store_stats(asset_flash_store(), ..)`, and
`firmware/main/core/asset_store.c:547` computes `free_blob_bytes` as
`blob_region_size - high_water` — the 6 MB **flash** partition. PSRAM occupancy is not on
the wire at all.

Decoding it was still worth doing, for the other half of the spec's claim: the durable
store has never held anything but picture frames, so `used_bytes` holding still across
refreshes is the observable end of the flash wear. That is what step 1 delivers.

**The PSRAM headroom figure needs a new status key**, which is a wire change. It belongs in
the flash session (step 4), not ahead of it.

And it cannot be read off the board today either, which makes the spec's memory budget
softer than it looks. `protocol_task.c:495` reports `free_heap` as
`esp_get_free_heap_size()` — the **total** across every capability, internal SRAM included.
The spec's arithmetic subtracts the 5.03 MB pool from that total ("8,358,839 B measured,
leaving about 2.94 MiB"), so the PSRAM-specific headroom is smaller than 2.94 MiB by
whatever internal SRAM contributes. The firmware calls `heap_caps_get_free_size` **nowhere**
— grep confirms it — so no log line answers the question either. It is still comfortable
arithmetic, but the pool's actual margin is unmeasured, not measured, and the risk that
matters (sixteen 329,740-byte allocations succeeding in a heap mbedTLS also draws from) is
untouched by the figure the spec quotes.

### Correction 2 — "every frame volatile" is wrong until the pool is real

The spec says step 3 can land ahead of the firmware because it is "still limited to two
slots until the firmware ships — so land it behind the budget rule, which will stage one
view per card and behave like today."

It will not behave like today, and the budget rule cannot make it:

- `firmware/main/core/volatile_asset_store.h:16` is `VOLATILE_ASSET_SLOT_COUNT 2U`. Its
  comment says that is "one displayed frame plus one incoming replacement", and **that
  comment describes intent, not the store's bound** -- measured on `dev-0005` on
  2026-09-28, the two-slot store committed **two** frames (76 ms and 97 ms) and refused
  the third with `ERR_FULL`. This plan said "one" before the board was asked; the
  argument below is unaffected, because two is still fewer than four picture cards.
- The budget rule bounds **views per card**, not cards. With four picture cards its share
  is `max(1, floor(15 / 4))`, so it asks for one resident frame per card — four frames into
  one slot.
- The device's carousel advances on its own every 50 s. A frame that is only volatile and
  has been evicted is a card with nothing to draw when its turn comes. `docs/hardware/board-notes.md`
  already names this failure from the other direction: "A card going blank. The volatile
  tier holds **one** committed frame, so tapping a second picture card before the next
  reconcile evicts the first."

So today's durable-for-non-visible behaviour is not a legacy policy to be removed early —
at one resident slot it is the **correct** policy, and the only thing that makes "every
frame volatile" correct is the 16-slot pool. Steps 2, 3 and 4 are one change.

### Correction 3 — the `Busy` backstop is already handled

The spec's Risks section asks that `Busy: volatile asset reserve failed` be "handled, not
merely logged". It already is, and no work is owed. `firmware/main/link/protocol_task.c:1008-1013`
answers `Busy` for exactly one reason on a volatile `AssetBegin` —
`VOLATILE_ASSET_STORE_ERR_FULL` — and on the host
`runtime/scene.rs:174-179` treats `Rejected(Busy)` as neither a card error nor a transport
failure: it marks the scene dirty, and the next render phase reconciles the frame durably
and pushes it. The cost of a refusal is one render phase, not a picture.

An inline durable fallback was considered and rejected: it moves a 1.5 s commit (and
possibly a 9.4 s compaction) onto the tap path to save one render phase, and the retry it
replaces already works.

---

## Step 1 — the host reads the device's asset-store stats (DELIVERED, `784ff52`)

- [x] `protocol`: `AssetStoreStats { used_bytes, free_bytes, asset_count }`, an
      `Option` field on `StatusResponse`, encode and decode of key 31 as a nested map.
      All-or-nothing, matching the firmware's own decoder — a partial map is a malformed
      frame, not a device with less to say.
- [x] `app-core`: `DeviceAssetStore` on `DeviceSnapshot` (`#[serde(default)]`), filled in
      `runtime/link.rs`'s `update_device_status`.
- [x] `server`: `asset_store` added to `change_key`'s blanked telemetry — its tallies move
      on every frame a face draws, and that list exists so numbers nothing renders cannot
      re-render the page.
- [x] The browser seam: contract fixture regenerated, `DeviceAssetStore` declared in
      `apps/deskmate/src/lib/types.ts`, mock harness fixture carries the shipped shape.
- [x] Gates: `fmt`, `clippy -D warnings`, `cargo test --workspace --all-targets`,
      `--doc`, `bun test`/`check`/`format:check`/`build`, faces `test`/`check`. All green.
- [x] Mutation-probed: each of the three sub-keys was separately defaulted to zero and the
      test caught all three. The first version of the test caught none of them but one —
      a single partial map pins only the last field read, and a decoder that zero-filled
      `used_bytes` would report a full store as an empty one.

**Not claimable:** nothing here has been seen on the board. The field is decoded and
carried; whether `dev-0005` reports it is unobserved.

---

## Steps 2–4 — one batch, owner-authorized, ending in a flash

Order within the batch matters (the faces package deploys separately from the binary), but
none of it should reach the live server before the flash, because the host half is wrong
at one resident slot.

### Task A — the faces contract: `views()` and a pure `onTap()`

Faces-only; `deploy.sh --faces-only`; no Rust.

**Task A cannot be landed on its own either, and the reason is sharper than "it would have
no consumer".** Every face's tap behaviour lives *inside* `render` today — `weather.ts`
flips its view in `renderWeatherResult`, `rss` and `hackernews` advance a page, `token`
rotates `TAPPABLE_CHARTS`. Moving that into `onTap` while the server still calls only
`render` leaves two possibilities, both worse than waiting: the logic exists twice and can
drift, or `onTap` is the version nothing runs and the one that ships is still the old one.
The contract has to arrive with the caller that uses it.

- [ ] `faces/src/face.ts`: add `views?(settings): ViewId[]` (priority order, first is the
      resting view) and `onTap?(state, event, views): ViewId` (pure — no fetch, no draw).
      `render` gains the selected view in `RenderContext`. A face with no `views()` keeps
      today's behaviour exactly.
- [ ] `faces/src/main.ts`: `describe` stays the static catalog. **Two new verbs are needed,
      not one**: `views` (`{kind, settings}` → the list, because `views()` depends on
      settings and so cannot live in `describe`) and `tap` (`{kind, settings, state, event}`
      → the selected `ViewId`, because the server must resolve a tap without rendering).
- [ ] The four faces express what they already switch between: `weather` now/days,
      `hackernews` and `rss` their page cycle, `token` line/candles. Each face's private
      view enum becomes its `views()`; the paging arithmetic becomes `onTap`.
- [ ] Tests: `views()` matches what `onTap` can return for every face; `onTap` is pure,
      asserted by handing it a fetch that panics; the existing golden SVGs do not move.

### Task B — the server stages every declared view

- [ ] `image_sources.rs`: a source holds several frames, keyed by view, instead of one.
      **No store schema bump is needed** — `load_frame` probes `image-frames/<id>.bin` and
      re-hashes it, and the JSON never records the frame at all, so extra views can be
      `image-frames/<id>--<view>.bin`, discovered by listing the directory, with a v1
      `image-sources.json` still valid. Bounded count per source; a v1 store with one file
      loads as one resting view.
- [ ] The resting view keeps the existing `<id>.bin` path, so every producer's POST and
      every existing frame on the live VM is untouched.
- [ ] `data_cards/worker.rs`: the refresher renders each declared view per refresh, within
      the card's share, and stages each.
- [ ] A tap resolves through the `tap` verb to a staged digest and pushes **only** a scene
      — no `AssetBegin`, no chunks.
- [ ] `frame(source_id)` must return the view the device should be showing, so the store
      needs a selected view per source, set before the runtime is notified. It need not
      persist: after a power cycle the resting view is correct, and the spec already accepts
      that a volatile frame does not survive a reboot.
- [ ] The digest ceiling is the real bound to respect: `MAX_ASSET_DIGESTS` is 32 and
      `asset_sync.rs` refuses a desired set larger than it. The existing
      `const _: () = assert!(MAX_IMAGE_SOURCES <= protocol::MAX_ASSET_DIGESTS)` in
      `image_sources.rs:38` is no longer the right invariant once a source has several
      frames; the staged total is what must fit.

### Task C — the host puts every picture frame in PSRAM

- [ ] `runtime/mod.rs`: `apply_image_source_update` stops choosing a tier by visibility.
      The capacity is **one named host constant** (15 resident frames after the flash), and
      this task is meaningless below it — see Correction 2.
- [ ] `asset_sync.rs`: reconcile learns a volatile desired set alongside the durable one;
      the keep-set is durable ∪ volatile. Note the keep-set is already documented as
      spanning both tiers (`docs/protocol/v2.md`), and `compose_asset_keep_set` — named in
      the spec — **does not exist**: the keep-set is built inline in `reconcile_releasing`.
- [ ] The budget rule: each picture card's share is `floor(15 / picture_cards)`, minimum
      one, leftovers to the cards that declared the most views, in loop order. Views past a
      card's share fall back to being rendered on the tap that asks for them.

### Task D — the firmware, the wire, and the flash

**Owner authorization required.** Batch with C2's tap coordinates: one image, one session.

- [ ] `VOLATILE_ASSET_SLOT_COUNT` 2 → 16, and its comment stops describing the two-slot
      bound as the only safe one.
- [ ] A new `StatusResponse` key for **volatile** store occupancy **and PSRAM-specific free
      size** (`heap_caps_get_free_size(MALLOC_CAP_SPIRAM)` plus its minimum-ever
      counterpart), so the pre-flight number Correction 1 says is missing exists. Additive,
      and it is the one piece of the wire change that pays for itself immediately. Reporting
      only the store's own occupancy would not answer the fragmentation risk; the heap's
      free and low-water figures are what do.
- [ ] `docs/protocol/v2.md`: the two-allocation sentence becomes the sixteen-allocation
      sentence, and the undocumented `Busy` reserve refusal is written down.
      `PROTOCOL_CURRENT_CAPABILITIES` does not move — bit 9 already means what it means.
- [ ] `make -C firmware/host_tests clean test` **and** `sanitize`; the volatile store fills,
      evicts by keep-set, and refuses a seventeenth reservation without losing a committed
      frame.

**On the board, and not claimable without it:** an OTA download after the image changes
(mandatory — statics move); the heap low-water mark and the device's own volatile occupancy
with the pool full; tap-to-redraw at both mountings; a release with an empty durable store,
timed; and a server-rendered face on the panel, which has never been observed.

---

## What the owner is being asked to decide

Steps 2–4 are one batch that ends in a USB flash and a mandatory on-board OTA
re-verification, and they are the only way the 16-slot pool becomes real. Step 1 shipped
without needing that. Nothing between the two is worth landing on the live server on its
own: at one resident slot, "every picture frame is volatile" means cards that go blank when
the carousel reaches them.
