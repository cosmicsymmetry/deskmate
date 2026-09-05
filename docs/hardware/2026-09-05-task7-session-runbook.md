# Task 7 hardware session — runbook (prepared 2026-09-05, board away)

Everything below was staged in advance so the session spends its time on observations,
not builds. The plan of record is
`docs/superpowers/plans/2026-08-29-deskmate-rasterization.md` Task 7 (Phase A pays
stage 3b Task 9 **Step 6 only** — Steps 1-5 and 7 passed 2026-08-30 — then Phase B is
the one stage-4 image). Results go to `docs/hardware/board-notes.md`, Phase A and B
kept separate. Never record an observation that was not made.

## What is already staged (verified 2026-09-05)

- **Server**: docker-vm runs the post-cleanup stage-4 server built from `f40958a`
  (deployed 2026-09-05 19:20 UTC). Registry loads **5 plugins, 0 failures**:
  `agenda, aqi, claude-limits, svg-aqi, svg-live-clock`. `dev-0005` intact,
  disconnected. The published pin is **unchanged at `v2.0.0-live2`**, so a connecting
  board is offered nothing.
- **Deploy fact learned during prep**: the stage-4 server does
  `include_bytes!("../../../../tools/fonts/…")`, so a `companion/`-only source export
  no longer builds. `~/deskmate-build/` on the VM now carries `tools/fonts/` beside
  `companion/`; keep rsyncing both.
- **Stage-4 image**: `v2.0.0-raster1.bin` is already in `/var/lib/deskmate/firmware/`
  but **not published** (the catalog serves only the env-pinned version).
  sha256 `29f15a6f7ee1f6f5717deaa09a21a3bdb75bd19c6ddeae5dd2b51ade1afc3b15`.
  Built from `f40958a` with `firmware/version.txt` = `v2.0.0-raster1`.
  Memory at that build (the pre-publish baseline for Step 4): DIRAM total **203,867**
  (`.text` 93,635, `.bss` **87,104**, `.data` 23,128), IRAM 16,384/16,384 (0 remaining),
  flash `.text` 1,136,096, `.rodata` 329,848.
- **Diag image**: `firmware/build-diag/deskmate.bin` (same tree, `DESKMATE_DEV_DIAG=1`),
  sha256 `f815edf07cef7981698304d964f468ed7cde6296d7a1783c4f32843f16fb1dbc`. Only for
  the closing framebuffer_diff step; never published, flashed over the cable only.
- **Refuse fixture**: `tools/hw-session-fixtures/svg-live-clock/` (a v2 SVG manifest
  whose face binds `{{ time:HH:mm }}`) is loaded on the server for Step 8. It is
  deliberately not under `companion/plugins/` — the curated registry is pinned to four
  ids.
- **Camera**: capture pipeline verified end-to-end, but the physical OBSBOT was asleep
  during prep (placeholder frame). If the OBSBOT app holds the camera, use
  `HWCAM_DEVICE='OBSBOT Virtual' HWCAM_VIDEO_SIZE=1920x1080 HWCAM_FRAMERATE=60`.

## Session-start checklist

1. Board on the desk, USB down (90° default), camera framing the panel; test one
   `tools/hwcam/capture.sh <dir> start` and *look at it*.
2. `git status` clean at (or after) the commit carrying this runbook; if the tree moved
   since, re-verify the staged image hashes still correspond to HEAD before publishing.
3. Server sanity (from the VM, or via `https://deskmate.rodi.one` with the same token):

   ```sh
   ssh rodion@100.93.166.123
   TOKEN=$(sudo -n grep ADMIN_TOKEN /etc/deskmate/server.env | cut -d= -f2)
   curl -s -H "Authorization: Bearer $TOKEN" http://192.168.8.20:8443/v1/plugins   # 5 ids
   curl -s -H "Authorization: Bearer $TOKEN" http://192.168.8.20:8443/v1/devices/dev-0005
   sudo -n journalctl -u deskmate-server -n 3 --no-pager   # plugin_count=5 failure_count=0
   ```

4. Let the board connect and settle; confirm `connected: true` and note
   `firmware_version` (expect `v2.0.0-live2`) and capabilities (expect **491** — the
   predecessor image does not know bit 9).

## Operator-route notes that bite

- `data` for the operator plugin route is the **inner** payload (root already applied):
  `[source].root` is applied only in the fetch path. For `svg-aqi` and `aqi` alike,
  send the shape the bindings address: `{"current": {"aqi": 42, "category": "good"}, …}`.
- **The revision footgun is still live on the raw `digital_clock` diagnostic form**:
  push revisions just above the runtime's counter, never arbitrary large ones. The
  negotiated plugin form mints its own revision and is safe.
- An **until-dismissed alert holds `interrupt_live` and defers OTA forever** — clear
  alerts before Phase B.

## Phase A — on `v2.0.0-live2` (do not flash anything yet)

**A1 (plan Step 1, mostly pre-paid).** Re-read the journal's registry line and the two
curls above; that re-verifies the redeploy at session time.

**A2 (plan Step 2, shrunk).** Steps 1-5/7 of stage 3b Task 9 passed 2026-08-30 and are
not redone (board-notes "Stage 3b Task 9 — PASSED 2026-08-30"). Nothing to do unless
something in A3 contradicts them.

**A3 (plan Step 3 — the asset-GC teardown, stage 3b's one open item).** Drive a durable
release whose keep-set shrinks: move `/var/lib/deskmate/plugins/aqi` aside, restart the
server, let the board reconnect (synchronize_full reconciles assets before layout).
Observe, on the panel and in status:
  1. the standalone-clock handoff while the teardown runs;
  2. scene destruction and font release (the aqi glyph face must not survive);
  3. compaction: the absent digest goes DEAD, retained digests are kept (compare
     `GET /v1/plugins` digests against asset-used counters/status);
  4. rebuild from the retained scene — the panel comes back without a reboot; a release
     that changes nothing must leave the panel unchanged.
Restore the `aqi` plugin dir and restart when done.

**The BUSY/OTA-owner variant needs an OTA in flight, which Phase A cannot have without
publishing something.** Default: observe it once during Phase B Step 10's mandated
"repeat one release while OTA owns the panel", and record it as completing stage 3b
Step 6's last clause — an explicit, recorded deviation from strict phase ordering. If
strict ordering is wanted instead, build a predecessor-era image under a fresh version
string (never reuse a failed string) and run the variant before flashing stage 4.

**If any A3 observation fails: stop. Amend stage 3b. Do not publish `v2.0.0-raster1`.**

## Phase B — publish the one stage-4 image

**B4 (publish + baseline).** Baseline numbers are recorded above (same tree, same
day). Publish:

```sh
sudo -n sed -i 's/^DESKMATE_FIRMWARE_VERSION=.*/DESKMATE_FIRMWARE_VERSION=v2.0.0-raster1/' /etc/deskmate/server.env
sudo -n systemctl restart deskmate-server
# verify the public artifact before the board sees it (download is unauthenticated):
curl -s https://deskmate.rodi.one/v1/firmware/v2.0.0-raster1.bin | shasum -a 256
```

Confirm the served hash equals `29f15a6f…`. One power cycle is an **owner action**
(battery: USB unplug is link loss, not power loss).

**B5 (OTA download, the real gate).** Watch `firmware_version`, `ota_state`,
`last_ota_error`, link continuity, and rollback-window survival
(`esp_ota_mark_app_valid…` — the image must still be running a minute later). One
transient retry of the identical image is permitted; a second identical failure is
deterministic — stop and bisect on the same base. Green tests are not evidence here.

**B6 (capability truth).** Device and server both report bit 9 by name, numeric
**1003**, no unknown bit. One volatile begin accepted.

**B7 (native vs raster, both orientations).** `aqi`/`agenda`/`claude-limits` stay
native; `svg-aqi` produces a volatile 448x368 frame and a one-node scene. Look at the
panel at 90° and 270° — no capture can prove the physical transform.

**B8 (the refuse rule).** Activate `svg-live-clock` (operator route, any data, e.g.
`{}`). Confirm: no raster asset sent, no frozen scene, `SceneRefused` with the
live-binding reason (`time:HH:mm`) in the card's editor/API, panel keeps prior content.

**B9 (30-second floor).** Three visibly distinct `svg-aqi` snapshots at t=0/5/10 s
(e.g. `aqi` 42 → 87 → 155). Expect: one immediate frame, nothing before t+30, then the
**newest** snapshot. Quote `AssetBegin`/`PushScene` timestamps from the journal.

**B10 (churn).** ≥20 raster revisions at the floor (~10 min; script it, don't babysit).
Flash asset-used bytes flat, PSRAM not trending down, old volatile digests freed, kept
digest resolvable, no clock flap on ordinary swaps, no tearing. Then the one release
while OTA owns the panel (see Phase A note) — defer/rebuild observed.

**B11 (framebuffer_diff — LAST, it costs the device identity).** Over the cable:
flash `firmware/build-diag/deskmate.bin`, take the board to local tier, run
`companion/crates/device/examples/framebuffer_diff` fresh. Expected split
**96 total / 8 excluded / 88 identical** is a software prediction — record the actual
numbers, and the raster row's byte comparison with its honest claim (payload delivery,
not source-render correctness). Then restore: full `idf.py flash` of the release build
(rollback tests need a full flash), return to networked tier, **mint a fresh identity**
(plaintext token exists only at mint), and re-home `dev-0005`'s config — the
claude-limits card lives in it — onto the new device id.

**B12.** Board-notes entry: versions, hashes, memory numbers, timestamps, every
observation and non-observation.

## Also in this sitting, if time allows (older debts)

- V2 Task 8: widening-backoff observation on a shipping build (kill the tunnel, watch
  reconnect intervals widen).
- V2 Task 9: tap latency.
- `claude-limits` on the panel (already in dev-0005's active playlist; an eyeball plus
  a webcam frame closes it).
