# Plugins, scenes, and a runtime asset store

Date: 2026-08-22
Status: approved design, not started. No code exists. Every number below marked
"measured" comes from reading checked-in source; every number marked "unverified"
needs a board build. Nothing here has been observed on hardware.

## Problem

Adding a widget to Deskmate costs a C template under `firmware/main/ui/templates/`,
a golden-case entry, a wire `TemplateKind`, a schema allowlist edit, and a firmware
OTA to every device in the fleet. Six templates exist. Each one was expensive, and
the two most-used — the clock and the pomodoro ring — were the most expensive of all.

That cost is acceptable for a fixed product and unacceptable for a plugin ecosystem.
The goal is that a person who wants a new face on the panel writes no C, ships no
firmware, and does not need to be trusted with either.

The reference point is TRMNL: a plugin is a data source plus a template, the server
renders it, the device displays it. That model ports. TRMNL's *refresh* model does
not — its device is a 1-bit e-ink poster refreshed every fifteen minutes, while
Deskmate is an emissive panel with a second-resolution timer and a tap target.

### What parity is and is not

`companion/crates/lvgl-sim/build.rs` compiles the firmware's **actual**
`main/ui/templates/*.c` and `main/ui/fonts/*.c`. There is one renderer compiled into
two hosts, so simulator/firmware pixel agreement is already structural, not
maintained by hand.

Parity was therefore never the pain, and this design does not claim to improve it.
The pain is the C-plus-OTA authoring cycle. Section 6 records a new parity
*obligation* this design creates.

## Reserved seams this design fills

Four slots already exist, unused, and were evidently left for this:

| Seam | Where | Current state |
| --- | --- | --- |
| `assets` partition, 6 MB | `firmware/partitions.csv` | Declared, referenced by no code |
| `CAPABILITY_ASSET_TRANSFER = 1 << 5` | `companion/crates/protocol/src/message.rs` | Defined, never set |
| `assets: []` + `AssetSettings` | `docs/config/v4.md`, `app-core/src/config.rs` | Validated, then rejected |
| `lv_tiny_ttf` | `firmware/managed_components/lvgl__lvgl/src/libs/tiny_ttf/` | Vendored, `LV_USE_TINY_TTF 0` |

`config.rs:819` rejects any non-empty `assets` array with `RequiresCapability`
("asset transfer is not implemented by this build"), and `required_capabilities()`
already maps the field to bit 5. Because compilation has always refused a non-empty
array, **no config has ever contained one**, so the v5 asset reshape below needs no
compatibility story.

## Non-goals

- Public plugin uploads. v1 ships the format and a curated set (§5).
- Headless-browser rendering. `resvg` only (§5).
- Portrait orientation, dashboards, size classes. Unchanged from v4.
- Changing where the LVGL draw buffers live (§8, open).
- Retiring the six C templates before stage 2 proves their replacements (§7).

## Architecture

```
L0  Server pipeline    fetch data -> evaluate plugin -> emit a SCENE
L1  Asset store        fonts, icon fonts, images -- 6 MB partition, mmap'd
L2  Scene renderer     generic LVGL interpreter for a declarative display list
L3  Plugin model       manifest + bindings + data source
```

The organizing rule:

> **Every plugin authors a scene. The server decides whether the device receives
> the scene or a rasterization of it.**

The render target is negotiated, not authored. A scene using a node the device
cannot draw, or targeting firmware without the interpreter, is rasterized
server-side and pushed as pixels. An SVG-authored plugin is the degenerate case: a
scene holding one full-bleed image node.

This is what keeps the two render paths from becoming two subsystems. There is one
pipeline and one fallback rule, and the pixel path is a compatibility mechanism
rather than a product tier. It also makes vocabulary extension safe: devices on old
firmware get rasterized scenes instead of broken ones.

## 1. Asset store

### Storage

A hand-laid-out partition, not a filesystem: a fixed directory region of records
(`sha256`, offset, length, kind, refcount) followed by a blob region.
`esp_partition_mmap()` maps the blob region read-only into the data address space,
and that pointer is passed directly to `lv_tiny_ttf_create_data_ex()` and to
`lv_image`.

No LittleFS. A filesystem costs flash, RAM, and a dependency, and abstracts away the
one thing needed here: a stable mmap'd pointer into flash. Font bytes are therefore
read in place and cost **no PSRAM**; only the rasterized glyph cache occupies RAM.

Content-addressing by SHA-256 buys three things: two plugins using the same face
ship it once; "do you already have this?" is answerable from the directory; and
commit is idempotent, so an interrupted transfer cannot corrupt the store.

### Two tiers

| Tier | Destination | Used for |
| --- | --- | --- |
| Durable | Flash partition | Fonts, icon fonts, plugin images. Survives reboot and OTA. |
| Volatile | PSRAM only | Rasterized frames. Never touches flash. |

One flag on the transfer, two destinations. The volatile tier exists so a
30-second-refresh raster plugin cannot consume the partition's write endurance.

### Schema v5 asset variants

v4's `font { pixel_size, glyph_ranges }` and `icon { width, height }` encode the
pre-`tiny_ttf` design, where glyphs were baked at a fixed size. They are replaced:

| v5 kind | Carries | Notes |
| --- | --- | --- |
| `font` | TTF/OTF blob | **No pixel size.** Size chosen per text node at render time. Optional server-side subsetting. |
| `icon-font` | TTF + name-to-codepoint map | Plugins write `icon: "cloud-rain"`. The server validates the name resolves. |
| `image` | LVGL binary image | Converted server-side, so the device needs no PNG decoder. |

An icon font is the whole icon answer. `weather_icon.c`'s 161 lines of procedural
vector drawing and its eleven-icon ceiling are replaced by glyphs at any size, in any
colour, through the same code path as text. LVGL's SVG/ThorVG path
(`LV_USE_VECTOR_GRAPHIC`, `LV_USE_THORVG_INTERNAL`, both vendored and off) would also
work and is rejected: it is a C++ vector engine with real flash and IRAM cost, and
**this board's IRAM is already reported 100% full**.

### Garbage collection

Append-only with compaction on demand. The host sends the referenced digest set
(`AssetRelease`); the device frees everything else and compacts past a fragmentation
threshold. Rare, explicit, interruptible.

### Memory rule

Every allocation on this path comes from PSRAM heap. `.bss` growth is measured before
and after, and the OTA download is re-verified on the physical board. This is not
caution for its own sake — see §9.

## 2. Scene format

### Absolute positioning; the server does all layout

The server holds the same TTF and the same metrics and can measure text exactly.
Laying out on the device would mean a layout engine in C kept in lockstep with the
simulator — reintroducing the cost this design exists to remove. This is the single
largest simplification in the design.

### Nodes

| Node | Fields |
| --- | --- |
| `rect` | x, y, w, h, radius, fill, opacity |
| `arc` | cx, cy, r, start_deg, end_deg, width, colour, caps |
| `line` | points, width, colour |
| `text` | x, y, w, align, font, size, colour, value, ellipsize |
| `image` | x, y, w, h, asset, recolour |
| `glyph` | icon-font, name, size, colour |

Six, deliberately. Every shipped template is expressible: digital clock is one
`text`; analog clock is `arc` plus `line` hands plus `rect` ticks; progress ring is
`arc` plus `text`; row-list is `rect`s and `text`s; big-number is `text`;
icon-badge-text is `glyph` plus `text`. That correspondence is the stage-2 acceptance
test (§7), not a coincidence.

### Bindings

A closed set, not an expression language:

```
{{time:HH:mm}}   {{time:ss}}         device clock, device timezone
{{timer.remaining:mm:ss}}            device-owned pomodoro state
{{timer.pct}}                        may drive an arc's end_deg
{{field.<name>}}                     an existing PushData field
```

`{{time:*}}` and `{{timer.*}}` are **live** bindings; `{{field.*}}` is not. That
distinction is load-bearing in §3.

Anything computed happens on the server. The device evaluates only these, so they are
the only firmware-side surface and the only thing needing firmware support to extend.
An `arc` bound to `{{timer.pct}}` keeps counting down with the link dead, which is
what preserves the repo's standalone rule.

### Atomicity

A scene replaces a card's scene as a unit, under `ApplyConfig`'s revision discipline.
A partially applied scene would be a visible tear.

## 3. Render negotiation

Resolved per card per revision from capability bits, asset inventory, node types, and
binding set:

| Condition | Decision |
| --- | --- |
| Scenes supported, all nodes known, assets present or installable | Native scene |
| Scene uses a live binding, device cannot render natively | **Refuse**, typed error |
| Otherwise | Rasterize to a volatile image asset plus a one-node scene |

The refuse case must be typed and visible in that card's editor. A card that would
freeze says so; it does not silently display a stopped clock. The V1 playlists plan
fixed the validation-mislabeling defect precisely so a failure is never presented as
something else, and that rule governs here.

**Any live binding requires native rendering.** Rasterizing a clock is wrong at every
resolution, not only below a minute: a `{{time:HH:mm}}` face rasterized on a 30-second
cadence displays the wrong minute for up to half of every minute, and pushing on exact
minute boundaries instead would mean 1,440 pushes per device per day that a single
network hiccup still renders wrong. So a card whose scene reads the clock or the timer
is refused on a device that cannot draw it natively, rather than approximated.

Rasterization cadence for everything else belongs to the server and follows the
provider's schedule, floored at 30 seconds.

The decision is a property of the (scene, device) pair, not of the plugin. The same
plugin on newer firmware upgrades to native with no authoring change.

## 4. Wire

Additive to protocol v1. Bit 5 (`ASSET_TRANSFER`) switches on; bit 8 (`SCENE_RENDER`)
is added.

> Every new bit must be OR'd into `CURRENT_CAPABILITIES` in `message.rs` **and** the
> firmware constant, with a test pinning the numeric value. Bit 7 sat defined-but-dark
> for most of V2 — the constant read 75 instead of 203, and a conforming host could
> not have provisioned the device. This is a checklist item.
>
> The values are arithmetic, so pin them in a test: today's `203` becomes **`235`** at
> stage 1 (bit 5, `+32`) and **`491`** at stage 2 (bit 8, `+256`).

| ID | Name | Payload |
| --- | --- | --- |
| 15 | `AssetBegin` | digest, kind, total_len, durable/volatile |
| 16 | `AssetChunk` | digest, offset, bytes |
| 17 | `AssetCommit` | digest |
| 18 | `AssetRelease` | referenced digest set |
| 19 | `PushScene` | card id, revision, scene CBOR |

Inventory needs no message of its own: the device answers `AssetBegin` with
`Ack{already_present}` and the host skips the chunks. Content-addressing makes that
free — no query, no list to keep in sync, no staleness.

### Concurrency

One outstanding request per connection with a 2000 ms timeout means a 500 KB font is
roughly 250 sequential round trips — about 7.5 s at 30 ms RTT, during which taps and
heartbeats queue.

Windowed transfer (N outstanding chunks, ack by highest contiguous offset) reaches
about 1 s and is **rejected for v1**: it breaks an invariant the whole firmware relies
on, inside a protocol that has been deliberately bounded.

Adopted instead: keep one-outstanding, make transfer **resumable and preemptible**.
The host may abandon a chunk sequence to service interactive traffic and resume at the
last committed offset; content-addressing makes resume idempotent. Assets install on
plugin install, not per refresh, so 7.5 s once is acceptable.

### Compression

LVGL vendors `rle` and `lz4` (`LV_USE_RLE`, `LV_USE_LZ4_INTERNAL`, both off). Flat
colour on true black should compress very well under RLE at a fraction of LZ4's cost.
Start with RLE, measure, keep LZ4 as a switch. TTFs are not compressed.

Transfer sits behind an `AssetTransport` seam so §8's spike can replace it wholesale.

## 5. Server pipeline and security

```
manifest -> Provider -> evaluate -> SCENE -> decision -> PushScene
                                                       | resvg -> RGB565 -> RLE
                                                         -> volatile asset + PushScene
```

`companion/crates/providers` already defines `Provider`, `ProviderSnapshot`,
`LastGood`, `RefreshPolicy`, and categorised `ProviderError`. Plugin data sources
implement that trait, so stale/last-good behaviour and the companion's existing
per-card `stale` flag apply to plugins with no new machinery.

Manifest is declarative; it contains no code:

```toml
name = "aqi"
version = "1.0.0"
source   = { kind = "json", url = "…", refresh_minutes = 15 }
assets   = [ { kind = "font", file = "Inter.ttf" } ]
template = "scene.hbs"
```

### Posture

The render host is the owner's homelab (docker-vm), behind Cloudflare, alongside
unrelated services. That is the threat model.

- **No arbitrary code.** Templates are a restricted expression language over fetched
  data, not a scripting runtime.
- **`resvg` only for rasterization.** No headless browser: it would make the homelab
  an arbitrary-code-execution host. Deferred indefinitely, not scheduled.
- **Egress allowlist.** Deny RFC1918, loopback, link-local, and `169.254.169.254`;
  resolve-then-pin against DNS rebinding; cap size, time, and redirects.
- **Per-plugin caps** on CPU, memory, render wall-clock, and refresh rate.
- **v1 ships the format and a curated set, not public uploads.** Proving the format
  with first-party plugins costs nothing v1 wants and removes the largest risk. Public
  uploads are stage 5, with their own review.

## 6. Parity and testing

### The new parity obligation

Today the simulator compiles the firmware's own font C files, so both hosts rasterize
identically by construction. Once fonts become runtime assets that stops being
automatic: the simulator must resolve the **same digest to the same bytes** as the
device. This design therefore *adds* a parity obligation rather than removing one, and
it must be discharged explicitly by an asset-store shim in `lvgl-sim` that reads from
the same content-addressed store the server pushes from.

`stb_truetype` is deterministic, so identical bytes at an identical size produce
identical glyphs. The obligation is about byte provenance, not about the rasterizer.

The scene interpreter itself compiles into `lvgl-sim` unchanged via
`build.rs`'s existing source list — one implementation, two hosts, as today.

### Coverage

- `cases.rs` gains scene cases: each node type at both orientations, plus composites.
  `framebuffer_diff.rs` retargets to them. Existing template cases remain as
  regression until the templates retire.
- Asset-store logic is hardware-independent and belongs in `firmware/main/core/`, free
  of ESP-IDF includes, host-tested in `firmware/host_tests`: directory parsing, GC,
  resume, corruption rejection.
- Hostile-input tests on the scene decoder: malformed CBOR, unknown node types,
  out-of-range coordinates, unresolvable asset references, recursion depth, oversize
  strings. Bytes from the host are untrusted, as everywhere else in this protocol.

### Mandatory hardware gates

- `idf.py size` before and after, with the `.bss` delta recorded in board-notes.
- **An OTA download verified on the physical board.** Non-negotiable; see §9.
- Glyph-cache-miss timing at 96 px on the LVGL task, to quantify the first-render hitch.
- `board_lcd_rounder_cb` correct at both orientations with image nodes present.

## 7. Rollout

| Stage | Content | Gate |
| --- | --- | --- |
| 1 | Asset store, `tiny_ttf`, bit 5 | OTA download verified on board; `.bss` delta recorded |
| 2 | Scene renderer, wire, bit 8 | The six templates re-expressed as scenes and framebuffer-diffed against their C versions |
| 3 | Plugin manifest, curated plugins, clock and pomodoro as scenes | Physical verification at both orientations |
| 4 | Rasterization fallback, SVG plugins | Refuse-rule and 30 s floor observed |
| 5 | Public uploads and sandbox | Separate risk review |

Stage 2's gate is the strongest claim available: **the scene renderer is correct if
and only if it reproduces every shipped template pixel for pixel.** Fifty-eight diff
cases and a working harness already exist to prove it, and the generic interpreter has
to earn its place against the hand-written C it replaces before anything depends on it.

**This spec is the architecture for all five stages; it is not one implementation
plan.** Stages 1 and 2 form the first plan — they are the foundation, they share the
hardware gates, and stage 2's pixel gate is what licenses everything after it. Stages 3
to 5 get their own plans, written at the previous stage's exit using what it taught.

Schema v4 becomes v5 (asset variants reshape; cards gain a plugin kind). Protocol
stays v1 and additive. The six C templates stay until stage 3 proves their equivalents.

V2's exit gate is still open — Task 8's widening-backoff observation and Task 9's tap
latency. Stage 1 is safe to start now; V2 should close before stage 3.

## 8. Open questions

**The draw-buffer spike.** Moving LVGL's draw buffers to PSRAM (`buff_spiram`, present
in the vendored `esp_lvgl_port_disp.h`) would return 94,208 bytes of internal DMA RAM
— the pool that starved mbedTLS and from which the hardware AES accelerator's DMA
buffers must come. If that margin makes two concurrent TLS sessions viable, asset
transfer becomes a plain HTTPS GET like OTA's and §4's chunked design is deleted.

This is a hypothesis, not a finding: the AES path's actual requirement has not been
measured, and 94 KB may not be the deciding margin. It is a cheap experiment with a
large payoff, and it is the reason §4 sits behind a seam. It is **not** in scope as a
change to ship — see §9 for why it is dangerous to fold into a feature.

**Task 9 tap latency.** Unobserved. It decides whether rasterized cards can be
interactive at all, since each of their taps is a round trip through Cloudflare. Owed
by V2's exit gate independently of this work.

## 9. Risks

**Memory layout is the dominant risk, and it is historical, not theoretical.** This
board has been broken twice by layout shifts, both times with every test green:

- The V1 boot crash-loop — a draw-buffer allocation failure caused by a layout shift,
  surfacing as an `IllegalInstruction` panic 1.3 s into every boot.
- `3f2aa03` — approximately **105 bytes of static internal DRAM** broke OTA downloads
  entirely, bisected on the board across three builds from an identical base.

`display.c` additionally records moving from two draw buffers to one while isolating
panel corruption. Every conclusion follows from this: allocate from PSRAM heap, keep
`.bss` flat, measure it, and re-verify the OTA download on hardware. Do not fold the
§8 spike into a feature change.

**IRAM is reported 100% full.** `stb_truetype` should land in flash `.text`, but that
is a claim to verify with `idf.py size`, not to assume.

**Glyph rasterization on the LVGL task.** A cache miss at 96 px rasterizes inline and
may show as a hitch. Mitigation is cache warming at card activation. This is exactly
the class of defect that appears only on hardware.

**Font licensing** becomes a real question at stage 5 and does not exist before it.

**Asset store exhaustion.** 6 MB is generous, not infinite. Refcounting, GC, and a
factory-reset wipe path are required, not optional.
