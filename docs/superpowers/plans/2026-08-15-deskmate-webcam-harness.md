# Webcam Verification Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two committed capture scripts plus a usage doc that let the agent verify the physical Deskmate panel by reading OBSBOT webcam frames, with no human eyes required.

**Architecture:** Thin shell over `ffmpeg -f avfoundation`. `capture.sh` grabs one sharp 4K frame into a session directory; `timelapse.sh` loops it for soak monitoring. The agent judges frames with vision — there is deliberately no OCR/CV/pixel code anywhere. Scripted device states reuse the existing `companion/crates/device` examples unchanged.

**Tech Stack:** bash, ffmpeg (present at `/opt/homebrew/bin/ffmpeg`), shellcheck, macOS avfoundation.

**Spec:** `docs/superpowers/specs/2026-08-15-deskmate-webcam-harness-design.md`

## Global Constraints

- macOS only; camera is an "OBSBOT Meet 2 StreamCamera" resolved **by name substring `OBSBOT`** at every invocation (avfoundation indices shift).
- Capture format is fixed: 3840x2160 at 30 fps, discard the first 75 frames (~2.5 s autofocus/exposure warmup — validated 2026-08-15), save frame 76 as JPEG.
- Session directories live **outside the repo**: `~/deskmate-hw-sessions/<YYYY-MM-DD>-<topic>/`. Never commit frames.
- Frame filenames are UTC timestamps `date -u +%Y%m%dT%H%M%SZ`, optional `-<label>` suffix.
- No new Rust, no firmware or companion-app changes, no OCR/pixel-judging code.
- Timelapse interval ≥ 15 s (each shot holds the camera ~5 s).
- Scripts print the produced frame path on stdout; all diagnostics go to stderr or the session log.
- Conventional commit prefixes. No tags.
- If `shellcheck` is missing, install it first: `brew install shellcheck`.

---

### Task 1: `tools/hwcam/capture.sh`

**Files:**
- Create: `tools/hwcam/capture.sh`

**Interfaces:**
- Produces: `capture.sh <session-dir> [label]` → writes `<session-dir>/<UTC-stamp>[-label].jpg`, prints that path on stdout, exit 0. Exit 1 on camera-not-found / failed / empty capture; exit 2 on usage error. Task 2 invokes it by path; Task 4 and all future verification sessions call it directly.

- [ ] **Step 1: Write the script**

Create `tools/hwcam/capture.sh` with exactly this content:

```bash
#!/bin/bash
# Capture one 4K frame of the Deskmate board from the OBSBOT webcam.
#
# Usage: capture.sh <session-dir> [label]
# Prints the produced frame path on stdout. Diagnostics go to stderr.
#
# The first 75 frames (~2.5s at 30fps) are discarded so autofocus and
# exposure settle; validated 2026-08-15 as the difference between an
# unreadable and a fully readable panel.
set -euo pipefail

usage() {
    echo "usage: $0 <session-dir> [label]" >&2
    exit 2
}
[ $# -ge 1 ] && [ $# -le 2 ] || usage
session_dir=$1
label=${2:-}

# avfoundation device indices shift as cameras come and go; resolve by name,
# stopping before the audio-device section (the OBSBOT microphone also matches).
device_index=$(ffmpeg -hide_banner -f avfoundation -list_devices true -i "" 2>&1 |
    awk '/audio devices:/ { exit }
         /OBSBOT/ { if (match($0, /\[[0-9]+\]/)) { print substr($0, RSTART + 1, RLENGTH - 2); exit } }')

if [ -z "$device_index" ]; then
    echo "error: no OBSBOT camera among avfoundation video devices:" >&2
    ffmpeg -hide_banner -f avfoundation -list_devices true -i "" 2>&1 |
        sed -n '/video devices/,/audio devices/p' >&2
    exit 1
fi

mkdir -p "$session_dir"
stamp=$(date -u +%Y%m%dT%H%M%SZ)
frame="$session_dir/$stamp${label:+-$label}.jpg"

if ! ffmpeg -hide_banner -loglevel error -f avfoundation -framerate 30 \
    -video_size 3840x2160 -i "$device_index" \
    -vf "select='gte(n,75)'" -frames:v 1 -y "$frame" >&2; then
    echo "error: capture failed" >&2
    exit 1
fi
if [ ! -s "$frame" ]; then
    echo "error: empty capture: $frame" >&2
    exit 1
fi
echo "$frame"
```

- [ ] **Step 2: Make it executable and lint it**

```sh
chmod +x tools/hwcam/capture.sh
shellcheck tools/hwcam/capture.sh
```

Expected: no output from shellcheck (clean).

- [ ] **Step 3: Live failure-path check (camera busy or absent is NOT simulated — argument errors are)**

```sh
tools/hwcam/capture.sh; echo "exit=$?"
```

Expected: usage line on stderr, `exit=2`.

- [ ] **Step 4: Live capture check**

```sh
tools/hwcam/capture.sh ~/deskmate-hw-sessions/$(date -u +%Y-%m-%d)-harness-build smoke
```

Expected: prints one path like `~/deskmate-hw-sessions/<date>-harness-build/<stamp>-smoke.jpg`; file is > 100 KB. The agent then **reads the frame** and confirms the Deskmate panel is visible and its text readable. (ffmpeg may print a benign "Selected pixel format (yuv420p) is not supported" advisory to stderr; that is not a failure.)

- [ ] **Step 5: Commit**

```sh
git add tools/hwcam/capture.sh
git commit -m "feat: webcam capture script for hardware verification"
```

---

### Task 2: `tools/hwcam/timelapse.sh`

**Files:**
- Create: `tools/hwcam/timelapse.sh`

**Interfaces:**
- Consumes: `capture.sh` from Task 1 (invoked from the same directory as this script).
- Produces: `timelapse.sh <session-dir> <interval-seconds> [label]` → loops capture until killed; appends start/stop/failure lines and per-shot capture output to `<session-dir>/timelapse.log`. Interval is the sleep between shots, so the true period is interval + ~5 s capture latency. Exit 2 on usage error.

- [ ] **Step 1: Write the script**

Create `tools/hwcam/timelapse.sh` with exactly this content:

```bash
#!/bin/bash
# Periodic Deskmate panel captures for soak monitoring. Runs until killed.
#
# Usage: timelapse.sh <session-dir> <interval-seconds> [label]
# interval-seconds is the sleep BETWEEN shots; each shot itself holds the
# camera for ~5s, so the true period is roughly interval + 5s.
#
# A failed shot is logged and the loop continues: transient camera
# exclusivity (e.g. a video call) must not kill a soak.
set -euo pipefail

usage() {
    echo "usage: $0 <session-dir> <interval-seconds> [label]" >&2
    exit 2
}
[ $# -ge 2 ] && [ $# -le 3 ] || usage
session_dir=$1
interval=$2
label=${3:-}
script_dir=$(cd "$(dirname "$0")" && pwd)

case $interval in
    '' | *[!0-9]*) usage ;;
esac
if [ "$interval" -lt 15 ]; then
    echo "error: interval must be >= 15s (each shot holds the camera ~5s)" >&2
    exit 2
fi

mkdir -p "$session_dir"
log="$session_dir/timelapse.log"
echo "$(date -u +%Y%m%dT%H%M%SZ) start interval=${interval}s label=${label:-none}" >>"$log"
trap 'echo "$(date -u +%Y%m%dT%H%M%SZ) stop" >>"$log"' EXIT

while :; do
    if ! "$script_dir/capture.sh" "$session_dir" ${label:+"$label"} >>"$log" 2>&1; then
        echo "$(date -u +%Y%m%dT%H%M%SZ) capture FAILED (continuing)" >>"$log"
    fi
    sleep "$interval"
done
```

- [ ] **Step 2: Make it executable and lint it**

```sh
chmod +x tools/hwcam/timelapse.sh
shellcheck tools/hwcam/timelapse.sh
```

Expected: no shellcheck output. (The `${label:+"$label"}` expansion is the standard optional-argument idiom; shellcheck accepts it.)

- [ ] **Step 3: Usage and interval validation checks**

```sh
tools/hwcam/timelapse.sh; echo "exit=$?"
tools/hwcam/timelapse.sh /tmp/x 5; echo "exit=$?"
tools/hwcam/timelapse.sh /tmp/x abc; echo "exit=$?"
```

Expected: `exit=2` for all three, with the usage line (first, third) and the interval error (second) on stderr. Nothing is written to `/tmp/x`.

- [ ] **Step 4: Three-shot smoke run**

```sh
SESSION=~/deskmate-hw-sessions/$(date -u +%Y-%m-%d)-harness-build
tools/hwcam/timelapse.sh "$SESSION" 15 lapse &
LAPSE_PID=$!
sleep 65 && kill "$LAPSE_PID"
ls "$SESSION"/*-lapse.jpg | wc -l
tail -5 "$SESSION"/timelapse.log
```

Expected: 3 or 4 `-lapse.jpg` frames (65 s at a ~20 s true period), a `start` line and a `stop` line in the log, no `FAILED` lines. The agent reads one frame to confirm readability.

- [ ] **Step 5: Commit**

```sh
git add tools/hwcam/timelapse.sh
git commit -m "feat: webcam timelapse script for soak monitoring"
```

---

### Task 3: `docs/hardware/webcam-harness.md`

**Files:**
- Create: `docs/hardware/webcam-harness.md`

**Interfaces:**
- Consumes: script behaviors exactly as specified in Tasks 1–2.
- Produces: the usage/convention reference future sessions and plans cite.

- [ ] **Step 1: Write the doc**

Create `docs/hardware/webcam-harness.md` with exactly this content:

```markdown
# Webcam Verification Harness

An OBSBOT Meet 2 webcam aimed at the board gives agent-driven verification
physical ground truth: the agent captures frames, reads them, and judges
pass/fail. No OCR or CV code exists by design — the agent's vision is the
judge. Design: `docs/superpowers/specs/2026-08-15-deskmate-webcam-harness-design.md`.

This complements the dev-link framebuffer capture (`framebuffer_diff.rs`),
which is pixel-perfect but blind to boot states, standalone fallback while
the link is down, release firmware, and real-panel artifacts.

## Scripts

- `tools/hwcam/capture.sh <session-dir> [label]` — one 3840x2160 JPEG,
  ~2.5 s autofocus warmup baked in, camera resolved by the name `OBSBOT` at
  every call. Prints the frame path on stdout; exit 1 if the camera is
  missing or the capture fails, exit 2 on usage errors.
- `tools/hwcam/timelapse.sh <session-dir> <interval-s> [label]` — loops
  capture until killed; the soak monitor. Interval is the sleep between
  shots (≥ 15 s); true period ≈ interval + 5 s. Start/stop/failure lines go
  to `<session-dir>/timelapse.log`; a failed shot is logged and the loop
  continues.

## Sessions

`~/deskmate-hw-sessions/<YYYY-MM-DD>-<topic>/` — timestamped frames plus a
`NOTES.md` the agent keeps (state↔frame pairing, observations, verdicts,
and for timelapses the sampling rule used when reviewing). Sessions are
evidence, not repo content; `docs/hardware/board-notes.md` remains the
durable record and cites the session directory and the specific frames
behind each verdict.

## Workflow

- **Framing preflight (every session):** one capture the agent reads to
  confirm the panel is sharp, centered, and readable before any real
  checks. Repeat after any doubt about camera movement; frames compared
  against each other must come from an unmoved camera.
- **Observation mode:** the companion app runs normally and owns the serial
  link; the agent captures around events (e.g. the user pulls USB on
  request, the agent verifies standalone-clock fallback on the panel).
- **Scripted mode:** the app is quit (the port is exclusive); the agent
  interleaves `companion/crates/device` example runs with captures and
  records the pairing in `NOTES.md`.

## Limitations

- Touch (taps, swipes) and cable pulls remain human actions; the harness
  removes the need for human eyes, not human hands.
- One camera consumer at a time: a video call can steal the OBSBOT; capture
  fails loudly, timelapse logs and continues.
- Captures cost ~5 s of camera hold each; do not schedule below 15 s
  intervals.
```

- [ ] **Step 2: Verify the doc's claims against the scripts**

Re-read both scripts and confirm every behavioral claim in the doc (exit codes, warmup, log lines, interval floor) matches the implementations. Fix whichever side is wrong — the scripts are the source of truth unless they contradict the spec.

- [ ] **Step 3: Commit**

```sh
git add docs/hardware/webcam-harness.md
git commit -m "docs: webcam harness usage and session conventions"
```

---

### Task 4: Live acceptance — preflight + 10-minute observation-mode timelapse

**Files:**
- Modify: `docs/hardware/board-notes.md` (append one entry)

**Interfaces:**
- Consumes: both scripts and the doc from Tasks 1–3; the running companion app (observation mode).
- Produces: a board-notes entry future plans can cite as the harness's commissioning record. After this task, the paused V1 packaging plan's Task 6 rows (offline/replug, corrupt recovery) resume **using this harness** under that plan, not this one.

- [ ] **Step 1: Framing preflight**

```sh
SESSION=~/deskmate-hw-sessions/$(date -u +%Y-%m-%d)-harness-acceptance
tools/hwcam/capture.sh "$SESSION" preflight
```

The agent reads the frame: panel sharp, fully in frame, text readable. If not, ask the user to adjust the camera and repeat.

- [ ] **Step 2: Ten-minute timelapse with the app running**

```sh
tools/hwcam/timelapse.sh "$SESSION" 25 soak &
LAPSE_PID=$!
sleep 600 && kill "$LAPSE_PID"
ls "$SESSION"/*-soak.jpg | wc -l
grep -c FAILED "$SESSION"/timelapse.log || true
```

Expected: ~20 frames (600 s at a ~30 s true period; accept 18–21), zero `FAILED` lines. The agent reviews every 5th frame plus the final one, confirms each is readable and shows a plausible playlist card, and writes the sampling rule and verdict to `"$SESSION"/NOTES.md`.

- [ ] **Step 3: Record commissioning in board-notes**

Append to `docs/hardware/board-notes.md` an entry titled "Webcam harness commissioned — <date>" recording: preflight result, timelapse frame count, FAILED count, the sampling rule used, the session directory path, and the explicit statement that judging is agent-vision (no CV code). Use only observed values — never projected ones.

- [ ] **Step 4: Commit and push**

```sh
git add docs/hardware/board-notes.md
git commit -m "docs: webcam harness commissioning record"
git push origin main
```

- [ ] **Step 5: Hand back to the packaging plan**

Announce that the V1 packaging plan (`docs/superpowers/plans/2026-08-15-deskmate-v1-packaging-hardening.md`) Task 6 resumes at its Step 4/5 rows, now using this harness for panel observations. That work is tracked there, not here.
