# Frames live in PSRAM

Status: **proposed**. Supersedes the storage half of
`2026-09-23-deskmate-tap-to-face-design.md`; that spec's seam (the tap reaching the face,
the render envelope, per-source face state) is unchanged and still current.

This crosses the **wire contract** and **firmware**, two of the three expensive
boundaries. It does not touch `CURRENT_SCHEMA_VERSION`. It needs the owner's explicit
authorization and one flash session, batched with C2.

> **AMENDED 2026-09-28 during implementation, then DELIVERED IN PART.** Step 1 shipped
> (`784ff52`) and **step 4 shipped and is verified on hardware** (`68f15a8`): `dev-0005`
> took `v2.2.0-psram` by OTA and reports `capacity 16`, with the flash commit and the
> compaction stall both gone. Steps 2 and 3 remain. The measured figures, including the
> PSRAM headroom this spec could only estimate, are in `docs/hardware/board-notes.md`.
>
> Step 1 shipped (`784ff52`). Two claims below
> did not survive contact with the code, and both are corrected in place where they appear:
> key 31 is a **flash** number and not the PSRAM pre-flight figure this spec takes it for
> (see *Risks*), and steps 2–3 **cannot** be landed ahead of the firmware "behaving like
> today" (see *Sequencing*) — they are one batch with step 4. The plan carries the full
> reasoning: `docs/superpowers/plans/2026-09-28-frames-live-in-psram.md`.

## The finding

The device has two asset tiers. The host has only ever used one of them, and it used the
wrong one.

- **Durable**, in a 6 MB flash partition. Stores the RLE565 wire bytes, memory-mapped for
  drawing (`asset_flash_map`), so it costs no RAM to display.
- **Volatile**, in PSRAM. Stores a decoded 329,740-byte RGB565 frame. Capability bit 9,
  `volatile_asset_store.c`, complete since `cfb6317` on 2026-09-01, and resolved *before*
  flash by `protocol_asset_resolver` with the comment: "Volatile PSRAM wins before flash so
  an atomic replacement can be rendered without ever spending partition endurance."

`AssetBegin`'s `volatile` key was hardcoded `false` at `asset_sync.rs:148` from the day the
tier was written. Three call sites passed `None` for the active volatile digest; the host
half was deleted twice as dead surface (`423ca87`, `7532cbc`, both 2026-09-19). No raster
frame reached PSRAM until 2026-09-27.

**Ephemeral data went to the durable tier, and the tier built for ephemeral data sat
unused for twenty-six days.** Every cost this track has chased is rent on that inversion.

### What durable buys for a picture frame: nothing

Measured and checked on `dev-0005`:

- **The config declares `assets: []`.** No fonts, no icons, no images. The durable store
  has held nothing but picture frames.
- **Nothing could declare one usefully.** Every font the host emits is
  `SceneFont::Baked(..)`; `scene_build.rs` contains no `SceneFont::Asset`. The durable tier
  has no reachable user.
- **The cache is never read.** The board was away 2026-09-26 21:10:52 → 2026-09-27
  18:52:26, twenty-one hours and forty-two minutes. On reconnect all four frames
  transferred in full: `already_present` was false for every one. The faces re-render every
  fifteen minutes and every render yields a new digest, so **a durable frame's useful life
  is at most one refresh interval**, and `CLAUDE.md` records that the board is normally
  powered off. The frames are written, compacted, and superseded before they are displayed.

### What it costs

| | durable | volatile | measured |
|---|---|---|---|
| `AssetCommit` | ~1,510–1,608 ms | **73–74 ms** | 2026-09-27 journal |
| `AssetRelease` after a replacement | 9,359–10,437 ms | n/a | 2026-09-26/27 |
| flash endurance | 4 records rewritten per 15 min, forever | none | -- |

The 9.4 s is `asset_store_plan_compaction` moving every record that sits after the one a
replacement orphaned. For 9.5 s of it the panel shows its own clock, which is what the
owner reported on 2026-09-24 as the device "switching into autonomous mode". It was never a
lost link.

## The decision

**A picture frame is a volatile asset. Always.** Not "the visible card", not "tap-driven
updates" -- both of those are policies over a two-slot tier and both fail, as
2026-09-27 proved: ordinary background refreshes of whichever card was on screen consumed
both slots, and the owner's tap was refused with
`Busy: volatile asset reserve failed` and fell back to flash, paying the old price plus a
wasted round trip plus a full re-reconcile.

**The durable tier reserves nothing.** It stays in the firmware and on the wire because it
already exists and costs nothing idle. If a custom font or a user-uploaded image ever
arrives it draws from the spare digests. The design does not budget for it.

## The pool

`VOLATILE_ASSET_SLOT_COUNT` moves from **2 to 16**. The slots are **one shared pool across
every card, not a per-card ration** -- three cards of five views, or one card of fifteen.

- **16 slots × 329,740 B = 5.03 MB.** Measured free heap on `dev-0005` is **8,358,839 B**
  (7.97 MiB), leaving about 2.94 MiB. PSRAM is 8 MB octal at 80 MHz, confirmed in the boot
  log; the LVGL draw buffers (58,880 B × 2) are in internal DMA-capable SRAM, not PSRAM, so
  this pool is not competing with them.
- **Fifteen of the sixteen are resident.** The store's design is *n* committed plus one
  incoming atomic replacement, so a count of 16 holds fifteen frames while a sixteenth
  arrives. Seventeen slots would hold a clean sixteen for 5.6 MB; the recommendation is 16
  because that is the figure the memory budget was approved against.
- **Memory is the only justification.** An earlier draft argued that 16 volatile plus 16
  durable exactly fills `AssetRelease`'s 32-digest cap. That was a rationalisation built on
  a reservation for a tier with no users. The cap is headroom: fifteen used, seventeen
  spare.

### When a face declares more views than its share

A face declares its views **in priority order**. Each picture card's share is
`floor(15 / picture_cards)`, minimum one; slots left over after that division go to the
cards that declared the most views, in loop order, so nothing is stranded. Views past a
card's share fall back to the present behaviour -- rendered and transferred on the tap that
asks for them. A card is never refused for declaring too many; it simply gets slower views
at the end of its list.

## The faces contract

Today a face has one entry point that fetches, draws, and interprets taps at once. Split
it:

```ts
export interface FaceDefinition {
  kind: string;
  label: string;
  fields: FieldSpec[];
  tap?: string;
  /** The views this face offers, in priority order. One is the resting view. */
  views?(settings: Settings): ViewId[];
  /** Pure: no fetch, no draw. Decides which view a tap selects. */
  onTap?(state: unknown, event: TapEvent, views: ViewId[]): ViewId;
  /** Fetches and draws one view. Called once per view per refresh. */
  render(settings: Settings, now: Date, context?: RenderContext): Promise<RenderResult> | RenderResult;
}
```

This keeps the 2026-09-23 decision that **the face owns what a tap means**, while taking
the fetch and the raster off the tap path. `onTap` is pure, so the server can answer a tap
by looking up a digest it already staged.

A face with no `views()` keeps today's behaviour exactly: one view, rendered on demand.
The weather, RSS and token faces each gain a `views()` that returns what they already
switch between.

## The wire contract amendment

`docs/protocol/v2.md` currently freezes the tier:

> The device holds at most two decoded PSRAM allocations: the displayed frame and one
> incoming atomic replacement.

becomes:

> The device holds at most sixteen decoded PSRAM allocations: fifteen resident frames and
> one incoming atomic replacement. The host learns the true figure from the device rather
> than assuming it; a device that cannot reserve a slot answers `AssetBegin` with `Busy`
> and the diagnostic `volatile asset reserve failed`.

That last sentence is not new behaviour -- it is what the firmware already does, observed
on 2026-09-27 -- but it is undocumented, and a host that grows the pool must handle it.

`PROTOCOL_CURRENT_CAPABILITIES` does not move: bit 9 already means "accepts and resolves
volatile image assets in PSRAM". The capacity is not on the wire.

## Host changes

| file | change |
|---|---|
| `app-core/src/asset_sync.rs` | `transfer_volatile` already exists (2026-09-27). Reconcile learns a volatile desired set alongside the durable one; `compose_asset_keep_set` returns durable ∪ volatile. |
| `app-core/src/runtime/mod.rs` | `apply_image_source_update` stops choosing a tier by visibility. Every picture frame is volatile. |
| `server/src/data_cards/` | The refresher renders every declared view per refresh and stages each. A tap resolves through `onTap` to a staged digest and pushes only a scene. |
| `faces/src/face.ts` | `views()` and `onTap()` as above. |

**The release cadence becomes unnecessary for frames.** `ASSET_RELEASE_INTERVAL` (30
minutes, added 2026-09-27) exists because reclaiming compacted flash. Releasing a *volatile*
digest frees PSRAM and orphans no durable record, so `asset_store_plan_compaction` should
emit no moves and the release should cost milliseconds. Expected, not measured -- the 9.4 s
figure was always a record dying mid-region. Keep the cadence until a release with an empty
durable store has been timed on the board.

## What changes for the owner

- **A tap redraws in roughly one round trip.** Projected ~80 ms over the tunnel from the
  measured 76–88 ms chunk round trip; ~5 ms if the board is ever pointed at the LAN.
  Unmeasured.
- **After a power cycle a card shows "Waiting for the first picture"** until the host
  pushes, instead of a stale frame. In practice this changes a few seconds of a day-old
  picture into a few seconds of an honest placeholder, because those frames are already
  replaced on every reconnect.
- **Background wire traffic multiplies by the number of views**, entirely off the
  interactive path.
- **Flash stops being written for frames**, ending the wear.

## Rejected

- **Volatile for the visible card only.** Shipped 2026-09-27 and refuted the same evening:
  the carousel makes a different card visible every 50 s, so scheduled refreshes exhaust
  two slots without anyone tapping.
- **Volatile for tap-driven updates only.** Survives longer but still collides when two
  cards are tapped inside one refresh window, and leaves the flash wear untouched.
- **Storing volatile frames compressed.** Would cost ~32 KB a slot instead of 330 KB, but
  adds an RLE decode to every draw. With 8 MB idle, RAM is the cheaper resource.
- **Repartitioning the now-idle 6 MB assets partition.** Out of scope: repartitioning is a
  full-flash operation and OTA does not update the bootloader. Noted so the next reader
  knows the space is there.

## Risks

- **PSRAM fragmentation is unverified.** 5.03 MB fits arithmetically, but sixteen
  329,740-byte allocations in a heap that mbedTLS also draws from is a different question
  from "is there enough free". **Pre-flight: watch the heap low-water mark and the device's
  own volatile occupancy with the pool full.**
  **CORRECTED 2026-09-28: key 31 is not that number.** This spec took
  `asset_store_free_bytes` for the PSRAM figure; `protocol_task.c:530-539` fills key 31 from
  `asset_store_stats(asset_flash_store(), ..)` and `asset_store.c:547` derives its
  `free_blob_bytes` from the **flash** blob region's high water. PSRAM occupancy is not on
  the wire at all. Key 31 is decoded now (`784ff52`) because it answers a different question
  this spec also asks -- whether the flash is still being written for frames -- and the
  volatile pre-flight figure needs a **new** status key, which is part of the step 4 wire
  change rather than something that can precede it.
- **Any firmware change costs an on-board OTA re-verification**, per the statics trap. One
  mitigation is already true: `volatile_asset_store_t` lives inside `s_context`, which is
  `heap_caps_calloc(..., MALLOC_CAP_SPIRAM)`, so growing the slot array grows a PSRAM heap
  allocation rather than `.bss`.
- **A frame that is only volatile vanishes on reboot.** Accepted above, and the reason it
  is safe is that the host re-pushes on every link. If that ever stops being true, this
  decision must be revisited.
- **`Busy: volatile asset reserve failed` must be handled, not merely logged.** With a
  pool the host can exhaust it by declaring too many views; the budget rule above is what
  prevents that, and the refusal is the backstop.
  **CORRECTED 2026-09-28: it is already handled, and no work is owed here.**
  `runtime/scene.rs:174-179` treats `Rejected(Busy)` as neither a card error nor a transport
  failure -- it marks the scene dirty, and the next render phase reconciles the frame
  durably and pushes it. A refusal costs one render phase, not a picture. An inline durable
  fallback was considered and rejected: it would move a 1.5 s commit onto the tap path to
  save that phase.

## Testing

- `asset_sync`: a volatile transfer sends no release and carries `volatile: true`; the
  keep-set is durable ∪ volatile; the budget rule stages a face's views in priority order
  and stops at the share.
- `runtime`: a picture update reaches the glass before any flash work (exists, 2026-09-27);
  a tap on a staged view pushes **only** a scene -- no `AssetBegin`, no chunks.
- `faces`: each face's `views()` matches what `onTap` can return; `onTap` is pure, asserted
  by giving it a fetch that panics.
- Firmware host tests: the volatile store fills, evicts by keep-set, and refuses a
  seventeenth reservation without losing a committed frame. `make sanitize` is not optional.
- **On the board, and not claimable without it:** tap-to-redraw at both mountings; the heap
  low-water mark with the pool full; an OTA download after the image changes; a release
  with an empty durable store, timed.

## Sequencing

1. Decode `StatusResponse` key 31 (`asset_store_used_bytes`, `free_bytes`, `asset_count`).
   Host-only, additive, no flash. **DONE 2026-09-28, `784ff52`.** It gives the flash-wear
   number, not the PSRAM one -- see the correction under *Risks*.
2. Faces contract: `views()` + pure `onTap`. Faces-only deploy, no Rust, no restart.
3. Host: every frame volatile, keep-set spans both tiers, budget rule. Server deploy.
4. Firmware: `VOLATILE_ASSET_SLOT_COUNT` 2 → 16, contract amendment, flash, OTA
   re-verification. **Batched with C2's tap coordinates: one image, one session.**

**CORRECTED 2026-09-28: steps 2–4 are one batch, and step 1 was the only one that could
ship alone.** This section said step 3 could land first "behind the budget rule, which will
stage one view per card and behave like today". It cannot, for a reason stated nowhere else
in this spec: `VOLATILE_ASSET_SLOT_COUNT` was 2, which the board showed holds **two**
committed frames before refusing the third (the header's "one displayed frame plus one
incoming replacement" was intent, not the bound) -- and the budget rule bounds views per
*card*, not cards, so with four picture cards it asks for four resident frames. The device's carousel then advances on its own to a card
whose only copy was evicted, which is the blank card `docs/hardware/board-notes.md` already
warns about from the other direction. Durable-for-non-visible is therefore not a legacy
policy to remove early; at one resident slot it is correct, and only the 16-slot pool makes
"every picture frame is volatile" true. Step 4 needs the owner's authorization, so the whole
remaining batch does.
