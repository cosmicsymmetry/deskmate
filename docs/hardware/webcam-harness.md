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
