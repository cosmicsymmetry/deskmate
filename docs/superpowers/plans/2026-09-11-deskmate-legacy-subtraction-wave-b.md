# Legacy Subtraction — Wave B: the card list is the loop (schema v10)

**Spec:** `docs/superpowers/specs/2026-09-11-deskmate-legacy-subtraction-design.md`

**Goal:** Remove `playlists[]` and `active_playlist_id` from the config document, so
the one loop the product has is the one list it stores.

**Architecture:** `cards[]` becomes ordered and IS the loop. Dwell moves onto the card;
`advance` moves to the document root. A subtractive v10 migration folds the active
playlist back into the card order and drops the rest.

## The two decisions the spec left open

**A card cannot exist outside the loop.** Nothing in the product can put one there:
`addCard` always enrols, and no action removes a card from the loop without deleting
it. The "not in loop" flag and its "Add to loop" button can only ever have described a
card migrated from a v4-v8 file whose library and playlist were separate lists. The
migration enrols such a card at the end of the loop, so nothing is lost, and the state
goes with the button, `card-tile--outside`, and `outsideCards`.

"Missing card" goes for a stronger reason: a playlist entry whose `card_id` does not
resolve is **unrepresentable** in v10. `removeEntry`'s only caller was that tile.

**`advance` is global, dwell is per card.** Advance is a mode for the whole loop --
timed or manual -- so it belongs at the root, where the pacing control in the ring's
head already reads as global. `dwell_seconds` moves from the playlist entry onto the
card, which is where the editor already edits it ("Stays on the panel for").

## Global constraints

- Protocol stays **v1**; `PROTOCOL_CURRENT_CAPABILITIES` stays **2027**. Nothing on the
  wire knows the word playlist, and this wave must not change that.
- **No diff under `firmware/main/`.**
- `MAX_PLAYLISTS` and `MAX_PLAYLIST_ENTRIES` (both 8) collapse into the existing
  `MAX_CARDS` (8), so no bound actually moves.
- Full gate set before handoff, each command separately (see CLAUDE.md).

## The v10 document

```json
{
  "schema_version": 10,
  "preferences": { "timezone": "Asia/Tbilisi", "autostart": true, "paused": false,
                   "orientation": "landscape" },
  "cards": [
    { "kind": "clock", "id": "clock", "title": "Desk", "show_seconds": true,
      "template": { "kind": "digital-clock" }, "tap_action": { "kind": "none" },
      "refresh": { "kind": "device-local" }, "alert": { "kind": "none" },
      "dwell_seconds": null }
  ],
  "image_sources": [],
  "assets": [],
  "advance": { "kind": "timed", "default_dwell_seconds": 30 },
  "updater": {}
}
```

`cards` is ordered and that order is the loop order. `dwell_seconds` is
`Option<u16>`; null means "use `advance`'s default".

## Migration (v4..v9 -> v10)

Extends the existing `drop_retired_cards_from_json` pass rather than growing a second
one, because it already operates on `Value` before the strict deserialize:

1. Strip retired card kinds and the playlist entries naming them (unchanged).
2. Take the active playlist. Emit its entries' cards first, in entry order, each
   carrying that entry's `dwell_seconds`.
3. Append every surviving card the active playlist did not name, in its original order,
   with `dwell_seconds: null`. **A card is never dropped.**
4. `advance` = the active playlist's `advance`, or `manual` if there is none.
5. Delete `playlists` and `active_playlist_id`.

**What this loses, deliberately:** the names of playlists, and any inactive playlist.
CLAUDE.md has said since the one-loop change that extra playlists in older files
round-trip unchanged; that stops being true here, which is the point of the wave. No
card is lost -- only the grouping the product cannot express.

## Tasks

### Task 1 — schema v10 in app-core

**Files:** `crates/app-core/src/config.rs`, `src/store.rs`, `tests/config.rs`,
`tests/store.rs`, fixtures, `docs/config/v10.md`.

- [ ] Failing test first: a v9 fixture with two playlists migrates to v10 with the
      active playlist's order preserved, the inactive one's cards appended, and
      `advance` carried over.
- [ ] `CardSettings`: add `dwell_seconds: Option<u16>` to all three variants, the
      `CardSettingsInner` shadow, `allowed_fields`, and an accessor.
- [ ] `AppConfig`: add `advance: CarouselAdvance`; delete `playlists`,
      `active_playlist_id`, `Playlist`, `PlaylistEntry`, `active_playlist()`,
      `MAX_PLAYLISTS`, `MAX_PLAYLIST_ENTRIES`, `DEFAULT_PLAYLIST_ID`,
      `DEFAULT_PLAYLIST_NAME`.
- [ ] `validate()`: drop every playlist rule; keep the "at least one card" rule, which
      now also guarantees the loop is non-empty.
- [ ] `compile()`: every compiled card becomes a screen -- the `active_card_ids` set
      and its lookup go.
- [ ] `CURRENT_SCHEMA_VERSION = 10`; migration arm widens to `4..=9`.
- [ ] Freeze `docs/config/v10.md` from v9.md; delete `docs/config/v4.md` only if the
      migration stops reading v4 (it does not -- keep v4..v9).
- [ ] Mutation-probe the migration: neutering the append-orphans step must fail a test.

### Task 2 — the runtime's rotation

**Files:** `crates/app-core/src/runtime/mod.rs`, `src/scheduler.rs`.

- [ ] `rotation_card_ids` becomes the card order itself.
- [ ] `current_dwell` reads the card's `dwell_seconds`, falling back to
      `config.advance`'s default.
- [ ] `advance_rotation` unchanged in behaviour; only its inputs move.

### Task 3 — server and fixtures

**Files:** `crates/server/src/store.rs`, `crates/server/tests/fixtures/*.json`.

- [ ] Repin every fixture to schema 10 with the new shape.
- [ ] Confirm no server source references a playlist.

### Task 4 — the window

**Files:** `apps/deskmate/src/lib/configDraft.ts`, `lib/types.ts`,
`lib/types.contract.ts`, `components/CardList.tsx`, `components/CardEditor.tsx`,
`components/LoopRing.tsx`, `dev/fixture.ts`, `dev/mockBackend.ts`, tests.

- [ ] `configDraft`: `addEntry`, `removeEntry`, `moveEntry` and the playlist helpers
      become card-list operations; `addCard` drops its second bound.
- [ ] `claimedIssues`: claim `cards[i].dwell_seconds` and `advance*`; the playlist arms
      go.
- [ ] `CardList`: delete `outsideCards`, the "not in loop" flag, "Add to loop", the
      "Missing card" tile and `card-tile--outside`.
- [ ] `CardEditor`: dwell edits the card's own field.
- [ ] `LoopRing` and the pacing control read `config.advance`.
- [ ] Regenerate the TypeScript contract fixture through its own sync test.

### Task 5 — documentation

- [ ] CLAUDE.md: the one-loop rule loses the "schema still carries playlists" caveat;
      current state says v10.
- [ ] Roadmap: Wave B complete.

## Exit

- [ ] Full gate set, each command separately, all exit 0.
- [ ] No diff under `firmware/main/`.
- [ ] `grep -rn "playlist" companion/crates companion/apps --include='*.rs'
      --include='*.ts' --include='*.tsx'` returns nothing outside migration code and
      `docs/`.
- [ ] `CURRENT_SCHEMA_VERSION` is 10 and `PROTOCOL_CURRENT_CAPABILITIES` is 2027.
