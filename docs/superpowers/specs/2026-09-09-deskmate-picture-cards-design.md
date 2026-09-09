# Picture cards — a card whose face is a PNG somebody sent us

**Status:** approved direction (owner, 2026-09-09). The owner's finding is that adding a
card the manifest way costs hours of work that produces no product value, and that the
main card type should instead be a server-side picture: "if the plugin — or I should say
card — is not interactive, does not require taps and doesn't update every second, then we
just do it on the server."
**Branch:** `feat/picture-cards` (worktree `.worktrees/picture-cards`), off `main` once
`chore/debt-cleanup` merges; off `chore/debt-cleanup` if it has not.
**Builds on:** stage 4's rasterization pipeline
(`docs/superpowers/plans/2026-08-29-deskmate-rasterization.md`), the asset store
(`docs/superpowers/plans/2026-08-22-deskmate-asset-store.md`), the one-loop UI
(`docs/superpowers/plans/2026-09-06-deskmate-one-loop.md`), schema v6
(`docs/config/v6.md`).

## 1. The problem, precisely

Adding a card today means authoring a plugin manifest, and the manifest is not the
expensive part — the proof obligations around it are. Shipping `claude-limits` required
golden frames, an entry in the parity ledger, fixtures, a machine-checked bounds section
in a frozen document (`crates/plugin/tests/manifest_v2_doc.rs`), and two additions to the
expression language (`scale()`, `time_at()`) with a load-bearing rollout order — all
because the expression language deliberately has no arithmetic and no way to render an
instant in the user's zone.

That cost is paid per card and does not amortise. It also gates card authoring on this
repository: nobody outside it can add a card at all.

Meanwhile the machinery to display an arbitrary 448x368 frame already exists, shipped, and
passed on hardware on 2026-09-06: `resvg` produces a canonical RGB565 blob, `AssetSync`
moves it, RLE565 compresses it, the asset store holds it, and `SceneImage` draws it at both
orientations. What is missing is not rendering. It is a way for bytes to *arrive*.

## 2. Goals and non-goals

**Goals.**

- A new card kind whose face is a PNG a producer sends to a webhook.
- Producers are arbitrary and external: one line of `curl`, no SDK, no manifest, no
  repository access.
- Durable, digest-addressed frames, so rotating the loop onto a picture card moves no
  bytes.
- Staleness without configuration.
- `claude-limits` converted to a picture card, proving the path end to end.
- No firmware change, no protocol change, no capability bit.

**Non-goals for this sub-project.**

- **Converting weather / RSS / JSON feed to server-rendered pictures.** That is where the
  owner's rule of thumb actually gets applied, and it depends on this card kind existing.
  Separate spec.
- **Interactivity on picture cards** (tap -> server -> new picture). Wanted, explicitly
  deferred; nothing here forecloses it, and §9 records the constraint it must respect.
- **Public plugin uploads and sandboxing.** Still the scene spec's stage 5, still gated on
  its own risk review.
- **Retiring the manifest path.** Manifests are frozen, not removed — see §8.

## 3. What is being decided, and what was decided instead

| Decision | Chosen | Rejected, and why |
| --- | --- | --- |
| Ingest format | **PNG only** | SVG from strangers is a far larger attack surface than PNG: entity expansion, external references, font loading. The existing SVG hardening was built for curated content. SVG stays server-authored. |
| Addressing | **Named image source, token-addressed** | Card-addressed (`/devices/{d}/cards/{c}/image`) couples the producer to a card id that changes when a card is deleted and re-added. |
| Asset lifetime | **Durable, digest-addressed** | Volatile has two PSRAM slots (`VOLATILE_ASSET_SLOT_COUNT`, `volatile_asset_store.h:16`) — enough for one regenerating card, not a loop. |
| Staleness | **Inferred from observed cadence** | A declared interval is friction every external producer pays forever, and half would declare it wrong. Bounds in §7 are what keep inference from being guesswork. |
| Source creation | **Creating a picture card creates its source** | "Create a source, then create a card that uses it" is the same empty step as "add to library, then place in playlist", which the one-loop change deleted. Reuse stays available; it is not the default path. |
| Implementation shape | **Image sources as their own server subsystem** | Modelling a source as a degenerate plugin would have been half the code, but routes the new primary card type through the abstraction being frozen this same week. |

## 4. Data model and schema v7

`AppConfig` (`app-core/src/config.rs:481`) gains one document-level field, a peer of
`cards` because a source is reusable across cards and devices:

```rust
pub struct AppConfig {
    pub schema_version: u32,              // 7
    pub preferences: AppPreferences,
    pub cards: Vec<CardSettings>,
    pub image_sources: Vec<ImageSource>,  // new
    pub assets: Vec<AssetSettings>,       // still deliberately unwired
    pub playlists: Vec<Playlist>,
    pub active_playlist_id: String,
    pub updater: UpdaterSettings,
}

pub struct ImageSource {
    pub id: String,
    pub name: String,
}
```

`ImageSource` carries authoring identity and nothing else. **No credential enters the
config.** The token digest lives in the server's own store keyed by source id, exactly as
device identities do (`server/src/registry.rs:490`): digests only, plaintext returned once
at mint and never persisted.

The card mirrors `CardSettings::Plugin` (`config.rs:1280`) field for field, because it is
the same species — a card whose face the server owns:

```rust
Picture {
    id: String,
    title: String,
    source_id: String,
    tap_action: WidgetTapAction,
    refresh: RefreshPolicy,
    alert: CardAlert,
}
```

`template` is deliberately absent, as it is for `Plugin`. On the wire the card compiles to
`template: DigitalClock` like every plugin card, so **the device learns nothing new**.
`cardLabel()` returns "Picture"; `cardTitle()` remains the owner's words, per the naming
rule.

**Server-side store**, separate from config:

```
source_id -> { token_digest: [u8; 32], frame_path: PathBuf, recent_push_times: [DateTime<Utc>; <=8] }
```

`frame_path` holds the **decoded canonical RGB565 blob**, not the PNG, so a reconnect or
factory reset re-syncs without re-decoding and the digest stays stable. `recent_push_times`
is **wall-clock, not `Instant`**, because the ring is persisted across restarts and a
monotonic clock does not survive one.

**Migration v6 -> v7 is purely additive**: `image_sources: []`, no card rewriting, nothing
lossy. Older files round-trip unchanged, per `docs/config/v6.md`'s migration contract.

**Two rules, stated rather than left implicit:**

1. A card naming an unknown `source_id` is a **typed validation error**, never a silently
   blank card.
2. The desired-asset set is a **union**: plugin registry assets united with every image
   source's frame, computed by one function with one caller. `AssetRelease.digests` is a
   device-wide *keep-set* and an omission means delete — this is the exact shape of the
   empty-registry wipe fixed during stage 3b.

**Operational trap inherited from every schema bump:** the deployed server compiles its own
`CURRENT_SCHEMA_VERSION` (`config.rs:355`), so **redeploy the server before saving a v7
config**, or every save fails with "schema version 7 is not supported".

## 5. Ingest and auth

**Producer-facing route:** `POST /v1/images/{token}`, `Content-Type: image/png`, body is
the picture. The token *is* the address — it identifies the source, so there is nothing
else to look up.

```
curl -X POST --data-binary @panel.png \
     -H 'Content-Type: image/png' \
     https://deskmate.rodi.one/v1/images/7f3a...c21
```

**Accepted risk, recorded deliberately:** a token in a path appears in Caddy and Cloudflare
access logs. The blast radius is bounded by construction — a source token is write-only,
scoped to one source, reads nothing, and is revoked independently of every other
credential. The same token is **also accepted as `Authorization: Bearer`**, verified by the
same function, for producers that prefer to keep it out of a URL. One digest check, two
carriers.

**Verification** follows the device-identity precedent: SHA-256 digest stored,
constant-time compare.

**Validation order is the security-relevant part.** The header is read and rejected on
dimensions *before* anything is allocated; that, not the byte cap, is what stops a
decompression bomb.

1. `DefaultBodyLimit::max(1 MiB)`, matching the device's own `ASSET_MAX_BYTES`
   (`firmware/main/core/asset_store.h:16`), layered per-route as `put_config` already does
   (`server/src/admin.rs:37`).
2. Parse the PNG header only. Require **exactly 448x368**, 8-bit, RGB or RGBA. Reject
   before decode.
3. Decode. RGBA is composited over black; the panel has no alpha.
4. Convert to the canonical 12-byte LVGL header plus RGB565 body, **reusing
   `server/src/rasterizer.rs`'s existing canonicalization** so there is one implementation
   of that format.
5. Digest over the **decoded** blob, per stage 4's rule.

The decoder is the `png` crate, already in the lockfile via `crates/lvgl-sim` — no new
vendor.

**Two behaviours that carry more weight than they look:**

- **An identical picture is a no-op.** If the new digest matches the stored one, record the
  push time for cadence and return 200 without touching the device. Producers on a timer
  push unchanged pictures constantly.
- **A per-source minimum accepted interval** of `MIN_PUSH_INTERVAL = 5 s` returns 429, so a
  runaway producer cannot grind the asset partition. Five seconds sits far below any
  legitimate cadence (`claude-limits` pushes every ~10 minutes) and above an accidental
  double-post; it is a named constant with a boundary test, like the staleness bounds.

**Errors are typed and written for the producer**, one line each, no internals: 401 bad or
revoked token, 413 too large, 415 not a PNG, 422 wrong dimensions or bit depth, 429 too
fast.

**Two admin-bearer routes complete the surface:** `POST /v1/images` mints a source and
returns the plaintext token **once**; `DELETE /v1/images/{id}` revokes it and drops its
frame from the desired set.

**Capacity.** `MAX_IMAGE_SOURCES = 8`. Eight frames is ~2.6 MB of the 6 MB `assets`
partition and 8 of the 31 durable digests (`MAX_DURABLE_REGISTRY_ASSETS`,
`server/src/plugin_registry.rs:31`); the frozen plugins keep the rest.

Frames persist through `app_core::secure_file` — atomic, `0600` — made public for exactly
this class of use by V3 sub-project 1.

## 6. Runtime and push path

`build_card_scene` (`app-core/src/runtime.rs:2766`) gains a `Picture` arm beside the
existing `Plugin` arm at `:2780`. It resolves `source_id` to a digest through the injected
host and returns a `DisplayList` scene that is one full-canvas `SceneImage` (two nodes when
the source is stale — see §7) — the shape
`raster_frame_push` (`runtime.rs:3505`) already builds and validates. **That shape is
extracted into a shared helper** so the picture card and the raster path have one
definition of "a frame as a face", not two that can drift.

**Negotiation returns `Native` by construction**: no live bindings, `image` is a supported
node kind, the asset is durable and resident. Picture cards never enter the raster path,
never consume a volatile slot, and are never subject to `RASTER_MIN_INTERVAL`
(`app-core/src/scheduler.rs:8`). Those stay reserved for what stage 4 built them for.

**The webhook reaches the runtime as a command**, modelled on `InjectPluginSnapshot`
(`app-core/src/commands.rs:92`), the existing precedent for the server injecting data into
the worker:

```rust
ImageSourceUpdated { source_id, digest, bytes, reply }
```

Handled in this order:

1. Replace that source's frame in the desired set.
2. Reconcile durable assets — **bytes before any digest is referenced**, the same ordering
   rule `synchronize_full` obeys by reconciling before `apply_layout`.
3. Mark `active_scene_dirty` **only if the visible card subscribes to that source**.
   Otherwise the frame is simply resident and the loop uses it when it next lands there.

**Rotation costs nothing**, which is the whole payoff of durable: landing on a picture card
pushes a ~100-byte scene naming a digest the device already holds.

**Failure keeps the last good face.** If the asset transfer fails, no scene is pushed, the
previous picture stays on the panel, and nothing is released until the new push succeeds —
stage 4's rule, unchanged.

**Before a source has ever been pushed to**, the card renders a text-only scene reading
**"Waiting for the first picture"** — a state *word*, not the "No data yet" badge, because
there is no frame at all. Same distinction already drawn for plugin cards awaiting a first
refresh.

**In the Mac app** a picture card behaves exactly like a plugin card: the existing
`GET /v1/devices/{id}/cards/{card_id}/preview` route returns the stored frame re-encoded to
PNG through the existing `rasterizer.rs:1191` `frame_png`, and in local tier the card is
flagged `needs the server`. No Mac-side decoder, no second rendering path.

## 7. Inferred staleness

Each source keeps a bounded ring of its last 8 **accepted push times** — including no-op
pushes where the picture was identical, because a producer sending the same picture is
still alive. The ring is persisted with the frame, so a redeploy does not blind every card
until cadence is relearned.

```
intervals = consecutive diffs of the ring      (requires >= 3, i.e. 4 pushes)
baseline  = median(intervals)
deadline  = clamp(3 * baseline, 15 min, 48 h)
stale     = now - last_push > deadline
```

- **Fewer than 3 intervals: never stale.** A cadence cannot be inferred from one gap.
- **Median, not mean**, so one late push does not permanently inflate the deadline.
- **The clamp is what makes it predictable.** A per-minute producer is not flagged after
  three minutes of silence; a daily producer is not given three days of rope.

`STALE_MULTIPLE = 3`, `STALE_FLOOR = 15 min`, `STALE_CEILING = 48 h` are named constants,
each pinned by a boundary test, so changing one is deliberate rather than drift.

**Presentation reuses what exists.** A stale picture card's scene becomes the image node
plus the same stale footer every other face draws via `with_scene_data_state`. Two
consequences, accepted rather than discovered:

- The footer **overlays the picture**; a picture is full-bleed and there is no reserved
  strip. This happens only when something is wrong, and being visibly wrong beats being
  quietly wrong. Producer guidance suggests keeping the bottom strip calm.
- A staleness transition costs **one ~150-byte scene push and zero asset movement**.

**Recovery is immediate**: any accepted push clears stale and re-pushes if that card is on
screen. Evaluation rides the existing scheduler tick — a comparison per source, not a timer
per source. In the Mac app this surfaces through the `stale` flag already on card tiles
(`lib/providers.ts`), so no new UI vocabulary is introduced.

## 8. Freezing the manifest path

Manifests are **frozen, not removed**: the four curated plugins keep working, `manifest-v1`
and `manifest-v2` keep their contracts, and the compile path keeps its tests. What stops:
no new manifest features, no new curated manifests, no contract amendments. `docs/plugins/
manifest-v1.md` and `manifest-v2.md` each gain a status header saying so and pointing here.

This is a policy change, not a code change, and it is the reason §3 rejected modelling
picture sources as degenerate plugins — routing the new primary card type through a frozen
abstraction would make the frozen thing load-bearing again and unremovable later.

## 9. Constraint for the deferred interactivity work

Picture-card interactivity is out of scope, but the design must not foreclose it. The
constraint to respect: a tap on a picture card travels device -> server as an interaction
event (the existing path), the server produces a new picture, and the card updates through
the ordinary `ImageSourceUpdated` flow. That is a **round trip**, structurally different
from the local optimistic feedback `scene_view_apply_local_action` gives a pomodoro. The
rule of thumb should therefore be read as *"non-interactive cards must be server-side"*,
not *"server-side cards cannot be interactive"*.

## 10. Testing

**Host coverage, no hardware.**

- **Ingest bounds** — exact dimensions accepted; 447x368, 449x368, 16-bit, non-PNG and
  oversize bodies each rejected with their own status. A **decompression-bomb fixture**
  (huge declared dimensions, tiny body) rejected *without allocating*, which is what proves
  header-before-decode rather than assuming it.
- **Canonical frame** — a known PNG produces a byte-exact RGB565 blob and a **pinned
  digest**, so an endianness or header slip cannot pass silently.
- **Idempotence** — the same picture twice: one asset transfer, two recorded push times.
- **Auth** — wrong and revoked tokens rejected; the plaintext never appears in the store
  file, mirroring the existing device-token test.
- **Keep-set union** — a device with plugins loaded and sources present receives a release
  naming both, asserted as the **exact request sequence** the device sees, the way
  `server/tests/hostile_device.rs` caught the original wipe.
- **Ordering and failure** — bytes precede any digest reference; a mid-transfer failure
  pushes no scene, keeps the previous digest active, and releases nothing.
- **Staleness** — one test per bound (floor, ceiling, minimum samples), one for
  median-resists-outlier, one for recovery on push.
- **Schema v7** — a v6 file migrates losslessly with `image_sources: []`; a card naming an
  unknown source is a typed validation error.
- **Negotiation** — a picture card resolves `Native`, not `Rasterize`, not `RefuseLive`.

**Mutation probes.** Each of these must turn a test red: delete the keep-set union; delete
the header-before-decode check; delete the minimum-samples guard; digest the PNG instead of
the decoded blob.

**Deliberately no new hardware pixel rows.** A picture card's face is a full-canvas
`SceneImage`, and `scene-image` is already in the on-target matrix at both orientations
since stage 3b Task 8. New rows would be near-duplicate coverage and would disturb a
hardware-pinned number for nothing. Host tests assert instead that the scene built is the
expected single-node scene. **The 96/10/86 framebuffer split stays as recorded.**

**Workspace gates** per `CLAUDE.md`: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace --all-targets`, `cargo test
--workspace --doc`. Firmware gates are untouched because no firmware changes.

## 11. Hardware verification

There is **no firmware change**, so no OTA, no memory-layout hazard, and nothing owed on
the download check. One short session, not a battery:

1. A PNG pushed from the real producer appears on the panel at the mounted orientation.
2. Rotating away and back moves **no bytes** — the digest is already resident.
3. A second, different picture replaces the first.

Recorded in `docs/hardware/board-notes.md` with the observed result, per the working
agreement.

## 12. Rollout

Order is load-bearing, and it is the same shape as the manifest-v2 rollout:

1. **Redeploy the server binary first.** A v7 config saved against a server built before
   this change fails with the typed schema error, and every save is rejected until the
   binary moves.
2. Mint the `claude-limits` source and capture its token once.
3. Update the producer to POST a PNG.
4. Save the v7 config that replaces the `claude-limits` plugin card with a picture card.

**Prerequisite outside this repository:** `claude-limits`'s producer lives in the TRMNL
repo on docker-vm and currently emits JSON; it needs to draw a 448x368 PNG (PIL is the
obvious tool). Committing that repo is the owner's call, as it was for the epoch field.

**Producer guidance** ships as `docs/images/producer-guide.md`: exact dimensions, the
one-line curl, the error table, the note that **RLE565 expands 2x on high-entropy input**
so flat-colour UI panels cross the link at ~10 KB while photographic content costs the full
~330 KB, and the suggestion to keep the bottom strip calm so a stale footer does not land
on content.

## 13. Open items

- **Sub-project 3** is the deferred interactivity work described in §9. It has no spec yet
  and is not scheduled.
- **Sub-project 2** (weather, RSS and JSON feed becoming server-rendered pictures) needs its
  own spec once this lands; it is where the owner's rule of thumb is actually applied, and
  it will want an in-process producer writing to an image source rather than a second
  ingest mechanism.
