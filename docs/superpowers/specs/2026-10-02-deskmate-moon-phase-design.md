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
- [ ] Full faces tests, typecheck, lint, format and catalog checks pass.
- [ ] Independent visual/code review and public PR with reproducible evidence.

No physical panel observation or production deployment is claimed. Existing
PRODUCT.md/ DESIGN.md contain older product prose; this local card addition does
not revise those unrelated contracts or change the established visual system.
