# One Loop — collapse the card library and playlists into the loop

> **STATUS — HISTORICAL IMPLEMENTATION RECORD.** This plan describes the system and
> paths at the time of its dated work; it is not current architecture or build guidance.

**Status:** approved by the owner on 2026-09-06 from a rendered mockup; implementing.
**Branch:** `feat/one-loop` (worktree `.worktrees/one-loop`), off `feat/v2-networked-device`.
**Scope:** companion app UI only. **Schema v6 is untouched, protocol v1 is untouched,
app-core and the server are untouched.** The document keeps exactly one playlist under
the hood (`AppConfig::default()` and every migration already produce exactly one); the UI
stops exposing playlists as a concept.

## Decision

The owner's finding: "first I need to add cards to the library and then from the library
I can add them to the playlist. This step is unnecessary. Why do we have these two
entities?" The evidence agreed — the owner's own config has one playlist, the named
"Workday/Evening" case exists only in fixtures, nothing on the wire knows the word
playlist, and getting Weather onto the panel took seven clicks across two panels and a
scroll. Three window topologies were judged against `DESIGN.md`, `PRODUCT.md` and the
code; the one below won unanimously.

**The tile grid is the loop.** The existing complication tiles keep their live values and
sit in loop order. Reordering happens on the grid. Adding a card is one slot at the end of
the grid that opens a menu (this is a second disclosure beside the settings sheet — the
owner asked for it explicitly, because a growing plugin registry must never reshape the
window; when the list needs search it becomes a picker window). The rail stays the face
and changes by one object: the pacing control. The editor sits under the grid. The
playlist panel is deleted.

## What changes (in this order)

### 1. `src/lib/configDraft.ts`

- `addCard(config, kind)` now **also enrols the new card at the end of the active
  playlist** in the same draft (`addEntry`). It is gated on both `MAX_CARDS` (8) and
  `MAX_PLAYLIST_ENTRIES` (8) — a legacy card outside the loop consumes a card slot but not
  an entry slot. Its doc comment ("playlist membership is an explicit, separate edit") is
  reversed by this decision; rewrite it.
- New `loopEntries(config)`: the active playlist's entries in order, each resolved to
  `{ index, entry, card: CardSettings | null }`. **Do not build the grid from
  `filmstripSegments`** — it silently drops entries whose `card_id` does not resolve, and
  such an entry (the contract fixture and the validation-failed persistence test both carry
  one) must render as a "Missing card" tile carrying its issue.
- `cardsOutsideLoop(config)` = cards in no active entry (today's `cardsOutsidePlaylist`
  against the active playlist).
- `firstRunSteps` becomes two steps: "Add a card", "Save your settings".
- `tapActionDescription` / the gesture note: "Swiping the screen moves through the loop."
- **Narrow `claimedIssues`.** It currently claims every `playlists*` path by prefix. After
  this change the only playlist-shaped issues a surface renders are
  `playlists[<active>].entries*` (grid tiles + editor dwell), `playlists[<active>].advance*`
  (pacing control) and `playlists[<active>].entries` (grid container). Everything else —
  `playlists[i].name`, `playlists[i].id`, inactive playlists, `active_playlist_id`,
  `playlists` — must fall through to `unclaimedIssues` and the leftover banner. A claimed
  issue nothing renders would break PRODUCT.md's invariant that no issue can silently block
  Save. Add a test: an issue at `playlists[1].name` is unclaimed.
- Delete the now-dead helpers and their tests: `addPlaylist`, `renamePlaylist`,
  `removePlaylist`, `setActivePlaylist`, `removeEntry`. Keep `addEntry` (Add to loop),
  `moveEntry`, `setEntryDwell`, `setPlaylistAdvance`, `removeCard`, `activePlaylist`,
  `filmstripSegments`, `loopSeconds`.
- Inactive playlists in existing documents round-trip untouched (`copyConfig` already
  does this). Do not delete them on save.

### 2. `src/components/LoopRing.tsx` (the rail)

- Head: the `tile-label` heading reads **"The loop"** (never the playlist name). The
  "Timed loop" / "Manual order" label is replaced by a **pacing control**: two
  pressed-state buttons `Timed` / `Manual` (`aria-pressed`, `setPlaylistAdvance`) and,
  under Timed, a default-dwell number input (`5..3600`, label sr-only "Default dwell in
  seconds", `aria-invalid` from `playlists[<active>].advance.default_dwell_seconds`
  issues). The transport ("Play the loop") stays where it is. Everything must still fit
  the head row at 1060×740 — measured ~372 px of 446.
- Legend: **displays and selects only.** Remove drag, the arrow buttons and the ⌥↑/↓
  handler; keep `is-live`, the swatch, the resolved dwell under Timed, and selection.
- Empty copy: "No cards yet." — no playlist wording anywhere in this component.

### 3. `src/components/CardList.tsx` → the loop grid

- Heading `h2` **"The loop"**, right side: `<span class="keyboard-hint">Drag to reorder
  · ⌥ ← →</span>` and the count badge `N/8` (N = `cards.length`, cap `MAX_CARDS`).
- Tiles from `loopEntries` in entry order, then `cardsOutsideLoop`. A missing-card entry
  renders a tile titled "Missing card" with its `playlists[<active>].entries[j].card_id`
  issue (`has-issue`), and a Remove that drops the entry.
- **Reorder on the grid**, all through `moveEntry` against the active playlist, resolving
  the entry index at call time (never a stale index): HTML5 drag as in today's
  `LoopRing`/`PlaylistPanel`; hover-revealed "earlier" / "later" round buttons at the
  tile's bottom-right (`aria-label` "Move <template — title> earlier|later", disabled at
  the ends); keyboard ⌥← / ⌥↑ earlier and ⌥→ / ⌥↓ later on a focused tile.
- Outside-the-loop tiles: flags `not in loop` and, when the card is alert-capable,
  `alerts` (the existing `flag--alert`); a `text-button` **"Add to loop"** (`addEntry`,
  `aria-label` "Add <template — title> to the loop"). The `unused` flag is deleted.
- **Add slot** as the last grid item: a dashed tile with the plus and "Add a card / Built
  in, or a plugin", `aria-haspopup="menu"` + `aria-expanded`. It opens a `role="menu"`
  under the slot: group "Built in" (the six kinds with the existing `addableKinds`
  descriptions) and group "Plugins on the server" rendered from a `pluginKinds` prop —
  **App passes `[]` today** (the app has no plugin registry source; omit the group when
  empty; do not fabricate one). Menu: `menuitem`s, ↑/↓ moves, Enter/Space picks, Escape
  closes and returns focus to the slot, outside click closes, `max-height` with scroll.
  At capacity the slot is disabled and `aria-describedby` names which limit bit ("8 cards"
  or "8 in the loop").
- Selecting a menu item = `addCard(kind)` → the new card is selected and its editor
  opens, exactly as today's kind buttons do.
- Remove on a tile stays the hover X → `removeCard` (disabled when it is the only card).
- Delete the `card-add` fieldset.

### 4. `src/components/CardEditor.tsx`

- New field for cards **in the loop**, under Timed advance only: label **"Stays on the
  panel for"**, number input in seconds, blank = inherit, placeholder "<default> s, the
  loop's default", `setEntryDwell(active, entryIndex, value)`, `aria-invalid` from
  `playlists[<active>].entries[j].dwell_seconds`. Cards outside the loop do not get it.
- Gesture note copy: "Swiping the screen moves through the loop."

### 5. `src/App.tsx`, `src/components/PlaylistPanel.tsx`, `src/styles.css`

- Delete `PlaylistPanel.tsx` and every `.playlist-*`, `.advance-option`,
  `.entry-dwell-field`, `.advance-dwell-field` and `.card-add` / `.add-card` rule.
- App composes the grid, the ring and the editor with the new props; the first-run
  checklist has two steps; notices unchanged.
- New CSS, tokens only (the approved mockup is the reference: `.loop__pacing`,
  `.loop__pace`, `.loop__dwell`, `.panel-heading__right`, `.card-tile__moves`,
  `.card-tile--add`, `.card-tile__add`, `.menu`, `.menu__group`, `.menu__label`,
  `.menu__item`, `.card-tile__join`, the outside tile's wrapping flags). Hover-revealed
  controls become permanently visible at the existing 430 px breakpoint, like the X.

### 6. Tests (`tests/components.test.tsx`, `tests/configDraft.test.ts`)

- Keep and re-point the two CLAUDE.md regression tests: "a card is called the same thing
  everywhere, and that is its template" (drop the PlaylistPanel render; assert the tile,
  the legend row and the editor heading) and "an untitled card is not labelled with its
  template twice". The `template — title` naming rule now covers the tile move buttons,
  the tile Remove, "Add to loop" and the dwell field.
- Replace the PlaylistPanel tests with: the grid renders in entry order then outsiders;
  earlier/later buttons and ⌥ arrows call `moveEntry` with the right indexes; the add
  slot opens the menu, Escape closes it and restores focus, picking a kind enrols the card
  and selects it; the outside tile shows `not in loop` (+ `alerts` for a pomodoro) and
  "Add to loop" enrols it; the missing-card entry renders as a tile with its issue; the
  dwell field writes the entry; the pacing control writes `advance`; `claimedIssues` no
  longer claims `playlists[1].name`; at capacity the slot is disabled and names the limit.
- `configDraft.test.ts`: `addCard` enrols; `firstRunSteps` is two steps; dead-helper
  tests deleted.
- `useAppState.test.ts` is unchanged (the fixture legitimately carries two playlists).

## Not in scope

- Schema v7 (removing `playlists[]` from the model) — a later spec; lossy for
  multi-playlist files and needs the server redeployed in lockstep.
- A plugin registry source for the app, and the picker window for a long registry.
- `DESIGN.md` / `PRODUCT.md` / CLAUDE.md amendments are done by the orchestrator after
  review, in the same branch.

## Verification

From `companion/apps/deskmate`: `bun test`, `bun run check`, `bun run lint`,
`bun run format:check`. The IPC contract (`types.contract.ts`) must not change. The dev
harness (`VITE_DESKMATE_MOCK=1 bun run dev`, `?scenario=default|firstrun|empty|invalid`)
must render every scenario; its fixture's `rss` card lives only in the inactive playlist
and must show as "not in loop".
