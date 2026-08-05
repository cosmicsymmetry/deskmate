# Deskmate Card Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the v2 `widgets[]`/`screens[]`/size-class/dashboard authoring model with one ordered `cards[]` array, host-driven timed rotation, and a bounded two-trigger alert mechanic — without changing the wire protocol or firmware.

**Architecture:** `AppConfig` gains `cards: Vec<CardSettings>` and loses `widgets`, `screens`, and all size classes. Each card carries `presence` (in-rotation / alert-only / off) and `alert` (none / on-timer-finish / before-event). `AppConfig::compile` lowers `cards[]` into the *unchanged* frozen `ApplyConfig { widgets, screens, rotation, revision }` wire shape, pinning `SizeClass::Full` and deriving each screen ID from its card ID. Timed rotation and alert triggers are evaluated entirely in the Rust runtime, which already owns time, provider data, and pomodoro state; the firmware receives only `ActivateScreen` and `TriggerInterrupt` as it does today. The companion webview collapses to a single card list plus a preview that renders from real last-good field values delivered over widened typed IPC.

**Tech Stack:** Rust 2024 workspace (`app-core`, `protocol`, `providers`, `engine`, `device`), Tauri v2, React 19 + TypeScript, Bun test, Biome, ESP-IDF 5.x / LVGL 9 firmware (unchanged by this plan).

**Spec:** `docs/superpowers/specs/2026-08-06-deskmate-card-model-design.md`

## Global Constraints

- The wire protocol does not change. `ApplyConfig` stays `{0: revision, 1: widgets, 2: screens, 3: rotation}`. No firmware source file is modified by this plan.
- One Rust runtime remains the only owner of serial I/O, revisions, provider work, timers, interrupts, actions, and replay. The webview gains no filesystem, HTTP, shell, process, serial, or updater primitives.
- Transaction order is fixed and unchanged: merge command-owned preferences, validate and compile, check connected-device capabilities, atomically persist, replace runtime state, then queue full replay.
- A configuration document is at most 65,536 UTF-8 bytes. Unknown fields are rejected at every level (`#[serde(deny_unknown_fields)]` on every struct, `deny_unknown_fields` on every tagged enum).
- Card count is `1..8`. Card IDs are `1..32` UTF-8 bytes, unique among cards. Cards and assets remain separate ID namespaces.
- Bounds, verbatim from the spec: `dwell_seconds` is `null` or `5..3600`; `default_dwell_seconds` is `5..3600`; `lead_minutes` is `1..60`; `hold.seconds` is `5..600`.
- Verification commands, run from `companion/`:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cd apps/deskmate && bun test && bun run check && bun run lint && bun run format:check && bun run build`
- Firmware host tests must keep passing **unmodified**: `make -C firmware/host_tests clean test`. That they pass without edits is the evidence the wire did not move.
- Conventional commit prefixes (`feat:`, `fix:`, `test:`, `docs:`, `chore:`). Do not rewrite shared history.
- Never claim hardware verification that was not observed on the physical board.

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `companion/crates/app-core/src/config.rs` | Card types, bounds, validation, compilation to wire | Heavy modify |
| `companion/crates/app-core/src/store.rs` | Persistence, version detection, v0/v1/v2 → v3 migration | Heavy modify |
| `companion/crates/app-core/src/state.rs` | IPC snapshot DTOs; gains card field values | Modify |
| `companion/crates/app-core/src/scheduler.rs` | Deadline bookkeeping; gains the rotation deadline | Modify |
| `companion/crates/app-core/src/runtime.rs` | Single-owner worker; rotation advance and alert triggers | Modify |
| `companion/crates/app-core/tests/{config,store,runtime}.rs` | Rust test suites | Modify |
| `companion/crates/app-core/tests/fixtures/*.json` | Config fixtures | Modify + rename |
| `companion/apps/deskmate/src-tauri/src/commands.rs` | IPC commands + TS contract generator test | Modify |
| `companion/apps/deskmate/src/lib/types.ts` | Hand-written TS DTOs | Heavy modify |
| `companion/apps/deskmate/src/lib/types.contract.ts` | Generated fixture — regenerate, never hand-edit | Regenerate |
| `companion/apps/deskmate/src/lib/configDraft.ts` | Draft helpers over `cards[]` | Heavy modify |
| `companion/apps/deskmate/src/components/CardList.tsx` | Single ordered card list (two sections) | **Create** |
| `companion/apps/deskmate/src/components/CardEditor.tsx` | Per-card editor for all six kinds | **Create** (replaces `WidgetEditor.tsx`) |
| `companion/apps/deskmate/src/components/Filmstrip.tsx` | Rotation strip + loop length + play | **Create** |
| `companion/apps/deskmate/src/components/DevicePreview.tsx` | Card face rendering from real field values | Heavy modify |
| `companion/apps/deskmate/src/components/WidgetGallery.tsx` | — | **Delete** (folded into `CardList`) |
| `companion/apps/deskmate/src/components/ScreenArranger.tsx` | — | **Delete** (folded into `CardList`) |
| `companion/apps/deskmate/src/App.tsx` | Shell wiring | Modify |
| `docs/config/v2.md` → `docs/config/v3.md` | Frozen config contract | Rename + rewrite |

**Ordering rationale:** Tasks 1–4 change the host model with no device-visible behaviour change. Tasks 5–6 add new device behaviour. Task 7 widens IPC. Tasks 8–11 rebuild the frontend. Task 12 closes the docs. Tasks 4 and 7 both regenerate `types.contract.ts`; that is expected and each regeneration is verified by the same sync test.

---

### Task 1: Card, presence, and alert types

Introduce the new value types **alongside** the existing ones so the workspace stays green. `AppConfig` is not touched yet.

**Files:**
- Modify: `companion/crates/app-core/src/config.rs`
- Modify: `companion/crates/app-core/src/lib.rs` (re-exports)
- Test: `companion/crates/app-core/tests/config.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `CardPresence`, `AlertHold`, `CardAlert`, `CarouselAdvance`, and the constants `MAX_CONFIG_CARDS: usize`, `MIN_DWELL_SECONDS: u16`, `MAX_DWELL_SECONDS: u16`, `MIN_ALERT_LEAD_MINUTES: u16`, `MAX_ALERT_LEAD_MINUTES: u16`, `MIN_ALERT_HOLD_SECONDS: u16`, `MAX_ALERT_HOLD_SECONDS: u16`. Methods: `CardPresence::dwell_seconds(self, default: u16) -> Option<u16>`, `CardPresence::is_in_rotation(self) -> bool`, `CardPresence::is_off(self) -> bool`, `CardAlert::is_none(self) -> bool`, `CarouselAdvance::default_dwell_seconds(self) -> Option<u16>`.

- [ ] **Step 1: Write the failing test**

Append to `companion/crates/app-core/tests/config.rs`:

```rust
#[test]
fn card_presence_resolves_dwell_against_the_carousel_default() {
    assert_eq!(
        CardPresence::InRotation { dwell_seconds: Some(45) }.dwell_seconds(20),
        Some(45)
    );
    assert_eq!(
        CardPresence::InRotation { dwell_seconds: None }.dwell_seconds(20),
        Some(20)
    );
    assert_eq!(CardPresence::AlertOnly.dwell_seconds(20), None);
    assert_eq!(CardPresence::Off.dwell_seconds(20), None);

    assert!(CardPresence::InRotation { dwell_seconds: None }.is_in_rotation());
    assert!(!CardPresence::AlertOnly.is_in_rotation());
    assert!(CardPresence::Off.is_off());
    assert!(!CardPresence::AlertOnly.is_off());
}

#[test]
fn card_behaviour_types_round_trip_as_closed_tagged_json() {
    let presence = CardPresence::InRotation { dwell_seconds: Some(30) };
    let json = serde_json::to_value(presence).unwrap();
    assert_eq!(json["kind"], "in-rotation");
    assert_eq!(json["dwell_seconds"], 30);
    assert_eq!(serde_json::from_value::<CardPresence>(json).unwrap(), presence);

    let alert = CardAlert::BeforeEvent {
        lead_minutes: 5,
        hold: AlertHold::Seconds { value: 60 },
    };
    let json = serde_json::to_value(alert).unwrap();
    assert_eq!(json["kind"], "before-event");
    assert_eq!(json["hold"]["kind"], "seconds");
    assert_eq!(serde_json::from_value::<CardAlert>(json).unwrap(), alert);

    assert!(CardAlert::None.is_none());
    assert!(!alert.is_none());

    let advance = CarouselAdvance::Timed { default_dwell_seconds: 20 };
    assert_eq!(advance.default_dwell_seconds(), Some(20));
    assert_eq!(CarouselAdvance::Manual.default_dwell_seconds(), None);

    // Unknown fields are rejected at every level.
    assert!(serde_json::from_str::<CardPresence>(
        r#"{"kind":"alert-only","dwell_seconds":10}"#
    )
    .is_err());
}
```

Add to the `use app_core::{...}` list at the top of that file: `AlertHold, CardAlert, CardPresence, CarouselAdvance`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p app-core --test config card_ 2>&1 | tail -20`
Expected: FAIL — `cannot find type CardPresence in crate app_core` / unresolved imports.

- [ ] **Step 3: Write minimal implementation**

In `companion/crates/app-core/src/config.rs`, add the constants next to the existing ones near line 40:

```rust
pub const MAX_CONFIG_CARDS: usize = 8;
pub const MIN_DWELL_SECONDS: u16 = 5;
pub const MAX_DWELL_SECONDS: u16 = 3_600;
pub const MIN_ALERT_LEAD_MINUTES: u16 = 1;
pub const MAX_ALERT_LEAD_MINUTES: u16 = 60;
pub const MIN_ALERT_HOLD_SECONDS: u16 = 5;
pub const MAX_ALERT_HOLD_SECONDS: u16 = 600;
```

Then add the types (place them beside `WidgetInterruptPolicy`, around line 548):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CardPresence {
    InRotation { dwell_seconds: Option<u16> },
    AlertOnly,
    Off,
}

impl CardPresence {
    pub const fn dwell_seconds(self, default: u16) -> Option<u16> {
        match self {
            Self::InRotation { dwell_seconds } => Some(match dwell_seconds {
                Some(seconds) => seconds,
                None => default,
            }),
            Self::AlertOnly | Self::Off => None,
        }
    }

    pub const fn is_in_rotation(self) -> bool {
        matches!(self, Self::InRotation { .. })
    }

    pub const fn is_off(self) -> bool {
        matches!(self, Self::Off)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AlertHold {
    UntilDismissed,
    Seconds { value: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CardAlert {
    None,
    OnTimerFinish { hold: AlertHold },
    BeforeEvent { lead_minutes: u16, hold: AlertHold },
}

impl CardAlert {
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    pub const fn hold(self) -> Option<AlertHold> {
        match self {
            Self::None => None,
            Self::OnTimerFinish { hold } | Self::BeforeEvent { hold, .. } => Some(hold),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CarouselAdvance {
    Manual,
    Timed { default_dwell_seconds: u16 },
}

impl CarouselAdvance {
    pub const fn default_dwell_seconds(self) -> Option<u16> {
        match self {
            Self::Manual => None,
            Self::Timed { default_dwell_seconds } => Some(default_dwell_seconds),
        }
    }
}
```

In `companion/crates/app-core/src/lib.rs`, add the new names to the `pub use config::{...}` list.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd companion && cargo test -p app-core --test config card_`
Expected: PASS (2 tests).

- [ ] **Step 5: Verify the workspace is still green**

Run: `cd companion && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass. Nothing consumes the new types yet.

- [ ] **Step 6: Commit**

```bash
git add companion/crates/app-core/src/config.rs companion/crates/app-core/src/lib.rs companion/crates/app-core/tests/config.rs
git commit -m "feat: add card presence, alert, and carousel advance types"
```

---

### Task 2: Swap AppConfig to cards[] and lower to the frozen wire

The breaking change, landed atomically so every commit compiles. `WidgetSettings` becomes `CardSettings`: `size` and `interrupt_policy` are removed, `presence` and `alert` are added. `ScreenSettings`, `ScreenLayout`, `TileSettings`, and `WidgetSize` are deleted.

**Files:**
- Modify: `companion/crates/app-core/src/config.rs`
- Modify: `companion/crates/app-core/src/lib.rs`
- Modify: `companion/crates/app-core/src/runtime.rs` (call sites)
- Modify: `companion/crates/app-core/src/store.rs` (call sites; migration lands in Task 3)
- Modify: `companion/apps/deskmate/src-tauri/src/commands.rs` (call sites)
- Test: `companion/crates/app-core/tests/config.rs`

**Interfaces:**
- Consumes: `CardPresence`, `CardAlert`, `AlertHold`, `CarouselAdvance` from Task 1.
- Produces:
  - `AppConfig { schema_version: u32, preferences: AppPreferences, cards: Vec<CardSettings>, assets: Vec<AssetSettings>, carousel: CarouselSettings, updater: UpdaterSettings }`
  - `CardSettings` — tagged enum with the six kinds; every variant has `id: String`, `template: DisplayTemplate`, `tap_action: WidgetTapAction`, `refresh: RefreshPolicy`, `presence: CardPresence`, `alert: CardAlert`, plus its provider fields (unchanged from `WidgetSettings`, minus `size`, minus `interrupt_policy`).
  - Accessors: `CardSettings::id(&self) -> &str`, `::template(&self) -> &DisplayTemplate`, `::tap_action(&self) -> &WidgetTapAction`, `::refresh(&self) -> RefreshPolicy`, `::presence(&self) -> CardPresence`, `::alert(&self) -> CardAlert`.
  - `CarouselSettings { advance: CarouselAdvance }`, defaulting to `CarouselAdvance::Manual`.
  - `CURRENT_SCHEMA_VERSION == 3`.
- Removed (later tasks must not reference these): `WidgetSettings`, `WidgetSize`, `WidgetInterruptPolicy`, `ScreenSettings`, `ScreenLayout`, `TileSettings`, `MAX_CONFIG_WIDGETS`, `MAX_CONFIG_SCREENS`, `MAX_SCREEN_ID_LEN`, `MAX_AUTO_ADVANCE_SECONDS`, `CarouselSettings::auto_advance_seconds`, `protocol::CAPABILITY_DASHBOARD_LAYOUTS` usage in `required_device_capabilities` (the constant itself stays in `protocol`).

- [ ] **Step 1: Write the failing test**

Append to `companion/crates/app-core/tests/config.rs`:

```rust
fn clock_card(id: &str, presence: CardPresence) -> CardSettings {
    CardSettings::Clock {
        id: id.into(),
        title: "Desk".into(),
        show_seconds: true,
        template: DisplayTemplate::DigitalClock,
        tap_action: WidgetTapAction::None,
        refresh: RefreshPolicy::DeviceLocal,
        presence,
        alert: CardAlert::None,
    }
}

fn pomodoro_card(id: &str, presence: CardPresence, alert: CardAlert) -> CardSettings {
    CardSettings::Pomodoro {
        id: id.into(),
        label: "Focus".into(),
        duration_seconds: 1_500,
        template: DisplayTemplate::ProgressRing,
        tap_action: WidgetTapAction::StartPause,
        refresh: RefreshPolicy::DeviceLocal,
        presence,
        alert,
    }
}

#[test]
fn compilation_lowers_cards_to_the_frozen_wire_shape() {
    let config = AppConfig {
        schema_version: 3,
        cards: vec![
            clock_card("clock", CardPresence::InRotation { dwell_seconds: Some(10) }),
            pomodoro_card(
                "focus",
                CardPresence::AlertOnly,
                CardAlert::OnTimerFinish { hold: AlertHold::UntilDismissed },
            ),
            clock_card("muted", CardPresence::Off),
        ],
        ..AppConfig::default()
    };

    let compiled = config.compile(7).unwrap();

    // Off cards vanish entirely; alert-only cards are screenless widgets.
    let widget_ids: Vec<&str> = compiled
        .layout
        .widgets
        .iter()
        .map(|widget| widget.widget_id.as_str())
        .collect();
    assert_eq!(widget_ids, ["clock", "focus"]);

    // Screen ID is the card ID, in card order, for in-rotation cards only.
    let screens: Vec<(&str, &str)> = compiled
        .layout
        .screens
        .iter()
        .map(|screen| (screen.screen_id.as_str(), screen.widget_id.as_str()))
        .collect();
    assert_eq!(screens, [("clock", "clock")]);

    // Size class is pinned to Full for every emitted widget, forever.
    assert!(
        compiled
            .layout
            .widgets
            .iter()
            .all(|widget| widget.size_class == protocol::SizeClass::Full)
    );

    // Off cards receive no initial push.
    let pushed: Vec<&str> = compiled
        .initial_pushes
        .iter()
        .map(|push| push.widget_id.as_str())
        .collect();
    assert_eq!(pushed, ["clock", "focus"]);
}

#[test]
fn compilation_is_deterministic_for_identical_input() {
    let config = AppConfig::default();
    assert_eq!(config.compile(4).unwrap(), config.compile(4).unwrap());
}

#[test]
fn timed_advance_no_longer_requires_an_unimplemented_capability() {
    let config = AppConfig {
        carousel: CarouselSettings {
            advance: CarouselAdvance::Timed { default_dwell_seconds: 20 },
        },
        ..AppConfig::default()
    };
    assert!(config.compile(1).is_ok());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p app-core --test config compilation_lowers 2>&1 | tail -20`
Expected: FAIL — `AppConfig` has no field `cards`; `CardSettings` not found.

- [ ] **Step 3: Rewrite the config types**

In `companion/crates/app-core/src/config.rs`:

1. Set `pub const CURRENT_SCHEMA_VERSION: u32 = 3;`
2. Delete `WidgetSize`, `WidgetInterruptPolicy`, `ScreenSettings`, `ScreenLayout`, `TileSettings`, and their `impl` blocks, plus the constants `MAX_CONFIG_WIDGETS`, `MAX_CONFIG_SCREENS`, `MAX_SCREEN_ID_LEN`, `MAX_AUTO_ADVANCE_SECONDS`.
3. Rename `WidgetSettings` to `CardSettings`. In every variant, delete `size: WidgetSize` and replace `interrupt_policy: WidgetInterruptPolicy` with:

```rust
        presence: CardPresence,
        alert: CardAlert,
```

4. In the `impl CardSettings` block, delete `size()`, and add the two accessors, following the existing `id()` match-arm style:

```rust
    pub const fn presence(&self) -> CardPresence {
        match self {
            Self::Clock { presence, .. }
            | Self::Pomodoro { presence, .. }
            | Self::Calendar { presence, .. }
            | Self::Weather { presence, .. }
            | Self::JsonFeed { presence, .. }
            | Self::Rss { presence, .. } => *presence,
        }
    }

    pub const fn alert(&self) -> CardAlert {
        match self {
            Self::Clock { alert, .. }
            | Self::Pomodoro { alert, .. }
            | Self::Calendar { alert, .. }
            | Self::Weather { alert, .. }
            | Self::JsonFeed { alert, .. }
            | Self::Rss { alert, .. } => *alert,
        }
    }
```

5. Replace `CarouselSettings`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarouselSettings {
    pub advance: CarouselAdvance,
}

impl Default for CarouselSettings {
    fn default() -> Self {
        Self { advance: CarouselAdvance::Manual }
    }
}
```

6. Replace the `widgets` and `screens` fields of `AppConfig` with `pub cards: Vec<CardSettings>`, and update `AppConfig::default()`:

```rust
            cards: vec![CardSettings::Clock {
                id: "clock".into(),
                title: "Desk".into(),
                show_seconds: true,
                template: DisplayTemplate::DigitalClock,
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::DeviceLocal,
                presence: CardPresence::InRotation { dwell_seconds: None },
                alert: CardAlert::None,
            }],
```

7. Replace the widget/screen halves of `compile()` (config.rs:344-391) with:

```rust
        let mut widgets = Vec::with_capacity(self.cards.len());
        let mut screens = Vec::with_capacity(self.cards.len());
        for (index, card) in self.cards.iter().enumerate() {
            if card.presence().is_off() {
                continue;
            }
            let Some(widget) = card.wire_config() else {
                compatibility_issues.push(ValidationIssue::new(
                    format!("cards[{index}]"),
                    ValidationCode::RequiresCapability,
                    "card provider/template/action is not implemented by this build",
                ));
                continue;
            };
            if card.presence().is_in_rotation() {
                // The screen ID is the card ID. The protocol treats widget and screen
                // IDs as distinct namespaces, so reuse is legal, and compilation still
                // invents no identifiers.
                screens.push(ScreenConfig {
                    screen_id: card.id().to_owned(),
                    widget_id: card.id().to_owned(),
                });
            }
            widgets.push(widget);
        }
```

Delete the `carousel.auto_advance_seconds` capability block entirely (config.rs:385-391) — this plan implements timed advance.

8. In `wire_config()`, pin the size class to `protocol::SizeClass::Full` and derive `interrupt_policy` from the alert:

```rust
            interrupt_policy: if self.alert().is_none() {
                protocol::InterruptPolicy::Disabled
            } else {
                protocol::InterruptPolicy::Enabled
            },
            size_class: protocol::SizeClass::Full,
```

9. Change `initial_pushes` to skip `Off` cards:

```rust
        let initial_pushes = self
            .cards
            .iter()
            .filter(|card| !card.presence().is_off())
            .map(|card| PushData {
                widget_id: card.id().into(),
                revision,
                fields: card.initial_fields(),
            })
            .collect();
```

10. In `required_device_capabilities()`, delete the `CAPABILITY_DASHBOARD_LAYOUTS` branch and change every `self.widgets` to `self.cards`.

11. Update `lib.rs` re-exports: remove the deleted names, add `CardSettings`.

- [ ] **Step 4: Update the validation body**

In `AppConfig::validate()`, replace the widgets/screens sections with card validation. Keep every existing per-variant check by leaving `card.validate(&path, &mut issues)` intact.

```rust
        validate_collection_bounds("cards", self.cards.len(), MAX_CONFIG_CARDS, &mut issues);

        let mut card_ids = HashSet::with_capacity(self.cards.len());
        let mut in_rotation = 0_usize;
        for (index, card) in self.cards.iter().enumerate() {
            let path = format!("cards[{index}]");
            validate_identifier(&format!("{path}.id"), card.id(), MAX_WIDGET_ID_LEN, &mut issues);
            if !card_ids.insert(card.id()) {
                issues.push(ValidationIssue::new(
                    format!("{path}.id"),
                    ValidationCode::DuplicateId,
                    format!("card ID {:?} is duplicated", card.id()),
                ));
            }
            card.validate(&path, &mut issues);
            validate_card_behaviour(&path, card, &mut issues);
            if card.presence().is_in_rotation() {
                in_rotation += 1;
            }
        }
        if in_rotation == 0 && !self.cards.is_empty() {
            issues.push(ValidationIssue::new(
                "cards",
                ValidationCode::OutOfRange,
                "at least one card must be in the rotation",
            ));
        }
        if let CarouselAdvance::Timed { default_dwell_seconds } = self.carousel.advance {
            validate_range(
                "carousel.advance.default_dwell_seconds",
                u32::from(default_dwell_seconds),
                u32::from(MIN_DWELL_SECONDS),
                u32::from(MAX_DWELL_SECONDS),
                &mut issues,
            );
        }
```

Add the free function beside the other validators:

```rust
fn validate_card_behaviour(path: &str, card: &CardSettings, issues: &mut Vec<ValidationIssue>) {
    let presence = card.presence();
    let alert = card.alert();

    if let CardPresence::InRotation { dwell_seconds: Some(seconds) } = presence {
        validate_range(
            &format!("{path}.presence.dwell_seconds"),
            u32::from(seconds),
            u32::from(MIN_DWELL_SECONDS),
            u32::from(MAX_DWELL_SECONDS),
            issues,
        );
    }

    // An alert-only card with no trigger could never appear on screen.
    if matches!(presence, CardPresence::AlertOnly) && alert.is_none() {
        issues.push(ValidationIssue::new(
            format!("{path}.presence"),
            ValidationCode::OutOfRange,
            "an alert-only card must configure an alert",
        ));
    }

    match alert {
        CardAlert::None => {}
        CardAlert::OnTimerFinish { hold } => {
            if !matches!(card, CardSettings::Pomodoro { .. }) {
                issues.push(ValidationIssue::new(
                    format!("{path}.alert"),
                    ValidationCode::OutOfRange,
                    "on-timer-finish alerts are only valid on pomodoro cards",
                ));
            }
            validate_alert_hold(path, hold, issues);
        }
        CardAlert::BeforeEvent { lead_minutes, hold } => {
            if !matches!(card, CardSettings::Calendar { .. }) {
                issues.push(ValidationIssue::new(
                    format!("{path}.alert"),
                    ValidationCode::OutOfRange,
                    "before-event alerts are only valid on calendar cards",
                ));
            }
            validate_range(
                &format!("{path}.alert.lead_minutes"),
                u32::from(lead_minutes),
                u32::from(MIN_ALERT_LEAD_MINUTES),
                u32::from(MAX_ALERT_LEAD_MINUTES),
                issues,
            );
            validate_alert_hold(path, hold, issues);
        }
    }
}

fn validate_alert_hold(path: &str, hold: AlertHold, issues: &mut Vec<ValidationIssue>) {
    if let AlertHold::Seconds { value } = hold {
        validate_range(
            &format!("{path}.alert.hold.value"),
            u32::from(value),
            u32::from(MIN_ALERT_HOLD_SECONDS),
            u32::from(MAX_ALERT_HOLD_SECONDS),
            issues,
        );
    }
}
```

If `validate_range` does not already exist with that signature, reuse whatever numeric-bounds helper `config.rs` already uses for `duration_seconds` and match its argument order.

- [ ] **Step 5: Fix every call site until the workspace compiles**

Run: `cd companion && cargo check --workspace 2>&1 | grep -E "^error" | head -40`

Work through the errors. The mechanical substitutions are:
- `config.widgets` → `config.cards`
- `WidgetSettings` → `CardSettings`
- `widget.interrupt_policy == WidgetInterruptPolicy::Enabled` → `!card.alert().is_none()`
- Anything reading `config.screens` to find the first/next screen now derives it from `config.cards.iter().filter(|c| c.presence().is_in_rotation())`.

In `runtime.rs`, the pomodoro-completion interrupt gate (runtime.rs:962, :1498, :1535) currently keys on the interrupt policy; change it to `!card.alert().is_none()` for now. Task 6 narrows it to `CardAlert::OnTimerFinish` specifically.

- [ ] **Step 6: Run the new tests**

Run: `cd companion && cargo test -p app-core --test config`
Expected: PASS, including the three tests from Step 1. Pre-existing tests that construct `WidgetSettings`/`ScreenSettings` will fail to compile — update them to the card constructors as you go.

- [ ] **Step 7: Full verification**

Run:
```bash
cd companion && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
make -C ../firmware/host_tests clean test
```
Expected: Rust passes. **Firmware host tests pass with no firmware file modified** — this is the evidence the wire did not move.

- [ ] **Step 8: Commit**

```bash
git add companion/crates companion/apps/deskmate/src-tauri
git commit -m "feat: replace widgets and screens with a single cards array"
```

---

### Task 3: Migration to schema v3 and fixtures

**Files:**
- Modify: `companion/crates/app-core/src/store.rs`
- Modify: `companion/crates/app-core/tests/store.rs`
- Modify: `companion/crates/app-core/tests/fixtures/{default,full,invalid,v2-surface}.json`
- Rename: `companion/crates/app-core/tests/fixtures/future-v3.json` → `future-v4.json`
- Create: `companion/crates/app-core/tests/fixtures/v2-legacy.json`

**Interfaces:**
- Consumes: `AppConfig`, `CardSettings`, `CardPresence`, `CardAlert`, `AlertHold`, `CarouselAdvance` from Task 2.
- Produces: `ConfigOrigin::MigratedV2` added to the existing `ConfigOrigin` enum. `decode_config` handles versions 0, 1, 2, and 3.

- [ ] **Step 1: Write the failing test**

Create `companion/crates/app-core/tests/fixtures/v2-legacy.json`. Note the deliberate mismatch between widget order and screen order — that is what proves card order follows `screens[]`:

```json
{
  "schema_version": 2,
  "preferences": {
    "timezone": "Asia/Tbilisi",
    "autostart": true,
    "paused": false,
    "orientation": "landscape-flipped"
  },
  "widgets": [
    {
      "kind": "pomodoro",
      "id": "focus",
      "size": "standard",
      "label": "Focus",
      "duration_seconds": 1500,
      "template": { "kind": "progress-ring" },
      "tap_action": { "kind": "start-pause" },
      "refresh": { "kind": "device-local" },
      "interrupt_policy": "enabled"
    },
    {
      "kind": "clock",
      "id": "clock",
      "size": "full",
      "title": "Desk",
      "show_seconds": true,
      "template": { "kind": "digital-clock" },
      "tap_action": { "kind": "none" },
      "refresh": { "kind": "device-local" },
      "interrupt_policy": "disabled"
    }
  ],
  "screens": [
    { "id": "clock-screen", "layout": { "kind": "single", "widget_id": "clock" } },
    { "id": "focus-screen", "layout": { "kind": "single", "widget_id": "focus" } }
  ],
  "assets": [],
  "carousel": { "auto_advance_seconds": 30 },
  "updater": { "channel": "stable", "checks": "notify" }
}
```

Append to `companion/crates/app-core/tests/store.rs`:

```rust
#[test]
fn v2_documents_migrate_to_cards_in_screen_order() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, include_str!("fixtures/v2-legacy.json")).unwrap();

    let outcome = ConfigStore::new(&path).load();
    assert_eq!(outcome.origin, ConfigOrigin::MigratedV2);
    assert!(outcome.recovery.is_none());

    let config = outcome.config;
    assert_eq!(config.schema_version, 3);

    // Order follows screens[], not widgets[].
    let ids: Vec<&str> = config.cards.iter().map(CardSettings::id).collect();
    assert_eq!(ids, ["clock", "focus"]);

    // Every migrated card is in the rotation, inheriting the global dwell.
    assert!(
        config
            .cards
            .iter()
            .all(|card| card.presence() == CardPresence::InRotation { dwell_seconds: None })
    );

    // interrupt_policy: enabled becomes the kind-appropriate alert.
    assert_eq!(
        config.cards[1].alert(),
        CardAlert::OnTimerFinish { hold: AlertHold::UntilDismissed }
    );
    assert_eq!(config.cards[0].alert(), CardAlert::None);

    // auto_advance_seconds becomes timed advance.
    assert_eq!(
        config.carousel.advance,
        CarouselAdvance::Timed { default_dwell_seconds: 30 }
    );

    // Preferences survive untouched.
    assert_eq!(config.preferences.timezone, "Asia/Tbilisi");
    assert_eq!(config.preferences.orientation, DisplayOrientation::LandscapeFlipped);
}

#[test]
fn migrated_v2_documents_always_satisfy_the_rotation_rule() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, include_str!("fixtures/v2-legacy.json")).unwrap();
    let config = ConfigStore::new(&path).load().config;
    assert!(config.validate().is_ok());
}

#[test]
fn unknown_future_versions_still_fail_safely() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, include_str!("fixtures/future-v4.json")).unwrap();

    let outcome = ConfigStore::new(&path).load();
    assert_eq!(outcome.origin, ConfigOrigin::LastGood);
    assert!(matches!(
        outcome.recovery,
        Some(StoreError::UnsupportedVersion { found: 4, supported: 3 })
    ));
    // The unreadable source bytes are never rewritten.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        include_str!("fixtures/future-v4.json")
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p app-core --test store v2_documents 2>&1 | tail -20`
Expected: FAIL — `fixtures/v2-legacy.json` cannot be parsed / `ConfigOrigin::MigratedV2` not found.

- [ ] **Step 3: Rename the future-version fixture**

```bash
cd companion/crates/app-core/tests/fixtures
git mv future-v3.json future-v4.json
```

Edit `future-v4.json` and set `"schema_version": 4`. Update every reference to `future-v3.json` in `tests/store.rs` to `future-v4.json`.

- [ ] **Step 4: Implement the migration**

In `companion/crates/app-core/src/store.rs`:

1. Add `MigratedV2` to `ConfigOrigin`.
2. Add the v2 legacy DTOs beside the existing `LegacyConfigV0`/`LegacyConfigV1`:

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyConfigV2 {
    schema_version: u32,
    preferences: AppPreferences,
    widgets: Vec<LegacyWidgetV2>,
    screens: Vec<LegacyScreenV2>,
    #[serde(default)]
    assets: Vec<AssetSettings>,
    #[serde(default)]
    carousel: LegacyCarouselV2,
    #[serde(default)]
    updater: UpdaterSettings,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyCarouselV2 {
    #[serde(default)]
    auto_advance_seconds: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyScreenV2 {
    id: String,
    layout: LegacyLayoutV2,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum LegacyLayoutV2 {
    Single { widget_id: String },
}
```

`LegacyWidgetV2` mirrors the v2 `WidgetSettings` shape exactly: the six kinds, each with `id`, `size: LegacyWidgetSize`, its provider fields, `template`, `tap_action`, `refresh`, and `interrupt_policy: LegacyInterruptPolicy`. Add `LegacyWidgetSize` (`Full`/`Standard`/`Tile`) and `LegacyInterruptPolicy` (`Disabled`/`Enabled`) as local `#[derive(Deserialize)]` enums with `#[serde(rename_all = "kebab-case")]`, since the real types no longer exist.

**Note the deliberate omission of a dashboard variant.** `LegacyLayoutV2` has only `Single`. A dashboard layout can never appear in a persisted file, because v2 rejected it with `requires-capability` during validation and validation runs before persistence. A dashboard document therefore fails to deserialize, which is the correct outcome — it was never valid.

3. Add the migration function:

```rust
fn migrate_v2(legacy: LegacyConfigV2) -> AppConfig {
    let mut by_id: HashMap<String, LegacyWidgetV2> = legacy
        .widgets
        .into_iter()
        .map(|widget| (widget.id().to_owned(), widget))
        .collect();

    // Card order follows screens[]: that is the carousel order the user authored.
    let mut cards = Vec::with_capacity(by_id.len());
    for screen in legacy.screens {
        let LegacyLayoutV2::Single { widget_id } = screen.layout;
        if let Some(widget) = by_id.remove(&widget_id) {
            cards.push(card_from_legacy(
                widget,
                CardPresence::InRotation { dwell_seconds: None },
            ));
        }
    }

    // Widgets with no referencing screen: v2 validation should prevent these, but the
    // protocol permits them. Preserve them rather than silently dropping configuration.
    let mut orphans: Vec<LegacyWidgetV2> = by_id.into_values().collect();
    orphans.sort_by(|left, right| left.id().cmp(right.id()));
    for widget in orphans {
        let presence = if widget.interrupts_enabled() {
            CardPresence::AlertOnly
        } else {
            CardPresence::Off
        };
        cards.push(card_from_legacy(widget, presence));
    }

    AppConfig {
        schema_version: CURRENT_SCHEMA_VERSION,
        preferences: legacy.preferences,
        cards,
        assets: legacy.assets,
        carousel: CarouselSettings {
            advance: match legacy.carousel.auto_advance_seconds {
                Some(seconds) => CarouselAdvance::Timed { default_dwell_seconds: seconds },
                None => CarouselAdvance::Manual,
            },
        },
        updater: legacy.updater,
    }
}
```

`card_from_legacy(widget, presence)` drops `size`, carries every other field across unchanged, and maps the interrupt policy to an alert:

```rust
fn legacy_alert(kind_is_pomodoro: bool, kind_is_calendar: bool, enabled: bool) -> CardAlert {
    if !enabled {
        return CardAlert::None;
    }
    if kind_is_pomodoro {
        return CardAlert::OnTimerFinish { hold: AlertHold::UntilDismissed };
    }
    if kind_is_calendar {
        return CardAlert::BeforeEvent {
            lead_minutes: 5,
            hold: AlertHold::Seconds { value: 60 },
        };
    }
    // The other four kinds had no trigger that could ever fire.
    CardAlert::None
}
```

4. In `decode_config`, add the `2 => { ... }` arm producing `(migrate_v2(legacy), ConfigOrigin::MigratedV2)`, guarding `legacy.schema_version != 2` exactly as the existing arms do.
5. Change the v0/v1 `migrate_legacy` to emit v3 cards directly: build `CardSettings` with `presence: CardPresence::InRotation { dwell_seconds: None }` and the alert derived by the same `legacy_alert` rule (v0/v1 pomodoro widgets got `interrupt_policy: Enabled`, so they become `OnTimerFinish`). Order still follows the legacy `screens` array.

- [ ] **Step 5: Update the JSON fixtures**

Rewrite `default.json`, `full.json`, `invalid.json`, and `v2-surface.json` (rename it `v3-surface.json`) to schema 3: `"schema_version": 3`, `widgets`/`screens` replaced by `cards`, every card carrying `presence` and `alert`, no `size`, and `"carousel": { "advance": { "kind": "manual" } }`.

`invalid.json` must now exercise the new boundaries. Include at least: a card with `presence: alert-only` and `alert: {"kind":"none"}`; a `dwell_seconds` of `4`; a `lead_minutes` of `61`; a `hold` of `{"kind":"seconds","value":4}`; an `on-timer-finish` alert on a clock card; and a duplicate card ID.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cd companion && cargo test -p app-core`
Expected: PASS. The `invalid.json` test asserts every new boundary code; extend its expected-issue list to match the fixture you wrote.

- [ ] **Step 7: Full verification**

Run: `cd companion && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add companion/crates/app-core
git commit -m "feat: migrate v0, v1, and v2 configs to the card model"
```

---

### Task 4: Regenerate the TypeScript IPC contract

The frontend will not compile after Task 2. This task realigns the DTOs so it does, without yet rebuilding the UI.

**Files:**
- Modify: `companion/apps/deskmate/src-tauri/src/commands.rs` (`contract_fixtures()`)
- Modify: `companion/apps/deskmate/src/lib/types.ts`
- Regenerate: `companion/apps/deskmate/src/lib/types.contract.ts`
- Modify: `companion/apps/deskmate/src/lib/configDraft.ts`, `components/*.tsx`, `App.tsx` (minimal compile fixes)

**Interfaces:**
- Consumes: the Rust types from Tasks 1–3.
- Produces TypeScript types: `CardSettings`, `CardPresence`, `CardAlert`, `AlertHold`, `CarouselAdvance`, `CardKind = CardSettings["kind"]`. `AppConfig.cards: CardSettings[]`. `IpcContractFixtures` gains `cards`, `card_presences`, `card_alerts`, `alert_holds`, `carousel_advances`; it loses `widget_settings`, `widget_sizes`, `interrupt_policies`, `screen_layouts`.

- [ ] **Step 1: Write the failing test**

Append to `companion/apps/deskmate/tests/configDraft.test.ts`:

```ts
import { expect, test } from "bun:test";
import { ipcContractFixtures } from "../src/lib/types.contract";

test("contract fixtures expose cards, not widgets or screens", () => {
  const config = ipcContractFixtures.snapshot.config;
  expect(config.schema_version).toBe(3);
  expect(Array.isArray(config.cards)).toBe(true);
  expect("widgets" in config).toBe(false);
  expect("screens" in config).toBe(false);
  expect(config.cards[0]).not.toHaveProperty("size");
  expect(config.cards[0]).toHaveProperty("presence");
  expect(config.cards[0]).toHaveProperty("alert");
});

test("every presence and alert variant is represented in the contract", () => {
  const presenceKinds = ipcContractFixtures.card_presences.map((p) => p.kind).sort();
  expect(presenceKinds).toEqual(["alert-only", "in-rotation", "off"]);

  const alertKinds = ipcContractFixtures.card_alerts.map((a) => a.kind).sort();
  expect(alertKinds).toEqual(["before-event", "none", "on-timer-finish"]);

  const advanceKinds = ipcContractFixtures.carousel_advances.map((a) => a.kind).sort();
  expect(advanceKinds).toEqual(["manual", "timed"]);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion/apps/deskmate && bun test tests/configDraft.test.ts 2>&1 | tail -20`
Expected: FAIL — `config.cards` undefined, `card_presences` not exported.

- [ ] **Step 3: Update the Rust contract fixture builder**

In `companion/apps/deskmate/src-tauri/src/commands.rs`, inside `contract_fixtures()`: replace `widget_settings`, `widget_sizes`, `interrupt_policies`, and `screen_layouts` with exhaustive lists of the new types.

```rust
            card_presences: vec![
                CardPresence::InRotation { dwell_seconds: Some(30) },
                CardPresence::InRotation { dwell_seconds: None },
                CardPresence::AlertOnly,
                CardPresence::Off,
            ],
            card_alerts: vec![
                CardAlert::None,
                CardAlert::OnTimerFinish { hold: AlertHold::UntilDismissed },
                CardAlert::BeforeEvent {
                    lead_minutes: 5,
                    hold: AlertHold::Seconds { value: 60 },
                },
            ],
            alert_holds: vec![
                AlertHold::UntilDismissed,
                AlertHold::Seconds { value: 60 },
            ],
            carousel_advances: vec![
                CarouselAdvance::Manual,
                CarouselAdvance::Timed { default_dwell_seconds: 20 },
            ],
```

The `cards` list must contain one instance of **every** card kind, so the TypeScript union is exercised end to end.

- [ ] **Step 4: Update the hand-written TypeScript DTOs**

In `companion/apps/deskmate/src/lib/types.ts`, delete `WidgetSize`, `WidgetInterruptPolicy`, `ScreenSettings`, `ScreenLayout`, `TileSettings`, and rename `WidgetSettings` to `CardSettings`, mirroring the Rust changes:

```ts
export type CardPresence =
  | { kind: "in-rotation"; dwell_seconds: number | null }
  | { kind: "alert-only" }
  | { kind: "off" };

export type AlertHold =
  | { kind: "until-dismissed" }
  | { kind: "seconds"; value: number };

export type CardAlert =
  | { kind: "none" }
  | { kind: "on-timer-finish"; hold: AlertHold }
  | { kind: "before-event"; lead_minutes: number; hold: AlertHold };

export type CarouselAdvance =
  | { kind: "manual" }
  | { kind: "timed"; default_dwell_seconds: number };

export interface CarouselSettings {
  advance: CarouselAdvance;
}

export type CardKind = CardSettings["kind"];
```

Every `CardSettings` variant drops `size`, drops `interrupt_policy`, and gains `presence: CardPresence` and `alert: CardAlert`. `AppConfig.widgets`/`screens` become `cards: CardSettings[]`. Update `IpcContractFixtures` to match Step 3.

- [ ] **Step 5: Regenerate the contract fixture**

```bash
cd companion/apps/deskmate/src-tauri
cargo test print_typescript_contract_fixture -- --ignored --nocapture --exact \
  > ../src/lib/types.contract.ts.raw
```

Strip the cargo test harness lines (`running N tests`, `test result: …`, blank leader/trailer) from `types.contract.ts.raw` so the file begins with `// Generated by the Rust IPC contract test;` and ends with a single trailing newline, then move it into place:

```bash
mv ../src/lib/types.contract.ts.raw ../src/lib/types.contract.ts
cargo test typescript_contract_fixture_stays_in_sync
```

Expected: PASS. If it fails, the stripping was wrong — the test compares the file byte-for-byte.

- [ ] **Step 6: Make the frontend compile**

Run: `cd companion/apps/deskmate && bun run check 2>&1 | head -40`

Apply the minimal mechanical fixes only — the real UI rebuild is Tasks 8–11:
- `configDraft.ts`: `config.widgets` → `config.cards`; delete `screenWidgetIds`, `primaryScreenWidgetId`, `moveScreen`, `screenMoveFromKey`; `addWidget` no longer pushes a screen.
- `App.tsx`, `DevicePreview.tsx`, `ScreenArranger.tsx`, `WidgetGallery.tsx`, `WidgetEditor.tsx`: replace `widgets`/`screens` props with `cards`. Keep them ugly; they are replaced shortly.

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd companion/apps/deskmate && bun test && bun run check && bun run lint && bun run format:check && bun run build`
Expected: all pass, including the two tests from Step 1.

- [ ] **Step 8: Commit**

```bash
git add companion/apps/deskmate
git commit -m "feat: realign the typed IPC contract onto the card model"
```

---

### Task 5: Host-driven timed rotation

**Files:**
- Modify: `companion/crates/app-core/src/scheduler.rs`
- Modify: `companion/crates/app-core/src/runtime.rs`
- Test: `companion/crates/app-core/src/scheduler.rs` (inline `mod tests`), `companion/crates/app-core/tests/runtime.rs`

**Interfaces:**
- Consumes: `CardPresence`, `CarouselAdvance`, `AppConfig::cards`.
- Produces on `Scheduler`: `set_rotation(&mut self, dwell: Option<Duration>, now: Instant)`, `rotation_due(&mut self, now: Instant) -> bool`, `clear_rotation(&mut self)`. `wait_duration` accounts for the rotation deadline.
- Produces on the runtime worker: `rotation_card_ids(config: &AppConfig) -> Vec<String>` (in-rotation card IDs in card order) and an `active_rotation_index: usize` on `WorkerState`.

- [ ] **Step 1: Write the failing test**

Append to the `mod tests` in `companion/crates/app-core/src/scheduler.rs`:

```rust
#[test]
fn rotation_deadline_fires_once_per_dwell_and_bounds_the_wait() {
    let now = Instant::now();
    let mut scheduler = Scheduler::new(
        now,
        Duration::from_secs(60),
        Duration::from_secs(60),
        Duration::from_secs(60),
    );

    // Manual advance: no rotation deadline exists.
    scheduler.set_rotation(None, now);
    assert!(!scheduler.rotation_due(now + Duration::from_secs(3_600)));

    // Timed advance: fires once at the deadline, then rearms.
    scheduler.set_rotation(Some(Duration::from_secs(20)), now);
    assert!(!scheduler.rotation_due(now + Duration::from_secs(19)));
    assert!(scheduler.rotation_due(now + Duration::from_secs(20)));
    assert!(!scheduler.rotation_due(now + Duration::from_secs(20)));
    assert!(scheduler.rotation_due(now + Duration::from_secs(40)));

    // The loop cannot sleep past a pending rotation.
    scheduler.set_rotation(Some(Duration::from_secs(5)), now);
    assert_eq!(
        scheduler.wait_duration(now, Duration::from_secs(3_600)),
        Duration::from_secs(5)
    );

    scheduler.clear_rotation();
    assert!(!scheduler.rotation_due(now + Duration::from_secs(3_600)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p app-core --lib rotation_deadline 2>&1 | tail -20`
Expected: FAIL — no method `set_rotation` on `Scheduler`.

- [ ] **Step 3: Write minimal implementation**

In `companion/crates/app-core/src/scheduler.rs`, add to the struct and impl:

```rust
pub(crate) struct Scheduler {
    // … existing fields …
    rotation: Option<RotationDeadline>,
}

struct RotationDeadline {
    dwell: Duration,
    next: Instant,
}
```

```rust
    pub(crate) fn set_rotation(&mut self, dwell: Option<Duration>, now: Instant) {
        self.rotation = dwell.map(|dwell| RotationDeadline { dwell, next: now + dwell });
    }

    pub(crate) fn clear_rotation(&mut self) {
        self.rotation = None;
    }

    pub(crate) fn rotation_due(&mut self, now: Instant) -> bool {
        let Some(rotation) = self.rotation.as_mut() else {
            return false;
        };
        if now < rotation.next {
            return false;
        }
        rotation.next = now + rotation.dwell;
        true
    }
```

Initialise `rotation: None` in `Scheduler::new`, and include it in `wait_duration`:

```rust
        let next = self
            .providers
            .values()
            .filter_map(|deadline| deadline.next)
            .chain(self.rotation.iter().map(|rotation| rotation.next))
            .chain([self.next_pomodoro, self.next_status, self.next_time_sync])
            .min()
            .unwrap_or(now + maximum);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd companion && cargo test -p app-core --lib rotation_deadline`
Expected: PASS.

- [ ] **Step 5: Write the runtime advance test**

Append to `companion/crates/app-core/tests/runtime.rs`, following the existing fake-device harness in that file:

```rust
#[test]
fn timed_advance_walks_in_rotation_cards_and_skips_the_others() {
    let config = AppConfig {
        carousel: CarouselSettings {
            advance: CarouselAdvance::Timed { default_dwell_seconds: 5 },
        },
        cards: vec![
            clock_card("first", CardPresence::InRotation { dwell_seconds: Some(5) }),
            pomodoro_card(
                "alerting",
                CardPresence::AlertOnly,
                CardAlert::OnTimerFinish { hold: AlertHold::UntilDismissed },
            ),
            clock_card("second", CardPresence::InRotation { dwell_seconds: Some(5) }),
            clock_card("muted", CardPresence::Off),
        ],
        ..AppConfig::default()
    };

    let harness = RuntimeHarness::start(config);
    harness.advance_rotation();
    harness.advance_rotation();
    harness.advance_rotation();

    // Only in-rotation cards are activated, in card order, wrapping at the end.
    assert_eq!(
        harness.activated_screen_ids(),
        ["second", "first", "second"]
    );
}

#[test]
fn manual_advance_never_activates_a_screen_on_its_own() {
    let config = AppConfig {
        carousel: CarouselSettings { advance: CarouselAdvance::Manual },
        ..AppConfig::default()
    };
    let harness = RuntimeHarness::start(config);
    harness.advance_rotation();
    assert!(harness.activated_screen_ids().is_empty());
}
```

If `RuntimeHarness` does not already expose `advance_rotation()` and `activated_screen_ids()`, add them: `advance_rotation` drives the worker loop forward past one dwell using the same injected-clock mechanism the existing provider tests use; `activated_screen_ids` returns the `ActivateScreen` messages the fake device recorded.

- [ ] **Step 6: Run test to verify it fails**

Run: `cd companion && cargo test -p app-core --test runtime timed_advance 2>&1 | tail -20`
Expected: FAIL — no screens activated.

- [ ] **Step 7: Implement rotation in the runtime worker**

In `companion/crates/app-core/src/runtime.rs`:

1. Add `active_rotation_index: usize` to `WorkerState`, initialised to `0`.
2. Add the helper:

```rust
fn rotation_card_ids(config: &AppConfig) -> Vec<String> {
    config
        .cards
        .iter()
        .filter(|card| card.presence().is_in_rotation())
        .map(|card| card.id().to_owned())
        .collect()
}
```

3. Wherever the config is installed or replaced (the same place `scheduler.replace_providers` is called), arm the rotation deadline from the *current* card's dwell and reset the index:

```rust
    let ids = rotation_card_ids(&state.config);
    state.active_rotation_index = 0;
    scheduler.set_rotation(current_dwell(&state.config, state.active_rotation_index), now);
```

with:

```rust
fn current_dwell(config: &AppConfig, index: usize) -> Option<Duration> {
    let default = config.carousel.advance.default_dwell_seconds()?;
    let card = config
        .cards
        .iter()
        .filter(|card| card.presence().is_in_rotation())
        .nth(index)?;
    card.presence()
        .dwell_seconds(default)
        .map(|seconds| Duration::from_secs(u64::from(seconds)))
}
```

`current_dwell` returns `None` under `CarouselAdvance::Manual`, so `set_rotation(None, ..)` disarms the deadline — that is what makes the manual test pass.

4. In the main loop, beside the existing `scheduler.status_due(now)` / `time_sync_due(now)` handling, add:

```rust
        if scheduler.rotation_due(now) {
            let ids = rotation_card_ids(&state.config);
            if ids.len() > 1 {
                state.active_rotation_index = (state.active_rotation_index + 1) % ids.len();
                state.pending_active_screen = Some(ids[state.active_rotation_index].clone());
                state.active_screen_dirty = true;
            }
            // Re-arm for the card we just moved to; dwell is per-card.
            scheduler.set_rotation(current_dwell(&state.config, state.active_rotation_index), now);
        }
```

Reuse the existing `active_screen_dirty` flush path (runtime.rs:1655-1664) rather than calling `device.activate_screen` inline, so a disconnected device queues the change for replay exactly as a manual activation does.

5. When a device event reports a local swipe (`EventAction::NavigatePrevious` / `NavigateNext`), set `active_rotation_index` to the index of the reported screen ID and re-arm the deadline, so a manual swipe restarts the dwell rather than advancing early.

- [ ] **Step 8: Run tests to verify they pass**

Run: `cd companion && cargo test -p app-core`
Expected: PASS.

- [ ] **Step 9: Full verification**

Run: `cd companion && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

- [ ] **Step 10: Commit**

```bash
git add companion/crates/app-core
git commit -m "feat: add host-driven timed carousel rotation"
```

---

### Task 6: Alert triggers and hold

Narrow the existing pomodoro interrupt to `CardAlert::OnTimerFinish`, add the calendar `BeforeEvent` trigger, and implement bounded `hold`.

**Files:**
- Modify: `companion/crates/app-core/src/runtime.rs`
- Modify: `companion/crates/app-core/src/scheduler.rs`
- Test: `companion/crates/app-core/tests/runtime.rs`

**Interfaces:**
- Consumes: `CardAlert`, `AlertHold`, and `engine::interrupts::InterruptState::schedule(widget_id, reason)` (existing, returns `Result<TriggerInterrupt, InterruptError>`).
- Produces on `Scheduler`: `set_alert_hold(&mut self, deadline: Option<Instant>)`, `alert_hold_due(&mut self, now: Instant) -> bool`. `wait_duration` accounts for the hold deadline.
- Produces on the worker: `armed_event_alerts: BTreeMap<String, i64>` mapping card ID to the unix-ms start time of the event it has already fired for, so one event fires exactly once.

- [ ] **Step 1: Write the failing test**

Append to `companion/crates/app-core/tests/runtime.rs`:

```rust
#[test]
fn only_matching_alert_kinds_fire() {
    let config = AppConfig {
        cards: vec![
            clock_card("clock", CardPresence::InRotation { dwell_seconds: None }),
            pomodoro_card("quiet", CardPresence::InRotation { dwell_seconds: None }, CardAlert::None),
            pomodoro_card(
                "loud",
                CardPresence::InRotation { dwell_seconds: None },
                CardAlert::OnTimerFinish { hold: AlertHold::UntilDismissed },
            ),
        ],
        ..AppConfig::default()
    };
    let harness = RuntimeHarness::start(config);

    harness.complete_pomodoro("quiet");
    harness.complete_pomodoro("loud");

    let fired: Vec<String> = harness
        .triggered_interrupts()
        .into_iter()
        .map(|interrupt| interrupt.widget_id)
        .collect();
    assert_eq!(fired, ["loud"]);
}

#[test]
fn a_bounded_hold_dismisses_itself_and_until_dismissed_does_not() {
    let bounded = AppConfig {
        cards: vec![pomodoro_card(
            "loud",
            CardPresence::InRotation { dwell_seconds: None },
            CardAlert::OnTimerFinish { hold: AlertHold::Seconds { value: 30 } },
        )],
        ..AppConfig::default()
    };
    let harness = RuntimeHarness::start(bounded);
    harness.complete_pomodoro("loud");
    assert_eq!(harness.triggered_interrupts().len(), 1);

    harness.advance_time(Duration::from_secs(30));
    assert_eq!(harness.dismissed_interrupts().len(), 1);

    let sticky = AppConfig {
        cards: vec![pomodoro_card(
            "loud",
            CardPresence::InRotation { dwell_seconds: None },
            CardAlert::OnTimerFinish { hold: AlertHold::UntilDismissed },
        )],
        ..AppConfig::default()
    };
    let harness = RuntimeHarness::start(sticky);
    harness.complete_pomodoro("loud");
    harness.advance_time(Duration::from_secs(3_600));
    assert!(harness.dismissed_interrupts().is_empty());
}

#[test]
fn a_calendar_alert_fires_once_per_event_inside_its_lead_window() {
    let config = AppConfig {
        cards: vec![calendar_card(
            "upnext",
            CardPresence::InRotation { dwell_seconds: None },
            CardAlert::BeforeEvent {
                lead_minutes: 5,
                hold: AlertHold::Seconds { value: 60 },
            },
        )],
        ..AppConfig::default()
    };
    let harness = RuntimeHarness::start(config);

    // Event starts in 6 minutes: outside the window, nothing fires.
    harness.push_next_event("upnext", 6 * 60);
    assert!(harness.triggered_interrupts().is_empty());

    // Event starts in 4 minutes: inside the window, fires exactly once.
    harness.push_next_event("upnext", 4 * 60);
    harness.push_next_event("upnext", 3 * 60);
    assert_eq!(harness.triggered_interrupts().len(), 1);

    // A different event re-arms the trigger.
    harness.push_next_event_starting_at("upnext", 4 * 60, /* distinct start */ 2);
    assert_eq!(harness.triggered_interrupts().len(), 2);
}
```

Add `calendar_card(id, presence, alert)` to the test helpers alongside `clock_card`/`pomodoro_card`, and add the harness methods `complete_pomodoro`, `triggered_interrupts`, `dismissed_interrupts`, `advance_time`, `push_next_event`, and `push_next_event_starting_at` following the existing fake-device recording pattern.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p app-core --test runtime alert 2>&1 | tail -20`
Expected: FAIL — the `quiet` pomodoro also fires (Task 2 gated on "any alert"), and no calendar trigger exists.

- [ ] **Step 3: Narrow the pomodoro trigger**

At runtime.rs:962, :1498, and :1535, replace the "alert is not none" gate written in Task 2 with an exact match:

```rust
    if matches!(card.alert(), CardAlert::OnTimerFinish { .. }) {
        let _ = state.interrupts.schedule(widget_id, "Timer finished");
    }
```

- [ ] **Step 4: Implement the hold deadline**

Add to `Scheduler` exactly as in Task 5 but with an absolute deadline rather than a repeating interval:

```rust
    pub(crate) fn set_alert_hold(&mut self, deadline: Option<Instant>) {
        self.alert_hold = deadline;
    }

    pub(crate) fn alert_hold_due(&mut self, now: Instant) -> bool {
        let Some(deadline) = self.alert_hold else {
            return false;
        };
        if now < deadline {
            return false;
        }
        self.alert_hold = None;
        true
    }
```

Add `alert_hold: Option<Instant>` to the struct, initialise to `None`, and chain it into `wait_duration`.

When an interrupt is scheduled, arm the hold from the firing card's alert:

```rust
    let hold = match card.alert().hold() {
        Some(AlertHold::Seconds { value }) => {
            Some(now + Duration::from_secs(u64::from(value)))
        }
        Some(AlertHold::UntilDismissed) | None => None,
    };
    scheduler.set_alert_hold(hold);
```

In the main loop, dismiss the active interrupt when the hold expires, reusing the existing dismissal path so the device restores its saved carousel screen:

```rust
        if scheduler.alert_hold_due(now) {
            if let Some(token) = state.interrupts.active().map(|tracked| tracked.message.token) {
                let _ = state.interrupts.dismiss(token);
                state.active_screen_dirty = true;
            }
        }
```

- [ ] **Step 5: Implement the calendar trigger**

Add `armed_event_alerts: BTreeMap<String, i64>` to `WorkerState`. Whenever calendar fields land for a card (the same place `state.latest_fields.insert` happens for provider results), evaluate:

```rust
fn evaluate_event_alert(
    state: &mut WorkerState,
    card_id: &str,
    lead_minutes: u16,
    now_unix_ms: i64,
) -> bool {
    let Some(start_unix_ms) = next_event_start_unix_ms(state, card_id) else {
        return false;
    };
    let lead_ms = i64::from(lead_minutes) * 60_000;
    let within_window = start_unix_ms > now_unix_ms && start_unix_ms - now_unix_ms <= lead_ms;
    if !within_window {
        return false;
    }
    // Fire once per distinct event start.
    if state.armed_event_alerts.get(card_id) == Some(&start_unix_ms) {
        return false;
    }
    state.armed_event_alerts.insert(card_id.into(), start_unix_ms);
    true
}
```

`next_event_start_unix_ms` reads the card's next event start out of `state.latest_fields`.

**This field does not exist yet and must be added first.** `IcsCalendar::fields` (`companion/crates/providers/src/ics.rs:103-141`) emits `title`, then `row{N}_title` and `row{N}_time` as **formatted display text** (`"10:30"`, `"Today · All day"`, `"Wed 09:15"`), then a `stale` boolean. Triggering an alert by parsing a localised display string would be unsound — it is lossy, timezone-formatted, and drops the date for non-today events.

Add a machine-readable field before wiring the trigger:

```rust
        fields.push(Field {
            key: "next_start_unix_ms".into(),
            value: FieldValue::Integer(
                self.events
                    .first()
                    .map_or(0, |event| event.start.timestamp_millis()),
            ),
        });
```

Emit `0` when there is no next event, and treat `0` as "no event" in `evaluate_event_alert`. Bump the `Vec::with_capacity(13)` at `ics.rs:112` to match the new field count, and add an `ics.rs` fixture test asserting the integer equals the parsed `DTSTART` for a known fixture — including a recurring event, where the value must be the *next occurrence*, not the series start.

Prune `armed_event_alerts` in `retain_widgets`-style fashion whenever the config is replaced, so removed cards do not leak entries.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cd companion && cargo test -p app-core --test runtime`
Expected: PASS.

- [ ] **Step 7: Full verification**

Run: `cd companion && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add companion/crates
git commit -m "feat: add bounded timer and calendar alert triggers"
```

---

### Task 7: Carry field values to the preview

**Files:**
- Modify: `companion/crates/app-core/src/state.rs`
- Modify: `companion/crates/app-core/src/runtime.rs`
- Modify: `companion/apps/deskmate/src-tauri/src/commands.rs`
- Modify: `companion/apps/deskmate/src/lib/types.ts`
- Regenerate: `companion/apps/deskmate/src/lib/types.contract.ts`
- Test: `companion/crates/app-core/tests/config.rs`

**Interfaces:**
- Produces in `app-core`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CardFieldValue {
    Text { value: String },
    Integer { value: i64 },
    Boolean { value: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardField {
    pub key: String,
    pub value: CardFieldValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardDataSnapshot {
    pub card_id: String,
    pub fields: Vec<CardField>,
}
```
  plus `AppSnapshot.card_data: Vec<CardDataSnapshot>`.
- Produces in TypeScript: `CardFieldValue`, `CardField`, `CardDataSnapshot`, `AppSnapshot.card_data`, and `IpcContractFixtures.card_data`.

**Note:** `protocol::Field` / `protocol::FieldValue` deliberately do **not** derive `Serialize`. Do not add serde to the protocol crate — mirror the values into these app-core DTOs instead, so the wire types stay free of presentation concerns.

- [ ] **Step 1: Write the failing test**

Append to `companion/crates/app-core/tests/config.rs`:

```rust
#[test]
fn card_data_serializes_as_tagged_values_for_the_preview() {
    let data = CardDataSnapshot {
        card_id: "upnext".into(),
        fields: vec![
            CardField {
                key: "row0_title".into(),
                value: CardFieldValue::Text { value: "Design review".into() },
            },
            CardField {
                key: "next_start_unix_ms".into(),
                value: CardFieldValue::Integer { value: 1_787_000_000_000 },
            },
            CardField {
                key: "stale".into(),
                value: CardFieldValue::Boolean { value: false },
            },
        ],
    };

    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["card_id"], "upnext");
    assert_eq!(json["fields"][0]["value"]["kind"], "text");
    assert_eq!(json["fields"][1]["value"]["kind"], "integer");
    assert_eq!(json["fields"][2]["value"]["kind"], "boolean");
    assert_eq!(
        serde_json::from_value::<CardDataSnapshot>(json).unwrap(),
        data
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p app-core --test config card_data 2>&1 | tail -20`
Expected: FAIL — `CardDataSnapshot` not found.

- [ ] **Step 3: Write minimal implementation**

Add the three types to `companion/crates/app-core/src/state.rs`, add `pub card_data: Vec<CardDataSnapshot>` to `AppSnapshot`, re-export from `lib.rs`, and add a conversion:

```rust
impl CardDataSnapshot {
    pub(crate) fn from_protocol(card_id: &str, fields: &[protocol::Field]) -> Self {
        Self {
            card_id: card_id.to_owned(),
            fields: fields
                .iter()
                .map(|field| CardField {
                    key: field.key.clone(),
                    value: match &field.value {
                        protocol::FieldValue::Text(value) => {
                            CardFieldValue::Text { value: value.clone() }
                        }
                        protocol::FieldValue::Integer(value) => {
                            CardFieldValue::Integer { value: *value }
                        }
                        protocol::FieldValue::Boolean(value) => {
                            CardFieldValue::Boolean { value: *value }
                        }
                    },
                })
                .collect(),
        }
    }
}
```

In `runtime.rs`, populate it from the field map the worker already maintains (`state.latest_fields`, runtime.rs:811), inside `WorkerState::snapshot` (runtime.rs:1005):

```rust
            card_data: self
                .latest_fields
                .iter()
                .map(|(card_id, fields)| CardDataSnapshot::from_protocol(card_id, fields))
                .collect(),
```

`latest_fields` is a `BTreeMap`, so ordering is deterministic and the existing `publish_if_changed` equality check keeps working unchanged.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd companion && cargo test -p app-core --test config card_data`
Expected: PASS.

- [ ] **Step 5: Extend and regenerate the contract**

Add `card_data` to `contract_fixtures()` in `commands.rs` with one instance of each `CardFieldValue` variant, add the matching TypeScript types to `types.ts` and `IpcContractFixtures`, then regenerate `types.contract.ts` exactly as in Task 4 Step 5 and re-run `cargo test typescript_contract_fixture_stays_in_sync`.

- [ ] **Step 6: Full verification**

Run:
```bash
cd companion && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cd apps/deskmate && bun test && bun run check && bun run lint && bun run format:check && bun run build
```
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add companion
git commit -m "feat: publish last-good card field values to the settings snapshot"
```

---

### Task 8: The single card list

**Files:**
- Create: `companion/apps/deskmate/src/components/CardList.tsx`
- Delete: `companion/apps/deskmate/src/components/WidgetGallery.tsx`, `companion/apps/deskmate/src/components/ScreenArranger.tsx`
- Modify: `companion/apps/deskmate/src/lib/configDraft.ts`, `src/App.tsx`, `src/styles.css`
- Test: `companion/apps/deskmate/tests/configDraft.test.ts`, `tests/components.test.tsx`

**Interfaces:**
- Consumes: `CardSettings`, `CardPresence`, `CardKind` from Task 4.
- Produces in `configDraft.ts`:
  - `addCard(config: AppConfig, kind: CardKind): { config: AppConfig; cardId: string }` — appends a card with sane defaults; **does not** create a screen.
  - `removeCard(config: AppConfig, cardId: string): AppConfig`
  - `moveCard(config: AppConfig, cardId: string, targetIndex: number): AppConfig` — reorders within `cards[]`.
  - `rotationCards(config: AppConfig): CardSettings[]`, `nonRotationCards(config: AppConfig): CardSettings[]`
  - `cardName(card: CardSettings): string`, `cardKindName(kind: CardKind): string`
  - `loopSeconds(config: AppConfig): number | null` — total rotation length, `null` under manual advance.
- Produces component: `CardList({ config, selectedCardId, onSelect, onAdd, onRemove, onReorder })`.

- [ ] **Step 1: Write the failing test**

Append to `companion/apps/deskmate/tests/configDraft.test.ts`:

```ts
import { addCard, loopSeconds, moveCard, nonRotationCards, rotationCards } from "../src/lib/configDraft";

const base: AppConfig = {
  schema_version: 3,
  preferences: { timezone: "UTC", autostart: false, paused: false, orientation: "landscape" },
  cards: [],
  assets: [],
  carousel: { advance: { kind: "timed", default_dwell_seconds: 20 } },
  updater: { channel: "stable", checks: "notify" },
};

test("adding a card appends it without creating a screen", () => {
  const { config, cardId } = addCard(base, "weather");
  expect(config.cards).toHaveLength(1);
  expect(config.cards[0].kind).toBe("weather");
  expect(cardId).toBe("weather");
  expect("screens" in config).toBe(false);
});

test("adding the same kind twice produces unique ids", () => {
  const first = addCard(base, "clock");
  const second = addCard(first.config, "clock");
  expect(second.cardId).toBe("clock-2");
});

test("cards split into rotation and non-rotation sections", () => {
  const config: AppConfig = {
    ...base,
    cards: [
      { ...addCard(base, "clock").config.cards[0], id: "a" },
      {
        ...addCard(base, "pomodoro").config.cards[0],
        id: "b",
        presence: { kind: "alert-only" },
        alert: { kind: "on-timer-finish", hold: { kind: "until-dismissed" } },
      },
      { ...addCard(base, "clock").config.cards[0], id: "c", presence: { kind: "off" } },
    ],
  };
  expect(rotationCards(config).map((card) => card.id)).toEqual(["a"]);
  expect(nonRotationCards(config).map((card) => card.id)).toEqual(["b", "c"]);
});

test("loop length sums only in-rotation dwell, inheriting the default", () => {
  const config: AppConfig = {
    ...base,
    cards: [
      { ...addCard(base, "clock").config.cards[0], id: "a",
        presence: { kind: "in-rotation", dwell_seconds: 45 } },
      { ...addCard(base, "clock").config.cards[0], id: "b",
        presence: { kind: "in-rotation", dwell_seconds: null } },
      { ...addCard(base, "clock").config.cards[0], id: "c", presence: { kind: "off" } },
    ],
  };
  expect(loopSeconds(config)).toBe(65);
  expect(loopSeconds({ ...config, carousel: { advance: { kind: "manual" } } })).toBeNull();
});

test("reordering moves a card within the array", () => {
  const config: AppConfig = {
    ...base,
    cards: [
      { ...addCard(base, "clock").config.cards[0], id: "a" },
      { ...addCard(base, "clock").config.cards[0], id: "b" },
      { ...addCard(base, "clock").config.cards[0], id: "c" },
    ],
  };
  expect(moveCard(config, "c", 0).cards.map((card) => card.id)).toEqual(["c", "a", "b"]);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion/apps/deskmate && bun test tests/configDraft.test.ts 2>&1 | tail -20`
Expected: FAIL — `addCard` is not exported.

- [ ] **Step 3: Write minimal implementation**

Rewrite `companion/apps/deskmate/src/lib/configDraft.ts`. `addCard` supports **all six kinds** — this closes the defect where weather, JSON feed, and RSS were unreachable:

```ts
export type CardKind = CardSettings["kind"];

const DEFAULT_PRESENCE: CardPresence = { kind: "in-rotation", dwell_seconds: null };

export function addCard(config: AppConfig, kind: CardKind): { config: AppConfig; cardId: string } {
  const used = new Set(config.cards.map((card) => card.id));
  const cardId = nextId(kind, used);
  const common = {
    id: cardId,
    tap_action: { kind: "none" } as const,
    presence: DEFAULT_PRESENCE,
    alert: { kind: "none" } as const,
  };

  let card: CardSettings;
  switch (kind) {
    case "clock":
      card = { kind, ...common, title: "Desk", show_seconds: true,
        template: { kind: "digital-clock" }, refresh: { kind: "device-local" } };
      break;
    case "pomodoro":
      card = { kind, ...common, label: "Focus", duration_seconds: 1500,
        template: { kind: "progress-ring" }, tap_action: { kind: "start-pause" },
        refresh: { kind: "device-local" },
        alert: { kind: "on-timer-finish", hold: { kind: "until-dismissed" } } };
      break;
    case "calendar":
      card = { kind, ...common, title: "Up next", source: { kind: "url", value: "" },
        template: { kind: "row-list" }, refresh: { kind: "interval", minutes: 15 } };
      break;
    case "weather":
      card = { kind, ...common, title: "Weather", location: "", units: "metric",
        template: { kind: "big-number-label" }, refresh: { kind: "interval", minutes: 30 } };
      break;
    case "json-feed":
      card = { kind, ...common, title: "Feed", url: "", mappings: [],
        template: { kind: "big-number-label" }, refresh: { kind: "interval", minutes: 15 } };
      break;
    case "rss":
      card = { kind, ...common, title: "Headlines", url: "", max_items: 3,
        template: { kind: "row-list" }, refresh: { kind: "interval", minutes: 30 } };
      break;
  }

  return { config: { ...copyConfig(config), cards: [...config.cards, card] }, cardId };
}

export function rotationCards(config: AppConfig): CardSettings[] {
  return config.cards.filter((card) => card.presence.kind === "in-rotation");
}

export function nonRotationCards(config: AppConfig): CardSettings[] {
  return config.cards.filter((card) => card.presence.kind !== "in-rotation");
}

export function loopSeconds(config: AppConfig): number | null {
  if (config.carousel.advance.kind !== "timed") {
    return null;
  }
  const fallback = config.carousel.advance.default_dwell_seconds;
  return rotationCards(config).reduce((total, card) => {
    const presence = card.presence;
    return total + (presence.kind === "in-rotation" ? (presence.dwell_seconds ?? fallback) : 0);
  }, 0);
}

export function moveCard(config: AppConfig, cardId: string, targetIndex: number): AppConfig {
  const source = config.cards.findIndex((card) => card.id === cardId);
  if (source < 0 || config.cards.length < 2) {
    return config;
  }
  const bounded = Math.max(0, Math.min(targetIndex, config.cards.length - 1));
  if (source === bounded) {
    return config;
  }
  const cards = [...config.cards];
  const [moved] = cards.splice(source, 1);
  cards.splice(bounded, 0, moved);
  return { ...config, cards };
}
```

Keep `copyConfig` and `nextId`, updated for `cards`. Delete `screenWidgetIds`, `primaryScreenWidgetId`, `moveScreen`, `screenMoveFromKey`, and `firstSelectableWidget` (replace with `firstSelectableCard(config)` returning `config.cards[0]?.id ?? null`).

- [ ] **Step 4: Build the CardList component**

Create `companion/apps/deskmate/src/components/CardList.tsx` with two sections. In-rotation rows are numbered and draggable; alert-only and off rows are not numbered, because they have no carousel position. Carry over the existing drag, `⌥↑/↓` keyboard reorder, and move-button affordances from `ScreenArranger.tsx`, and the add-card buttons from `WidgetGallery.tsx`, but:
- cap adding at **8**, not 16 (`config.cards.length >= 8`), matching the config and wire contract;
- offer all six kinds;
- never render a card ID to the user.

- [ ] **Step 5: Wire it into App.tsx and delete the old panels**

Replace the `WidgetGallery` + `ScreenArranger` usage in `App.tsx:341-365` with a single `<CardList …/>`, then:

```bash
cd companion/apps/deskmate
git rm src/components/WidgetGallery.tsx src/components/ScreenArranger.tsx
```

Remove the `1 ·` / `2 ·` / `3 ·` step-label markup and its CSS — the numbering promised a linear flow the layout never had.

- [ ] **Step 6: Write the component test**

Append to `companion/apps/deskmate/tests/components.test.tsx`, following the render helper already used there:

```tsx
test("the card list separates rotation from alerts and never shows ids", () => {
  const config = /* a config with one in-rotation, one alert-only, one off card */;
  const { getByRole, queryByText } = renderCardList(config);

  expect(getByRole("list", { name: /in rotation/i })).toBeTruthy();
  expect(getByRole("list", { name: /alerts and muted/i })).toBeTruthy();
  // Internal identifiers never reach the user.
  expect(queryByText("clock-screen")).toBeNull();
});

test("adding is disabled at the eight-card contract limit", () => {
  const config = /* eight cards */;
  const { getAllByRole } = renderCardList(config);
  for (const button of getAllByRole("button", { name: /^Add / })) {
    expect((button as HTMLButtonElement).disabled).toBe(true);
  }
});
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd companion/apps/deskmate && bun test && bun run check && bun run lint && bun run format:check && bun run build`
Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add companion/apps/deskmate
git commit -m "feat: replace the widget gallery and screen arranger with one card list"
```

---

### Task 9: The card editor

**Files:**
- Create: `companion/apps/deskmate/src/components/CardEditor.tsx`
- Delete: `companion/apps/deskmate/src/components/WidgetEditor.tsx`
- Modify: `companion/apps/deskmate/src/lib/configDraft.ts`, `src/App.tsx`
- Test: `companion/apps/deskmate/tests/components.test.tsx`

**Interfaces:**
- Consumes: `CardSettings`, `CardPresence`, `CardAlert`, `AlertHold`, `ValidationIssue`.
- Produces: `CardEditor({ card, issues, pomodoro, onChange, onRemove, onTimerAction, onChooseCalendarFile })` and, in `configDraft.ts`, **`issuesForCard(issues: ValidationIssue[], config: AppConfig, cardId: string): ValidationIssue[]`**.

**Critical:** validation issue paths are array-index based (`cards[2].title`). Because `cards[]` is now user-reorderable, indices are no longer stable identities. `issuesForCard` must resolve the card's *current* index from the config and match on that, so dragging a card never moves an error onto a different row.

- [ ] **Step 1: Write the failing test**

Append to `companion/apps/deskmate/tests/configDraft.test.ts`:

```ts
test("issues follow the card across a reorder", () => {
  const config: AppConfig = {
    ...base,
    cards: [
      { ...addCard(base, "clock").config.cards[0], id: "a" },
      { ...addCard(base, "calendar").config.cards[0], id: "b" },
    ],
  };
  const issues: ValidationIssue[] = [
    { path: "cards[1].source", code: "out-of-range", message: "calendar source is required" },
  ];

  expect(issuesForCard(issues, config, "b")).toHaveLength(1);
  expect(issuesForCard(issues, config, "a")).toHaveLength(0);

  // After dragging "b" to the front, the SAME issue list must still attach to "b".
  const reordered = moveCard(config, "b", 0);
  const rebased: ValidationIssue[] = [
    { path: "cards[0].source", code: "out-of-range", message: "calendar source is required" },
  ];
  expect(issuesForCard(rebased, reordered, "b")).toHaveLength(1);
  expect(issuesForCard(rebased, reordered, "a")).toHaveLength(0);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion/apps/deskmate && bun test tests/configDraft.test.ts 2>&1 | tail -20`
Expected: FAIL — `issuesForCard` is not exported.

- [ ] **Step 3: Write minimal implementation**

```ts
export function issuesForCard(
  issues: ValidationIssue[],
  config: AppConfig,
  cardId: string,
): ValidationIssue[] {
  const index = config.cards.findIndex((card) => card.id === cardId);
  if (index < 0) {
    return [];
  }
  const prefix = `cards[${index}]`;
  return issues.filter(
    (issue) => issue.path === prefix || issue.path.startsWith(`${prefix}.`),
  );
}
```

- [ ] **Step 4: Build the CardEditor component**

Create `CardEditor.tsx` from `WidgetEditor.tsx`, then:

1. Add provider fields for weather (`location`, `units`), JSON feed (`url`, `mappings`), and RSS (`url`, `max_items`).
2. Add a **Behaviour** fieldset:
   - Presence radio group: *In rotation* / *Alert only* / *Off*. *Alert only* is disabled with an explanatory hint whenever `alert.kind === "none"`, so the dead-card combination cannot be produced by the UI.
   - When in rotation: a dwell control offering *Use the default (N s)* plus explicit values, writing `dwell_seconds: null` for the default.
3. Add an **Alert** fieldset, rendered **only** for pomodoro and calendar cards — the other four kinds have no trigger that could ever fire, so showing a disabled control would be noise:
   - Pomodoro: a checkbox for *Take over the screen when the timer ends*, plus a hold selector.
   - Calendar: a checkbox for *Take over the screen before an event*, plus lead-time and hold selectors.
4. State the card's tap behaviour in plain language, and note that a tap dismisses an active alert instead of performing the card action.
5. Use `issuesForCard` for every error lookup. Do not accept a `cardIndex` prop.

- [ ] **Step 5: Wire it in and delete the old editor**

Replace `WidgetEditor` in `App.tsx` with `CardEditor`, then `git rm src/components/WidgetEditor.tsx`.

- [ ] **Step 6: Write the component test**

```tsx
test("alert-only is unavailable until an alert is configured", () => {
  const card = /* pomodoro card with alert: { kind: "none" } */;
  const { getByRole } = renderCardEditor(card);
  expect((getByRole("radio", { name: /alert only/i }) as HTMLInputElement).disabled).toBe(true);
});

test("weather cards offer no alert controls", () => {
  const card = /* weather card */;
  const { queryByRole } = renderCardEditor(card);
  expect(queryByRole("group", { name: /alert/i })).toBeNull();
});
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd companion/apps/deskmate && bun test && bun run check && bun run lint && bun run format:check && bun run build`
Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add companion/apps/deskmate
git commit -m "feat: add a card editor covering every kind, presence, and alert"
```

---

### Task 10: Filmstrip preview on real data

**Files:**
- Create: `companion/apps/deskmate/src/components/Filmstrip.tsx`
- Modify: `companion/apps/deskmate/src/components/DevicePreview.tsx`
- Modify: `companion/apps/deskmate/src/App.tsx`, `src/styles.css`
- Test: `companion/apps/deskmate/tests/components.test.tsx`

**Interfaces:**
- Consumes: `CardDataSnapshot` from Task 7, `loopSeconds`/`rotationCards` from Task 8.
- Produces: `Filmstrip({ config, selectedCardId, onSelect, onReorder })` and, in `configDraft.ts`, `cardFields(cardData: CardDataSnapshot[], cardId: string): Map<string, CardFieldValue>`.

- [ ] **Step 1: Write the failing test**

```tsx
test("the preview renders real field values and marks sample data", () => {
  const cardData: CardDataSnapshot[] = [
    {
      card_id: "upnext",
      fields: [{ key: "row0_title", value: { kind: "text", value: "Q3 Planning Sync" } }],
    },
  ];
  const { getByText, queryByText } = renderPreview({ cardId: "upnext", cardData });
  expect(getByText("Q3 Planning Sync")).toBeTruthy();
  expect(queryByText(/sample/i)).toBeNull();
});

test("a card with no data yet is clearly labelled as a sample", () => {
  const { getByText } = renderPreview({ cardId: "upnext", cardData: [] });
  expect(getByText(/sample/i)).toBeTruthy();
});

test("the filmstrip shows the loop length and only in-rotation cards", () => {
  const config = /* two in-rotation cards at 45s and 20s, one off card */;
  const { getByText, queryByText } = renderFilmstrip(config);
  expect(getByText(/1 min 5 s/)).toBeTruthy();
  expect(queryByText(/muted/i)).toBeNull();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion/apps/deskmate && bun test tests/components.test.tsx 2>&1 | tail -20`
Expected: FAIL — hardcoded "Design review" is rendered instead of the pushed value; `Filmstrip` does not exist.

- [ ] **Step 3: Rewrite DevicePreview to consume real values**

Delete the hardcoded faces at `DevicePreview.tsx:31-75`. Each face now reads from `cardFields(cardData, card.id)` and falls back to a clearly-marked sample only when the map is empty. Change the panel label at `DevicePreview.tsx:104` from `"Layout preview · not pixel-identical"` to `"Same data as your display · approximate pixels"`, and delete the `preview-dots` fieldset (`DevicePreview.tsx:119-137`) — the filmstrip replaces it.

- [ ] **Step 4: Build the Filmstrip**

Create `Filmstrip.tsx`: one segment per in-rotation card, `flex-grow` proportional to its resolved dwell, showing the card name and dwell. Include the computed loop length from `loopSeconds`, a play control that steps the selection through the rotation at real dwell timing, and drag-to-reorder that calls `onReorder`. Respect `prefers-reduced-motion` by disabling the automatic play stepping. Under `advance.kind === "manual"`, show the order without timings and hide the play control.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd companion/apps/deskmate && bun test && bun run check && bun run lint && bun run format:check && bun run build`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add companion/apps/deskmate
git commit -m "feat: preview cards from real field values with a rotation filmstrip"
```

---

### Task 11: Close the remaining settings defects

**Files:**
- Modify: `companion/apps/deskmate/src/lib/configDraft.ts`, `src/App.tsx`
- Test: `companion/apps/deskmate/tests/configDraft.test.ts`, `tests/components.test.tsx`

**Interfaces:**
- Produces: `firstRunSteps(config: AppConfig, connected: boolean): { label: string; done: boolean }[]` replacing the boolean `needsFirstRunGuidance`.

- [ ] **Step 1: Write the failing test**

```ts
test("first-run guidance clears without demanding specific card kinds", () => {
  // A user who wants only a clock and a weather card is fully set up.
  const config = addCard(addCard(base, "clock").config, "weather").config;
  const configured: AppConfig = {
    ...config,
    cards: config.cards.map((card) =>
      card.kind === "weather" ? { ...card, location: "Tbilisi" } : card,
    ),
  };
  expect(firstRunSteps(configured, true).every((step) => step.done)).toBe(true);
});

test("guidance still flags a card that is missing its source", () => {
  const config = addCard(base, "calendar").config; // empty ICS url
  expect(firstRunSteps(config, true).some((step) => !step.done)).toBe(true);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion/apps/deskmate && bun test tests/configDraft.test.ts 2>&1 | tail -20`
Expected: FAIL — `firstRunSteps` is not exported; `needsFirstRunGuidance` demands a pomodoro and a calendar.

- [ ] **Step 3: Write minimal implementation**

Replace `needsFirstRunGuidance` (`configDraft.ts:220-223`) with steps derived from what the user actually configured:

```ts
export function firstRunSteps(
  config: AppConfig,
  connected: boolean,
): { label: string; done: boolean }[] {
  const needsSource = config.cards.some((card) => {
    if (card.kind === "calendar") {
      return card.source.value.trim() === "";
    }
    if (card.kind === "weather") {
      return card.location.trim() === "";
    }
    if (card.kind === "json-feed" || card.kind === "rss") {
      return card.url.trim() === "";
    }
    return false;
  });

  return [
    { label: "Connect the display with USB", done: connected },
    { label: "Check your timezone", done: config.preferences.timezone.trim() !== "" },
    { label: "Finish setting up each card", done: !needsSource },
  ];
}
```

Render the banner in `App.tsx` only while at least one step is undone.

- [ ] **Step 4: Sweep the remaining defects**

- Confirm the add-card cap is `8` everywhere (Task 8 Step 4) and that no `16` remains: `grep -rn "16" companion/apps/deskmate/src/components/`.
- Confirm no component renders a card ID: `grep -rn "\.id}" companion/apps/deskmate/src/components/`.
- Confirm no `widgetIndex` prop survives: `grep -rn "widgetIndex\|primaryScreenWidgetId\|screens" companion/apps/deskmate/src/`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd companion/apps/deskmate && bun test && bun run check && bun run lint && bun run format:check && bun run build`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add companion/apps/deskmate
git commit -m "fix: correct card limits, first-run guidance, and identifier leakage"
```

---

### Task 12: Update the frozen contract and milestone documents

Per CLAUDE.md, code and documentation must not diverge; this task lands with the work, not after it.

**Files:**
- Rename + rewrite: `docs/config/v2.md` → `docs/config/v3.md`
- Modify: `docs/superpowers/specs/2026-08-03-deskmate-design.md` (sections 4, 5, decision log)
- Modify: `docs/superpowers/plans/2026-08-05-deskmate-m4-v1-completion.md`
- Modify: `CLAUDE.md`
- Modify: `docs/protocol/v1.md` (clarifying note only)

- [ ] **Step 1: Rewrite the config contract**

```bash
git mv docs/config/v2.md docs/config/v3.md
```

Rewrite it to describe schema v3: the `cards[]` array, `presence`, `alert`, `carousel.advance`, the card/asset ID namespaces, every bound from the Global Constraints, the v0/v1/v2 migration, and the statement that compilation lowers cards into the unchanged `ApplyConfig` pair with `SizeClass::Full` pinned and screen IDs equal to card IDs. Remove all size-class, screen, and dashboard text.

- [ ] **Step 2: Amend the design spec**

In `docs/superpowers/specs/2026-08-03-deskmate-design.md`:
- Section 4: replace the single/dashboard screen model and the three size classes with the card model, timed rotation, and alerts. Keep the clean-canvas and mounting-orientation paragraphs.
- Section 5: remove per-size-class layouts from the template list.
- Decision log: add the card-model rows from the new spec's section 12, and mark the "Screen model" row superseded, citing `docs/superpowers/specs/2026-08-06-deskmate-card-model-design.md`.

- [ ] **Step 3: Amend the M4 plan**

In `docs/superpowers/plans/2026-08-05-deskmate-m4-v1-completion.md`:
- Add the card-model task before Task 3 and mark it complete, with implementation evidence recording the exact observed test counts.
- Task 3: delete the full/standard/tile compatibility bullets; each template gets one 448x368 layout.
- Task 4: replace the tile-dashboard task with timed rotation and alerts, noting it is delivered by this plan and requires no firmware layout work.
- Task 8: add the single card list, filmstrip, six addable kinds, and card-data IPC.
- Task 10: replace grid checks in the exit gate with rotation and alert checks at 90° and 270°.

- [ ] **Step 4: Update CLAUDE.md**

Update the current-state bullets: schema v3 and `docs/config/v3.md` are the frozen contract; M4 Task 4 is now timed rotation and alerts rather than tile dashboards. Replace the sentence *"All current widgets use the clean 448x368 canvas. The v1 `standard` size value remains wire/config compatibility only and renders like `full`; do not restore a status strip."* with a statement that there is exactly one canvas and one layout per template, that size classes no longer exist in config, that `SizeClass::Full` is pinned on the wire, and that dashboards and status strips must not be reintroduced.

- [ ] **Step 5: Add the protocol clarifying note**

In `docs/protocol/v1.md`, beside the size-class table (lines 185-187), note that the host pins `Full` from schema v3 onward, that `Standard` and `Tile` remain reserved for decode compatibility only, and that the firmware must still reject an unsupported class from a malformed host.

- [ ] **Step 6: Full verification from a clean checkout state**

```bash
cd companion && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cd apps/deskmate && bun test && bun run check && bun run lint && bun run format:check && bun run build
make -C ../../../firmware/host_tests clean test
grep -rn "auto_advance_seconds\|WidgetSize\|ScreenLayout\|dashboard" docs/ companion/crates companion/apps/deskmate/src | grep -v "docs/superpowers/plans/2026-08-0[45]"
```

Expected: all suites pass; the final `grep` returns only intentional historical references in superseded plan documents.

- [ ] **Step 7: Commit**

```bash
git add docs CLAUDE.md
git commit -m "docs: freeze config v3 and realign the milestone documents"
```

---

## Physical verification (required before marking the milestone task complete)

This plan changes no firmware, but it changes what the host sends and when. Per CLAUDE.md, record observed results in `docs/hardware/board-notes.md` — never claim a check that was not run on the board.

- [ ] Flash the current firmware unchanged. Confirm `StatusResponse` reports protocol 1 and the same capability bits as before this work.
- [ ] Load a migrated M3/v2 configuration. Confirm card order matches the previous screen order and the display shows the same first screen.
- [ ] With timed advance at distinct per-card dwells, confirm each card holds for its configured time and the loop wraps.
- [ ] Swipe during a dwell. Confirm the dwell restarts rather than advancing early, and no duplicate `ActivateScreen` is observed.
- [ ] Fire a pomodoro alert with `until-dismissed`; confirm it holds until tapped and restores the correct carousel screen.
- [ ] Fire a calendar alert with a bounded hold; confirm it self-dismisses at the hold and fires exactly once per event.
- [ ] Set every card to alert-only or off in the app; confirm the save is rejected with the rotation-rule error and the device keeps its previous configuration.
- [ ] Repeat the rotation and alert checks at both 90° and 270°.
- [ ] Unplug and replug mid-rotation; confirm replay restores the active card and pending alert state without duplicates.

## Self-Review

**Spec coverage** — every section of `2026-08-06-deskmate-card-model-design.md` maps to a task:

| Spec section | Task |
|---|---|
| 3 The card / 3.1 Presence / 3.2 Alerts | 1, 2 |
| 3.3 Carousel | 1, 2, 5 |
| 4 Config schema v3 | 2 |
| 5 Migration | 3 |
| 6 Wire and firmware impact | 2 (Step 7 proves it) |
| 7.1 One list | 8 |
| 7.2 Preview is the feedback loop | 7, 10 |
| 7.3 Filmstrip | 10 |
| 7.4 Defects to fix | 8 (16→8, six kinds), 9 (ID-keyed issues), 11 (first-run, ID leakage) |
| 7.5 Gesture disclosure | 9 |
| 8 Validation rules 1–7 | 2 (rules 1–6), 2 Step 3.7 (rule 7) |
| 9 Testing | 3, 5, 6, 8, 9, 10 + physical checklist |
| 10 M4 plan impact | 12 |

**Placeholder scan** — the two component tests in Tasks 8 and 10 use `/* … */` for fixture construction rather than spelling out multi-card literals. That is deliberate: the exact literal depends on the render helper already present in `tests/components.test.tsx`, which the implementer will be reading. Every assertion is concrete. No step says "add error handling" or "write tests for the above".

**Type consistency** — checked across tasks: `CardPresence`/`CardAlert`/`AlertHold`/`CarouselAdvance` are defined in Task 1 and used unchanged in 2, 3, 5, 6; `CardSettings` accessors `presence()`/`alert()` are defined in Task 2 and used in 2, 5, 6; `CardDataSnapshot` is defined in Task 7 and consumed in 10; `issuesForCard` is defined in Task 9 and used only there; `loopSeconds`/`rotationCards` are defined in Task 8 and used in 10. `addCard` returns `{ config, cardId }` in both its definition (Task 8) and its uses (Tasks 8, 11).

**Confirmed prerequisite inside Task 6:** the calendar alert needs a machine-readable event start, and the provider does not emit one. `IcsCalendar::fields` (`companion/crates/providers/src/ics.rs:103-141`) produces only formatted display text plus a `stale` boolean. Task 6 Step 5 therefore adds a `next_start_unix_ms` integer field to the ICS provider, with fixture coverage including a recurring event, before wiring the trigger. This is the only file outside `app-core` and the frontend that this plan modifies.

**One deliberate scope note:** Task 2 is the largest task and cannot be split further without leaving a non-compiling commit — removing `widgets`/`screens` from `AppConfig` breaks `runtime.rs`, `store.rs`, and `commands.rs` simultaneously. Task 1 exists specifically to land the new value types under test first, so Task 2 is a mechanical substitution rather than a design exercise.
