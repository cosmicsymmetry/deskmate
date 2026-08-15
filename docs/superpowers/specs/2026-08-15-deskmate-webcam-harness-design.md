# Deskmate Webcam Verification Harness — Design

**Date:** 2026-08-15
**Status:** Approved by user (conversation, 2026-08-15)

## Purpose

Give agent-driven hardware verification physical ground truth without a human
watching the panel. An OBSBOT Meet 2 webcam is aimed at the device on the desk;
the agent captures frames, reads them, and judges pass/fail. This complements —
never replaces — the dev-link framebuffer capture (`framebuffer_diff.rs`), which
is pixel-perfect but blind to boot states, standalone fallback while the link is
down, release firmware, and real-panel artifacts.

The harness is **Claude-judged by design**: no OCR, no pixel heuristics, no CV
code. The agent's vision is the judge. A 3840x2160 capture with autofocus
warmup was validated on 2026-08-15: every panel label is readable.

Human hands remain required for touch (taps, swipes) and cable pulls. The
harness removes the need for human *eyes*, not human *hands*.

## First jobs (success criteria)

1. The remaining V1 install-matrix rows (offline/replug, corrupt-recovery) from
   `docs/superpowers/plans/2026-08-15-deskmate-v1-packaging-hardening.md` Task 6,
   with the user pulling cables on request and the agent verifying the panel.
2. Timelapse-monitored soaks, so items like the waived 30-minute mixed soak can
   be closed with nobody watching.
3. General adoption in future milestone plans' physical checks.

Rotation/alert timing checks are explicitly out of scope for the first build.

## Components

All committed to the repo under `tools/hwcam/`:

- `capture.sh <session-dir> [label]` — captures one 3840x2160 JPEG from the
  OBSBOT to `<session-dir>/<UTC-timestamp>[-label].jpg` and prints the path.
  The camera is located **by name** ("OBSBOT") from
  `ffmpeg -f avfoundation -list_devices` at every invocation, because
  avfoundation indices shift as devices come and go. The first 75 frames
  (~2.5 s at 30 fps) are discarded so autofocus/exposure settle; validated as
  the difference between an unreadable and a fully readable panel.
- `timelapse.sh <session-dir> <interval-s> [label]` — loops `capture.sh` until
  killed. Each shot re-opens the camera (~5 s), so intervals must be ≥ 15 s.
  This is the soak monitor.
- `docs/hardware/webcam-harness.md` — usage, camera-setup expectations, session
  conventions, the agent's judging role, and limitations.

No new Rust. Scripted device states reuse the existing `companion/crates/device`
examples (`hardware_acceptance.rs`, `heartbeat_soak.rs`, `m2_stress.rs`) and
`app-core`'s `alert_replay_check.rs` unchanged.

## Sessions and evidence

- A session is a directory `~/deskmate-hw-sessions/<YYYY-MM-DD>-<topic>/`
  holding timestamped frames plus a free-form `NOTES.md` the agent keeps
  (state↔frame pairing, observations, verdicts). Sessions live outside the
  repo: frames are heavy evidence artifacts, not repo content.
- `docs/hardware/board-notes.md` remains the durable record. Entries cite the
  session directory and name the specific frames that support each verdict.
- Frame filenames are UTC timestamps (`date -u +%Y%m%dT%H%M%SZ`), optionally
  suffixed with a label. Wall-clock-on-panel vs. capture-timestamp comparisons
  are therefore possible (clock-correctness checks).

## Workflow

**Framing preflight (every session):** one capture the agent reads to confirm
the panel is sharp, fully in frame, and readable before any real checks. If
not, the agent asks the user to adjust the camera and repeats the preflight.

**Observation mode:** the companion app runs normally and owns the serial link.
The agent captures around events — e.g., the user pulls the USB cable on
request; the agent verifies standalone-clock fallback appears on the panel,
then verifies playlist resume after replug. Used for the install-matrix rows,
tray-resident soaks, and anything exercising the real app end-to-end.

**Scripted mode:** the app is quit (the port is exclusive). The agent
interleaves device-crate example runs with captures and records the pairing in
`NOTES.md`. Used for protocol-level state setup and driven soaks.

**Timelapse review:** the agent samples frames (e.g., every Nth plus any the
timelapse log flags) rather than reading every frame; the sampling rule used is
recorded in `NOTES.md`.

## Error handling

- Camera not found: list available devices, exit nonzero.
- Capture failed or produced an empty file: exit nonzero with the ffmpeg error.
- Timelapse: a failed shot is logged (with timestamp and reason) and the loop
  continues — transient camera exclusivity (e.g., a video call) must not kill a
  soak. The log lives in the session directory.
- The camera must not move between frames that will be compared; the framing
  preflight is repeated if there is any doubt.

## Testing / acceptance

- Both scripts pass `shellcheck` with no warnings.
- Live framing preflight passes on the actual desk setup.
- The remaining Task 6 install-matrix rows are executed using the harness and
  recorded in the plan and board-notes. (Re-homed: these rows execute under
  the V1 packaging plan's Task 6, per the implementation plan's Task 4 Step 5;
  the other acceptance bullets closed 2026-08-15 in the harness plan itself.)
- A ~10-minute timelapse smoke run: expected frame count (±1), all sampled
  frames readable.

## Out of scope (explicit YAGNI)

- OCR / pixel-diff / automated judging of webcam frames.
- Controllable USB power (smart hub) for automated unplug tests.
- Capture integrated into the Rust examples (a step→frame manifest); revisit
  only if manual state↔frame pairing proves error-prone in practice.
- Any firmware or companion-app changes.
