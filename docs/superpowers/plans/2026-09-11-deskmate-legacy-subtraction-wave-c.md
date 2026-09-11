# Legacy Subtraction — Wave C: the device stops modelling templates (protocol v2)

> ## STATUS — complete and MERGED to `main` 2026-09-12; the server is NOT deployed
>
> All four tasks are done and every software gate is green: the full companion workspace
> set (`fmt --check`, `clippy -D warnings`, `test --all-targets`, `test --doc`),
> `make -C firmware/host_tests clean test` and `sanitize`, `idf.py -C firmware build`,
> and the frontend typecheck.
>
> **The plan below says this wave must not be merged until the hardware session. The
> owner directed otherwise on 2026-09-12 and it was merged.** That moves the safety
> property rather than removing it: it used to be "the branch is unmerged", and it is now
> **"do not redeploy the server until the board is flashed"**. `main` holds a v2 server
> and `dev-0005` still runs v1 firmware; a redeploy before the flash answers every frame
> with `VersionMismatch` and darkens the panel. The rollout order in "Why this one lands
> differently" is unchanged and still binding -- only step 2's merge has already happened.
> What the session owes is stubbed at the end of `docs/hardware/board-notes.md`.
>
> **Three deviations from the plan as written, all widenings:**
>
> 1. **`DeviceEvent` lost its second identifier too.** The plan renamed `ActivateScreen`
>    to `ActivateCard` "to stop pretending there are two identifier spaces", but
>    `DeviceEvent` carried both `widget_id` (key 2) and `screen_id` (key 3), always set
>    to the same card by every producer. Key 2 is now `card_id` and key 3 is retired.
>    `TriggerInterrupt.widget_id`, `carousel_binding_t`'s three parallel id pairs,
>    `interrupt_state`'s `saved_screen_id`, and the host's `active_screen`/`dirty_widgets`
>    /`UnknownWidget`/`UnknownScreen` all followed. Leaving them would have kept the two
>    spaces alive under a renamed message.
> 2. **The three id-length constants collapsed to one.** `MAX_WIDGET_ID_LEN`,
>    `MAX_SCREEN_ID_LEN` and `MAX_CARD_ID_LEN` were all 32 for the same thing; likewise
>    `PROTOCOL_MAX_CONFIG_WIDGETS`/`_SCREENS`. One space, one constant.
> 3. **`field.` was removed from the scene binding grammar, not just from the device.**
>    The plan retired `SCENE_BINDING_FIELD` in firmware. `protocol::binding_is_valid`
>    still accepted the prefix, so a host could have minted a scene the device now refuses
>    — a mismatch no host test could see, because no host test asks the C parser. Both
>    sides refuse it now, and the two `lvgl-sim` cases that bound `field.status` were
>    rewritten to bind `date` and `timer.status` instead, preserving both pixel behaviours
>    (a value that resolves, and one that renders the `--` placeholder).
>
> **The measured `.bss` delta is −104 bytes** (87,104 → 87,000; `.data` and IRAM
> byte-flat, DIRAM total 203,891 → 203,763), which is within one byte of the shift that
> broke OTA downloads in `3f2aa03`. The on-board OTA check is mandatory, not a formality.
>
> One prediction the hardware session must verify rather than trust: the
> `framebuffer_diff` matrix is now **44 rows** (Wave A's oracle retirement took 34, v9's
> plugin removal 18), and with the four `field.*` exclusions closed the only one left is
> the `progress-ring--running-mid-countdown` timing race — so the expected split is
> `44 total / 2 excluded / 42 identical / 0 differing`, against the last observed
> `96 / 10 / 86`.

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

- [x] `PROTOCOL_VERSION` 1 → 2; re-base `PROTOCOL_CURRENT_CAPABILITIES`.
- [x] `ApplyConfig`/`WidgetConfig` lose template, size class, interrupt policy, screens.
- [x] `PushData` → `PushTimer`; delete `Field`, `FieldValue`, `TemplateKind`,
      `SizeClass`, `InterruptPolicy`, `ScreenConfig` and the three error codes.
- [x] `ActivateScreen` → `ActivateCard`.
- [x] Regenerate the fixtures; `docs/protocol/v2.md` frozen from v1.

### Task 2 — firmware

- [x] Delete `core/template_fields.{c,h}` and its host test.
- [x] `core/widget_model.c` keeps revisions and the card list; the field state,
      template registry and per-template validation go.
- [x] `core/scene_binding.c` loses `SCENE_BINDING_FIELD` and the `scene_field_fn` seam.
- [x] `link/protocol_task.c`: decode `PushTimer` into the binding context directly;
      `fill_timer_bindings` reads it instead of a field bag; `show_carousel_fallback`
      loses the binding it computes and discards.
- [x] `ui/carousel.c` navigates the card list rather than a screens array.
- [x] Record `idf.py size` before and after: **the `.bss` delta is the number that
      justifies the on-board OTA check.**

### Task 3 — the host

- [x] `app-core`: `wire_config` stops emitting a template; `push_fields` becomes
      `push_timer`; the scene builders keep their inputs (they are host-side).
- [x] `lvgl-sim`: the simulator's binding context loses its field table.
- [x] `framebuffer_diff`: the four `field.*` exclusions go — they exist only because no
      registry accepts a field name, and there is no registry.

### Task 4 — documentation

- [x] `docs/protocol/v2.md`; mark v1 superseded.
- [x] CLAUDE.md: protocol v2, new capabilities, and the rollout order above.
- [x] `docs/hardware/board-notes.md`: a stub naming what the session must observe.

## Exit

- [x] Full host gate set green.
- [x] `idf.py -C firmware build` succeeds; size delta recorded.
- [x] `make -C firmware/host_tests clean test` and `sanitize` green.
- [x] ~~**NOT merged, NOT deployed.** The branch is the deliverable.~~ Merged to `main`
      on 2026-09-12 by owner direction; **not deployed**, and the server must not be
      redeployed until the board is flashed. See the STATUS header.
