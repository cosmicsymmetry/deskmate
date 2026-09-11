# Retire the device-rendered data cards, and stop offering to swap a plugin card's plugin

Branch: `refactor/retire-device-data-cards`, cut from `feat/picture-cards` at `3f42a49`.
Owner direction, 2026-09-10, verbatim in two parts:

> removing obsolete embedded cards that are rendered in device. The reason is that we can
> have way more flexible, for instance, weather card rendered on the server rather than on
> the device. And the device version, unlike Clock or Pomodoro timer, do not need faster
> refreshes. Generally our logic is to keep everything that doesn't need fast refreshes on
> the server. Following that logic we will delete ICS calendar, weather, RSS feed Cards
> entirely. Later on we'll recreate that functionality as server-side cards.

> while choosing a server rendered card and its settings you can have a plugin drop down
> menu where you can choose another server-side rendered card. I have no idea why you made
> that, I have no idea why any user in any case would want that, and we just need to get rid
> of that.

## Established facts — do not re-derive these

1. **The device has no card-kind code left.** Stage 3a retired the six hand-written C
   templates; the panel draws host-pushed scenes. `grep -rn 'weather\|calendar\|rss'
   firmware/main/` returns one default string `"Calendar"` in `template_fields.c` and one
   comment. **This work makes no firmware change**, so no OTA re-verification is owed and
   `PROTOCOL_CURRENT_CAPABILITIES` stays 2027.
2. **The wire is untouched.** Protocol stays v1. `TemplateKind` keeps all six values and
   `firmware/main/core/template_fields.c` keeps every registry entry: plugin and picture
   cards pin `TemplateKind::DigitalClock`, and the other five are used by
   `app-core/tests/scene_parity.rs`, `lvgl-sim`'s cases, and the operator route
   `POST /v1/devices/{id}/scene`. Removing them would delete the renderer's own evidence.
3. **`DisplayTemplate` and every `build_*_scene` builder stay whole**, for the same
   reason. After this change only `DigitalClock`, `AnalogClock` and `ProgressRing` are
   reachable *from a card*; `RowList`, `BigNumberLabel` and `IconBadgeText` remain
   reachable from the parity gate and the operator scene route. Leave them.
4. **A schema bump has three deployment boundaries** (server binary, `Deskmate.app`, then
   the save) — `docs/config/v7.md` says so, and v8 inherits that.
5. The owner's live `dev-0005` config and the Mac's own store both carry `weather` and
   `rss` cards today. The migration below is what keeps them loadable.

## Scope A — delete four card kinds

Delete `calendar`, `weather`, `json-feed` and `rss` **entirely**. `json-feed` is not in the
owner's list because they did not name every kind; it is the same thing — point a card at a
URL and map fields — and it is exactly what a plugin does better, server-side. It goes with
the other three.

Survivors: `clock`, `pomodoro` (device-local, fast refresh), `plugin`, `picture`.

### A1. Schema v8 and its migration

- `CURRENT_SCHEMA_VERSION` becomes `8`. Create `docs/config/v8.md` from v7 (v8 is
  **subtractive**, the first such bump — say so, and say what a document loses).
- Add `ConfigOrigin::MigratedV7`.
- **Every** migration path (v0–v3 legacy widgets, and the v4–v7 parse-as-current arm) drops
  cards of the four retired kinds, and drops every playlist entry naming one.
- A playlist left with no entries is removed, **except** the active playlist: if that one
  empties, seed it with the same `clock` card and single entry `AppConfig::default()`
  builds, so the migrated document validates and the panel still shows something. Add a
  test for the all-retired-cards document.
- v0–v3 documents must still migrate. `CalendarSource`, `WeatherUnits` and
  `JsonFieldMapping` must stop being part of the public `app_core` API; how you keep the
  legacy parsers compiling (private legacy types in `store.rs`, or variants that capture
  only what the drop needs) is your call — state which you chose.
- Fixtures: bump `full.json` to v8, copy the pre-bump `full.json` to `v7-roundtrip.json`
  first (the convention v3/v4/v5/v6 follow — do not destroy the only real v7 document),
  update `future-v7.json`'s test to the new unsupported version, and fix every fixture that
  is loaded with `serde_json::from_str::<AppConfig>` rather than through migration. The
  picture-cards plan records that eight such fixtures bypass migration entirely and cost 56
  test failures; expect the same class of breakage here.

### A2. Rust

- `app-core/src/config.rs` — remove the four `CardSettings` variants and their
  `strict_tagged_enum::CardSettingsInner` twins, their `compile`/`validate` arms,
  `ProviderKind::{Calendar,Weather,JsonFeed,Rss}`, `MAX_RSS_ITEMS`, `CalendarSource`,
  `WeatherUnits`, `JsonFieldMapping`.
- **Remove `CardAlert::BeforeEvent`.** Only a calendar card could carry it — validation
  says so in as many words — so with calendar gone it is unreachable. Remove the scheduler
  wake kind and the runtime machinery that exists **only** to re-evaluate it
  (`scheduler.rs`'s per-card alert-evaluation deadline, `runtime.rs`'s event-start map).
  **Do not touch the pomodoro alert path**: `AlertHold`, `CardAlert::OnTimerFinish`, hold
  bookkeeping and the takeover face all stay exactly as they are.
- Rename `MIN_CALENDAR_REFRESH_MINUTES`/`MAX_CALENDAR_REFRESH_MINUTES` to
  `MIN_CARD_REFRESH_MINUTES`/`MAX_CARD_REFRESH_MINUTES` — they bound plugin and picture
  cards now and the word "calendar" should stop naming anything in this product. Do not
  change their values.
- `app-core/src/runtime.rs` — `ProviderRequest` keeps its single `Plugin` variant (leave it
  an enum). Delete the `SystemProvider` enum, `system_provider()`, and the four legacy
  aliases `CalendarRefreshRequest`, `CalendarRefreshResult`, `CalendarRefresher`,
  `SystemCalendarRefresher` (rename their users to the `Provider*` names). Keep
  `SystemProviderRefresher` as the refresher that serves no request — after this change it
  returns its typed error for everything, which is exactly right for the hostless Mac
  runtime, whose plugin cards are skipped before a request is ever built. Rewrite its doc
  comment to say that rather than leaving a name that promises providers.
- `providers` crate — delete `ics.rs`, `weather.rs`, `rss.rs`, `json_feed.rs`. **Keep
  `http.rs`**: `config.rs` still needs `validate_http_url` for `WidgetTapAction::OpenUrl`.
  Keep `lib.rs`'s `Provider`/`ProviderSnapshot`/`LastGood`/`ProviderError` — the plugin
  provider is built on them.
- `server/src/plugin_refresher.rs` — drop the non-plugin fallback and its tests.
- `apps/deskmate/src-tauri/src/commands.rs` — remove the retired arms; regenerate the
  `types.contract.ts` fixture the test at the bottom of that file checks.
- `deskmate-cli/src/m2.rs` builds calendar cards for the M2 demo. That demo is for a
  milestone whose card kinds no longer exist. **Delete the calendar demo path** rather than
  faking it; if that empties the file or a subcommand, remove the subcommand and say so.

### A3. Frontend (`companion/apps/deskmate/`)

TypeScript's exhaustive switches will find most of this. Expect at least:
`lib/types.ts` (`CardSettings`, `AddableCardKind`), `lib/types.contract.ts`,
`lib/configDraft.ts` (`copyConfig`, `cardName`, `cardKindName`, `addCard`),
`lib/providers.ts`, `components/CardList.tsx` (`addableKinds`, `tileValue`),
`components/CardEditor.tsx`, and the four test files under `tests/`.

The built-in group in the add menu becomes just Digital clock and Pomodoro. That is
correct, not a regression — do not pad it.

## Scope B — a plugin card's plugin is not editable

In `components/CardEditor.tsx`, the `card.kind === "plugin"` block renders a `<select>` of
every plugin in the server's catalog. Replace it with a **read-only statement of which
plugin this card is**. A card's plugin is its identity, chosen when the card was added; a
control that silently turns an Air-quality card into an Agenda card is a trap, and the
owner has rejected it.

Preserve, in the read-only form, every honesty property the current block's comments
argue for — they were not wrong about the states, only about offering a choice:

- catalog present and the id is in it → the plugin's `display_name` and `version`;
- catalog present and the id is **not** in it → the id plus `Not installed on the server`;
- catalog `null` → the id plus the existing `catalogUnavailableReason` wording, because
  "not installed" is a claim the app cannot back up when it could not check;
- the machine id stays visible in the small line — it is the card's only identity when the
  catalog is unreachable;
- `FieldIssues` for `plugin_id` stays.

Use the existing `field`/`small` markup so it sits in the form like the other rows; it must
not read as a disabled control. Update `tests/components.test.tsx` accordingly, including a
test that the editor offers **no** way to change `plugin_id`.

## Explicit non-goals — do not do these

- **No firmware change.** Not `template_fields.c`, not the wire enum, nothing under
  `firmware/`.
- **No protocol change**, no capability bit, no new message type.
- **Do not remove the plugin card's "Refresh every" control.** It looks redundant beside
  "The server fetches this plugin every N minutes", and it partly is, but the runtime
  builds that card's provider deadline from it (`runtime.rs`, the `CardSettings::Plugin`
  arm of the config-apply loop) — a card left on `RefreshPolicy::Manual` gets no deadline
  and never refreshes. Removing it safely is separate work. Leave it and mention it in your
  report.
- **Do not touch the picture card's "Picture source" select.** Same shape as Scope B, not
  the same call; the owner has not seen it yet.
- **Do not delete any `build_*_scene` builder, `DisplayTemplate` variant, `lvgl-sim` case,
  golden frame, or `scene_parity.rs` row.** Titles like `"Calendar"` inside those fixtures
  are display strings, not card kinds — leave them.
- **Do not commit.** Leave every change in the working tree.

## Docs to update in the same change

- `docs/config/v8.md` (new, from v7): the four kinds are gone, `CardAlert::BeforeEvent` is
  gone, what migration does to a document that has them, and the three deployment
  boundaries.
- `CLAUDE.md`: one bullet, in the house style of the ones around it — what was deleted and
  why, that schema is v8 and firmware/protocol are untouched, and that a v≤7 document
  loses those cards on load.
- Mark `docs/config/v7.md` superseded in-file, the way v6 marks itself.

## Gates — run all of them, report exact counts

```sh
cd companion
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --no-fail-fast
cargo test --workspace --doc
cd apps/deskmate && /Users/rodion/.bun/bin/bun test && /Users/rodion/.bun/bin/bun run check
```

Your sandbox cannot bind loopback sockets, so `server` integration tests may fail in a way
you cannot tell from a real failure. Report which ones failed and with what error; the
orchestrator re-runs them.

## Report

Files changed, test counts before/after, every decision this brief did not settle, and
anything you found that contradicts the "established facts" above.
