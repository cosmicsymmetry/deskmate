# Moon Phase plugin

2026-10-02, Track B. Owner request: “do moon phase,” selecting the proposed simple
offline moon, phase name and next full moon card. Follow the existing plugin PR
submission path. This is an extension of the panel's established visual language.

## Scope and direction

- An ordinary sandboxed picture producer, `moon-phase` 1.0.0; config v10, protocol
  v2, firmware, browser components and plugin runtime stay unchanged.
- Read mode: answer “what phase is the moon, and when is it full next?” at a glance.
  One 204px moon disc is the visual focus on the 448×368 black panel, followed by
  the phase name, whole-percent illumination and a UTC full moon date.
- Inherit the existing face palette and bundled Inter 400/600. The disc is exact
  projected sphere geometry driven by the computed fraction; no decorative texture,
  photo, emoji, frame, header or invented sky position.
- Northern/Southern hemisphere uses the existing enum field and autosave form.
  Northern is the default. Southern reverses the lit side only.
- No network, secrets, third-party executable dependencies, state or tap behavior.
  Request 15-minute refreshes; no device-local ticking or exact-time promise.

## Calculation contract

Use the injected UTC instant, independent of host/owner timezone. Implement Paul
Schlyter's solar/lunar elements and perturbations and find the next 180-degree
longitude crossing, distinguishing it from the new moon wrap. Support host dates
1900–2099; invalid clocks fail visibly. The README records source links, precision,
phase-name windows, UTC dates, year rollover and the schematic hemisphere view.

## Acceptance and evidence

- [x] Manifest discovery exposes the plugin and hemisphere setting, with no requests.
- [x] Sandbox tests cover independent NASA full moon times (39 across 2026, 2028,
  2099), USNO primary phases, illumination, both hemispheres, event rollover,
  timezone independence, leap day, year rollover and invalid clocks.
- [x] The real renderer produces 448×368 PNGs in both settings.
- [x] Reproducible full/desk previews cover all eight phases, Southern view and the
  longer year-rollover footer. Local inspection found no clipping or overlaps.
- [x] Full faces tests, typecheck, lint, format and catalog checks pass: after
  integrating main's PRs #14/#15, **612 tests, 0 failures, 3384 assertions**. The
  plugin's focused suite has 56 tests / 184 assertions. No lint warnings remain.
- [x] `plugin:check moon-phase` passes all ten fixtures, with no errors or limit
  notices. Its 20 full/desk PNGs are byte-identical to the reviewed samples.
- [x] Fresh independent visual/code review: **ship**, no material findings.
  Reviewer inspected all eight full-size phases, Southern/year-end variants and
  all ten desk images; independently reran the focused suite and checked the
  orbital terms against the original reference. Documentation review also passed;
  no design-system changes are needed.
- [x] Public [PR #16](https://github.com/cosmicsymmetry/deskmate/pull/16) includes
  reproducible previews and the review record. All six CI jobs passed on submitted
  head `4252e38` ([run](https://github.com/cosmicsymmetry/deskmate/actions/runs/36992891706));
  no skipped jobs. All 20 Linux artifact PNGs matched the reviewed local samples.

## Merge and deployment

The owner authorized “merge and deploy” on 2026-10-02. PR #16 merged as
`122faf7c8ed3a62961f33a8a6701c4658dfa3a45`; that clean merged revision shipped via
`deploy.sh --faces-only` at **2026-10-02 10:22:16 UTC**. Before shipping, ancestry
checks confirmed it contains every live component revision (`424c8a7`, A/B/C1/D
history); Track H's schema/wire lock is preserved and this release crosses neither.

The deployment's full faces checks passed locally and on the VM. The installed
catalog exposes Moon Phase and its Northern/Southern setting at a 900-second
requested cadence. Both settings rendered distinct 448×368 PNGs through the
installed entrypoint; those server outputs were inspected. The server stayed active
with the same process, web/binary stayed at `424c8a7`, and HTTP checks returned 200
for the app and the expected 401 for unauthenticated devices.

No physical panel observation is claimed. Existing PRODUCT.md/DESIGN.md contain
older product prose; this local card addition does not revise those unrelated
contracts or change the established visual system.
