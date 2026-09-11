# Legacy Subtraction — Wave C: the device stops modelling templates (protocol v2)

**Spec:** `docs/superpowers/specs/2026-09-11-deskmate-legacy-subtraction-design.md`

**Goal:** Remove the pre-scene wire. The device has drawn only host-pushed scenes since
stage 3a, yet it still holds a widget model of templates, size classes and screens, plus
a per-template field registry whose only surviving job is carrying three magic pomodoro
keys.

## Why this one lands differently

Waves A and B were host-only, so they could merge and deploy the moment the gates passed.
Wave C changes the **wire**, which means the server and the firmware must move together:

- A v2 server cannot talk to a v1 device. The live server at `deskmate.rodi.one` is the
  one `dev-0005` dials into.
- Firmware statics move when `template_fields.c`'s registries and most of
  `widget_model.c` go, and this repo has twice lost days to memory-layout shifts with
  every test green.

**Therefore: this wave is implemented and gated on a branch, and is NOT merged to `main`
and NOT deployed.** Merging arms the next routine redeploy to break the fleet. The merge,
the flash and the redeploy belong to one hardware session, in this order:

1. Flash the v2 firmware over USB and confirm it boots and draws.
2. Merge and redeploy the server.
3. Confirm the device reconnects and the panel keeps drawing.
4. **Run the OTA download check** — mandatory, because `.bss` moved.
5. Record the observed result in `docs/hardware/board-notes.md`.

Rollback is the previous server binary plus a reflash; both halves must go back together.

## The v2 wire

`ApplyConfig` becomes the loop and nothing else:

```
ApplyConfig { revision, rotation, cards: [ { card_id, tap_action } ] }
```

Gone: `template_kind`, `size_class`, `interrupt_policy`, and the whole `screens` array —
since schema v10 a screen id is always the card id, so the array restated the card list.

`PushData` is replaced by an explicit timer:

```
PushTimer { card_id, revision, total_ms, remaining_ms, running }
```

The device currently derives this by reading `duration_seconds`, `remaining_seconds` and
`running` out of a ProgressRing field bag by name (`protocol_task.c`'s
`fill_timer_bindings`). Nothing else consumes pushed fields: **no production builder has
emitted a `field.*` binding since v9 removed manifest plugins**, so `SCENE_BINDING_FIELD`
and the entire field registry go with `PushData`.

`ActivateScreen` becomes `ActivateCard { card_id }` — a rename that stops pretending
there are two identifier spaces.

Error codes `UnsupportedTemplate`, `UnsupportedSizeClass` and `UnknownScreen` go.

## Capabilities

Protocol v2 re-bases them. The bits that described template support — core widgets,
config rotation, extended templates — say nothing about a device that only draws scenes.
What survives is what a host still has to ask about: scene rendering, firmware update,
networking, volatile assets, durable asset encoding.

## Tasks

### Task 1 — the protocol crate

- [ ] `PROTOCOL_VERSION` 1 → 2; re-base `PROTOCOL_CURRENT_CAPABILITIES`.
- [ ] `ApplyConfig`/`WidgetConfig` lose template, size class, interrupt policy, screens.
- [ ] `PushData` → `PushTimer`; delete `Field`, `FieldValue`, `TemplateKind`,
      `SizeClass`, `InterruptPolicy`, `ScreenConfig` and the three error codes.
- [ ] `ActivateScreen` → `ActivateCard`.
- [ ] Regenerate the fixtures; `docs/protocol/v2.md` frozen from v1.

### Task 2 — firmware

- [ ] Delete `core/template_fields.{c,h}` and its host test.
- [ ] `core/widget_model.c` keeps revisions and the card list; the field state,
      template registry and per-template validation go.
- [ ] `core/scene_binding.c` loses `SCENE_BINDING_FIELD` and the `scene_field_fn` seam.
- [ ] `link/protocol_task.c`: decode `PushTimer` into the binding context directly;
      `fill_timer_bindings` reads it instead of a field bag; `show_carousel_fallback`
      loses the binding it computes and discards.
- [ ] `ui/carousel.c` navigates the card list rather than a screens array.
- [ ] Record `idf.py size` before and after: **the `.bss` delta is the number that
      justifies the on-board OTA check.**

### Task 3 — the host

- [ ] `app-core`: `wire_config` stops emitting a template; `push_fields` becomes
      `push_timer`; the scene builders keep their inputs (they are host-side).
- [ ] `lvgl-sim`: the simulator's binding context loses its field table.
- [ ] `framebuffer_diff`: the four `field.*` exclusions go — they exist only because no
      registry accepts a field name, and there is no registry.

### Task 4 — documentation

- [ ] `docs/protocol/v2.md`; mark v1 superseded.
- [ ] CLAUDE.md: protocol v2, new capabilities, and the rollout order above.
- [ ] `docs/hardware/board-notes.md`: a stub naming what the session must observe.

## Exit

- [ ] Full host gate set green.
- [ ] `idf.py -C firmware build` succeeds; size delta recorded.
- [ ] `make -C firmware/host_tests clean test` and `sanitize` green.
- [ ] **NOT merged, NOT deployed.** The branch is the deliverable.
