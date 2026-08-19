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
  Geometry and device are overridable via `HWCAM_DEVICE`, `HWCAM_VIDEO_SIZE`,
  `HWCAM_FRAMERATE` and `HWCAM_WARMUP_FRAMES`, because which camera is
  reachable depends on how the OBSBOT software is running: **when its app is
  open it holds the physical `OBSBOT Meet 2 StreamCamera` and publishes a
  processed feed as `OBSBOT Virtual Camera`, which offers 1920x1080@60
  only.** Asking the held physical device for 4K fails with an I/O error, not
  a resolution error, so the failure does not name its cause. For that setup:
  `HWCAM_DEVICE='OBSBOT Virtual' HWCAM_VIDEO_SIZE=1920x1080 HWCAM_FRAMERATE=60`.
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
- **The virtual camera can return a convincing wrong frame.** "Fails loudly"
  holds for the physical device but *not* for `OBSBOT Virtual Camera`: with
  video switched off in the OBSBOT app it keeps producing frames, emitting the
  app's logo placeholder. `capture.sh` succeeds and writes a valid JPEG of
  something that is not the board. An agent judging frames must confirm the
  panel is actually present before reading anything off it; a capture that
  succeeds is not evidence that the board was photographed. Observed
  2026-08-19.
- Captures cost ~5 s of camera hold each; do not schedule below 15 s
  intervals.
