import { describe, expect, test } from "bun:test";

import {
  activePlaylist,
  addCard,
  addEntry,
  addPlaylist,
  cardFields,
  cardMoveFromKey,
  cardsOutsidePlaylist,
  cardsContainerIssues,
  filmstripAdvance,
  filmstripDeadline,
  filmstripSegments,
  firstRunSteps,
  firstSelectableCard,
  formatDuration,
  issuesForCard,
  issuesForField,
  issuesForPath,
  libraryCards,
  loopSeconds,
  moveCard,
  moveEntry,
  nextFilmstripCardId,
  numberValue,
  playlistEntries,
  removeCard,
  removeEntry,
  removePlaylist,
  renamePlaylist,
  setActivePlaylist,
  setEntryDwell,
  setPlaylistAdvance,
  tapActionDescription,
  withAlert,
} from "../src/lib/configDraft";
import type { AppConfig, CardDataSnapshot, CardSettings, ValidationIssue } from "../src/lib/types";
import { ipcContractFixtures } from "../src/lib/types.contract";

function initialConfig(): AppConfig {
  return {
    schema_version: 5,
    preferences: {
      timezone: "UTC",
      autostart: false,
      paused: false,
      orientation: "landscape",
    },
    cards: [
      {
        kind: "clock",
        id: "clock",
        title: "Desk",
        show_seconds: true,
        template: { kind: "digital-clock" },
        tap_action: { kind: "none" },
        refresh: { kind: "device-local" },
        alert: { kind: "none" },
      },
    ],
    assets: [],
    playlists: [
      {
        id: "workday",
        name: "Workday",
        advance: { kind: "manual" },
        entries: [{ card_id: "clock", dwell_seconds: null }],
      },
    ],
    active_playlist_id: "workday",
    updater: { channel: "stable", checks: "notify" },
  };
}

function cardsConfig(ids: string[]): AppConfig {
  const base = initialConfig();
  return {
    ...base,
    cards: ids.map((id) => ({ ...base.cards[0], id })),
  };
}

describe("configuration draft helpers", () => {
  test("adds cards with stable unique card IDs", () => {
    const withSecondClock = addCard(initialConfig(), "clock");
    expect(withSecondClock.cardId).toBe("clock-2");

    const withPomodoro = addCard(withSecondClock.config, "pomodoro");
    const complete = addCard(withPomodoro.config, "calendar");
    expect(complete.config.cards.map((card) => card.kind)).toEqual([
      "clock",
      "clock",
      "pomodoro",
      "calendar",
    ]);
    expect(new Set(complete.config.cards.map((card) => card.id)).size).toBe(4);
  });

  test("first-run guidance follows add a card, add it to a playlist, then save without demanding a card kind", () => {
    const empty: AppConfig = {
      ...initialConfig(),
      cards: [],
      playlists: [{ ...initialConfig().playlists[0], entries: [] }],
    };
    expect(firstRunSteps(empty, false).map((step) => step.done)).toEqual([false, false, false]);

    const withWeather = addCard(empty, "weather").config;
    expect(firstRunSteps(withWeather, false).map((step) => step.done)).toEqual([
      true,
      false,
      false,
    ]);

    const configured = addEntry(withWeather, "workday", "weather");
    expect(firstRunSteps(configured, false).map((step) => step.done)).toEqual([true, true, false]);
    expect(firstRunSteps(configured, true).every((step) => step.done)).toBe(true);
  });

  test("adding a card appends it without creating a screen", () => {
    const { config, cardId } = addCard(initialConfig(), "weather");
    expect(config.cards.map((card) => card.kind)).toContain("weather");
    expect(cardId).toBe("weather");
    expect("screens" in config).toBe(false);
  });

  test("adding every kind produces a reachable, addable card", () => {
    const kinds = ["clock", "pomodoro", "calendar", "weather", "json-feed", "rss"] as const;
    let config: AppConfig = { ...initialConfig(), cards: [] };
    for (const kind of kinds) {
      config = addCard(config, kind).config;
    }
    expect(config.cards.map((card) => card.kind)).toEqual([...kinds]);
  });

  // Regression test for the final-review finding: `addCard` defaulted weather and
  // json-feed to `big-number-label`, a template `validate()` accepted but
  // `wire_config()` (in `config.rs`) could not yet lower, so a freshly-added card of
  // either kind validated cleanly yet always failed to save. `wire_config()` now lowers
  // all six templates (see `companion/crates/app-core/src/config.rs`), so weather and
  // json-feed default to the templates their field composition was actually designed
  // for — `icon-badge-text` and `big-number-label` — instead of the `row-list`
  // placeholder that rendered a title over empty rows. See
  // `companion/crates/app-core/tests/config.rs`'s
  // `every_freshly_added_card_kind_validates_and_compiles` for the Rust-side proof that
  // every one of these defaults both validates AND compiles.
  test("every freshly-added card kind defaults to a template the wire actually implements", () => {
    const compilableTemplates = new Set([
      "digital-clock",
      "analog-clock",
      "progress-ring",
      "row-list",
      "big-number-label",
      "icon-badge-text",
    ]);
    const expectedTemplateKind: Record<string, string> = {
      clock: "digital-clock",
      pomodoro: "progress-ring",
      calendar: "row-list",
      weather: "icon-badge-text",
      "json-feed": "big-number-label",
      rss: "row-list",
    };
    const kinds = ["clock", "pomodoro", "calendar", "weather", "json-feed", "rss"] as const;
    let config: AppConfig = { ...initialConfig(), cards: [] };
    for (const kind of kinds) {
      const { config: next, cardId } = addCard(config, kind);
      config = next;
      const card = config.cards.find((candidate) => candidate.id === cardId);
      if (!card) {
        throw new Error(`addCard did not append the ${kind} card`);
      }
      expect(compilableTemplates.has(card.template.kind)).toBe(true);
      expect(card.template.kind).toBe(expectedTemplateKind[kind]);
    }
  });

  test("addCard deep-copies the existing cards, not just the appended one", () => {
    const config = initialConfig();
    const original = config.cards[0];
    const { config: next } = addCard(config, "pomodoro");

    // The pre-existing clock card in the returned draft must be a distinct object from
    // the source config's — otherwise mutating one through the draft would alias back
    // onto the live snapshot `config` was copied from.
    expect(next.cards[0]).not.toBe(original);
    expect(next.cards[0]).toEqual(original);
  });

  test("resolves the active playlist and returns null when its id does not resolve", () => {
    const config = initialConfig();
    expect(activePlaylist(config)?.id).toBe("workday");
    expect(activePlaylist({ ...config, active_playlist_id: "missing" })).toBeNull();
  });

  test("resolves playlist entries in entry order and omits missing card references", () => {
    const withTimer = addCard(initialConfig(), "pomodoro").config;
    const config: AppConfig = {
      ...withTimer,
      playlists: [
        {
          ...withTimer.playlists[0],
          entries: [
            { card_id: "pomodoro", dwell_seconds: null },
            { card_id: "missing", dwell_seconds: null },
            { card_id: "clock", dwell_seconds: null },
          ],
        },
      ],
    };
    expect(playlistEntries(config, "workday").map((card) => card.id)).toEqual([
      "pomodoro",
      "clock",
    ]);
    expect(playlistEntries(config, "missing")).toEqual([]);
  });

  test("returns every library card and cards outside one playlist", () => {
    const config = addCard(addCard(initialConfig(), "pomodoro").config, "weather").config;
    expect(libraryCards(config).map((card) => card.id)).toEqual(["clock", "pomodoro", "weather"]);
    expect(cardsOutsidePlaylist(config, "workday").map((card) => card.id)).toEqual([
      "pomodoro",
      "weather",
    ]);
  });

  test("adds playlists with slugified, unique ids", () => {
    const once = addPlaylist(initialConfig(), "  Deep Focus!  ");
    const twice = addPlaylist(once, "Deep Focus");
    expect(once.playlists.at(-1)).toEqual({
      id: "deep-focus",
      name: "Deep Focus!",
      advance: { kind: "manual" },
      entries: [],
    });
    expect(twice.playlists.at(-1)?.id).toBe("deep-focus-2");
  });

  test("renames a playlist without changing its stable id", () => {
    const next = renamePlaylist(initialConfig(), "workday", "  Work hours  ");
    expect(next.playlists[0].id).toBe("workday");
    expect(next.playlists[0].name).toBe("Work hours");
  });

  test("removing the active playlist activates the first remaining playlist and refuses the last", () => {
    const withSecond = addPlaylist(initialConfig(), "Evening");
    const next = removePlaylist(withSecond, "workday");
    expect(next.playlists.map((playlist) => playlist.id)).toEqual(["evening"]);
    expect(next.active_playlist_id).toBe("evening");
    expect(removePlaylist(next, "evening")).toBe(next);
  });

  test("sets the active playlist only when it exists", () => {
    const config = addPlaylist(initialConfig(), "Evening");
    expect(setActivePlaylist(config, "evening").active_playlist_id).toBe("evening");
    expect(setActivePlaylist(config, "missing")).toBe(config);
  });

  test("adds a playlist entry but refuses duplicates and a ninth entry", () => {
    const withTimer = addCard(initialConfig(), "pomodoro").config;
    const added = addEntry(withTimer, "workday", "pomodoro");
    expect(added.playlists[0].entries.map((entry) => entry.card_id)).toEqual(["clock", "pomodoro"]);
    expect(addEntry(added, "workday", "pomodoro")).toBe(added);

    const full = cardsConfig(["a", "b", "c", "d", "e", "f", "g", "h", "i"]);
    const fullPlaylist: AppConfig = {
      ...full,
      playlists: [
        {
          ...full.playlists[0],
          entries: full.cards
            .slice(0, 8)
            .map((card) => ({ card_id: card.id, dwell_seconds: null })),
        },
      ],
    };
    expect(addEntry(fullPlaylist, "workday", "i")).toBe(fullPlaylist);
  });

  test("removes a playlist entry by index and ignores an out-of-range index", () => {
    const config = addEntry(addCard(initialConfig(), "pomodoro").config, "workday", "pomodoro");
    const next = removeEntry(config, "workday", 0);
    expect(next.playlists[0].entries.map((entry) => entry.card_id)).toEqual(["pomodoro"]);
    expect(removeEntry(next, "workday", 4)).toBe(next);
  });

  test("moves playlist entries with clamped targets and index-safe source boundaries", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = addCard(config, "weather").config;
    config = addEntry(addEntry(config, "workday", "pomodoro"), "workday", "weather");

    const toEnd = moveEntry(config, "workday", 0, 99);
    expect(toEnd.playlists[0].entries.map((entry) => entry.card_id)).toEqual([
      "pomodoro",
      "weather",
      "clock",
    ]);
    const toStart = moveEntry(toEnd, "workday", 2, -99);
    expect(toStart.playlists[0].entries.map((entry) => entry.card_id)).toEqual([
      "clock",
      "pomodoro",
      "weather",
    ]);
    expect(moveEntry(config, "workday", -1, 1)).toBe(config);
    expect(moveEntry(config, "workday", 3, 1)).toBe(config);
  });

  test("sets an entry dwell by index without disturbing the other entries", () => {
    const config = addEntry(addCard(initialConfig(), "pomodoro").config, "workday", "pomodoro");
    const next = setEntryDwell(config, "workday", 1, 45);
    expect(next.playlists[0].entries).toEqual([
      { card_id: "clock", dwell_seconds: null },
      { card_id: "pomodoro", dwell_seconds: 45 },
    ]);
    expect(setEntryDwell(next, "workday", 5, null)).toBe(next);
  });

  test("sets one playlist's advance mode", () => {
    const config = addPlaylist(initialConfig(), "Evening");
    const next = setPlaylistAdvance(config, "evening", {
      kind: "timed",
      default_dwell_seconds: 30,
    });
    expect(next.playlists[0].advance).toEqual({ kind: "manual" });
    expect(next.playlists[1].advance).toEqual({
      kind: "timed",
      default_dwell_seconds: 30,
    });
  });

  test("loop length sums playlist entry dwell with the timed default and is null for manual", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = addEntry(config, "workday", "pomodoro");
    config = setEntryDwell(config, "workday", 1, 45);
    config = setPlaylistAdvance(config, "workday", {
      kind: "timed",
      default_dwell_seconds: 20,
    });
    expect(loopSeconds(config, "workday")).toBe(65);
    expect(
      loopSeconds(setPlaylistAdvance(config, "workday", { kind: "manual" }), "workday"),
    ).toBeNull();
    expect(loopSeconds(config, "missing")).toBeNull();
  });

  test("removing a card strips its entries from every playlist", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = addEntry(config, "workday", "pomodoro");
    config = addPlaylist(config, "Evening");
    config = addEntry(config, "evening", "pomodoro");
    const next = removeCard(config, "pomodoro");
    expect(next.cards.map((card) => card.id)).toEqual(["clock"]);
    expect(next.playlists.map((playlist) => playlist.entries)).toEqual([
      [{ card_id: "clock", dwell_seconds: null }],
      [],
    ]);
    expect(firstSelectableCard(next)).toBe("clock");
  });

  test("cardFields reads a card's published fields, keyed by name", () => {
    const cardData: CardDataSnapshot[] = [
      {
        card_id: "upnext",
        fields: [
          { key: "row0_title", value: { kind: "text", value: "Q3 Planning Sync" } },
          { key: "stale", value: { kind: "boolean", value: false } },
        ],
      },
    ];
    const fields = cardFields(cardData, "upnext");
    expect(fields.get("row0_title")).toEqual({ kind: "text", value: "Q3 Planning Sync" });
    expect(fields.get("stale")).toEqual({ kind: "boolean", value: false });
    expect(fields.get("row1_title")).toBeUndefined();
  });

  test("cardFields returns an empty map for a card with no published snapshot yet", () => {
    expect(cardFields([], "upnext").size).toBe(0);
    const cardData: CardDataSnapshot[] = [{ card_id: "some-other-card", fields: [] }];
    expect(cardFields(cardData, "upnext").size).toBe(0);
  });

  test("filmstripSegments proportions active-playlist entries by resolved dwell and excludes library-only cards", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = addCard(config, "weather").config;
    config = addEntry(config, "workday", "pomodoro");
    config = setEntryDwell(config, "workday", 0, 45);
    config = setPlaylistAdvance(config, "workday", {
      kind: "timed",
      default_dwell_seconds: 20,
    });
    const segments = filmstripSegments(config);
    expect(segments.map((segment) => segment.cardId)).toEqual(["clock", "pomodoro"]);
    expect(segments[0].dwellSeconds).toBe(45);
    expect(segments[1].dwellSeconds).toBe(20);
    // 45 of 65 total seconds, and 20 of 65 — proportional to dwell, not count.
    expect(segments[0].widthPercent).toBeCloseTo((45 / 65) * 100, 5);
    expect(segments[1].widthPercent).toBeCloseTo((20 / 65) * 100, 5);
    expect(segments[0].offsetPercent).toBe(0);
    expect(segments[1].offsetPercent).toBeCloseTo((45 / 65) * 100, 5);
  });

  test("filmstripSegments gives every card an equal share under manual advance, where there is no dwell to encode", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = addEntry(config, "workday", "pomodoro");
    config = setEntryDwell(config, "workday", 0, 45);
    const segments = filmstripSegments(config);
    expect(segments.map((segment) => segment.dwellSeconds)).toEqual([0, 0]);
    expect(segments[0].widthPercent).toBe(50);
    expect(segments[1].widthPercent).toBe(50);
  });

  test("nextFilmstripCardId wraps past the last segment", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = addEntry(config, "workday", "pomodoro");
    const segments = filmstripSegments(config);
    expect(nextFilmstripCardId(segments, "clock")).toBe("pomodoro");
    expect(nextFilmstripCardId(segments, "pomodoro")).toBe("clock");
    expect(nextFilmstripCardId(segments, "unknown-id")).toBe("clock");
    expect(nextFilmstripCardId([], "clock")).toBeNull();
  });

  test("filmstripDeadline offsets from the given start time by the dwell, flooring a non-positive dwell to one second", () => {
    expect(filmstripDeadline(1_000, 45)).toBe(1_000 + 45_000);
    expect(filmstripDeadline(1_000, 0)).toBe(1_000 + 1_000);
    expect(filmstripDeadline(1_000, -5)).toBe(1_000 + 1_000);
  });

  test("filmstripAdvance's due/not-due decision is a pure function of elapsed time, not of how many times it is checked", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = addEntry(config, "workday", "pomodoro");
    config = setEntryDwell(config, "workday", 0, 45);
    config = setEntryDwell(config, "workday", 1, 20);
    config = setPlaylistAdvance(config, "workday", {
      kind: "timed",
      default_dwell_seconds: 20,
    });
    const segments = filmstripSegments(config);
    const startedAtMs = 0;
    const deadlineMs = filmstripDeadline(startedAtMs, 45); // 45_000

    // Simulates a re-render-happy caller re-checking the very same deadline
    // hundreds of times before it is actually due — this is exactly the
    // shape of the bug being regression-tested: a snapshot-driven re-render
    // roughly once a second must never itself cause an advance. However
    // many times this is checked before the deadline, the answer must stay
    // "not yet".
    for (let check = 0; check < 500; check += 1) {
      expect(filmstripAdvance(segments, "clock", deadlineMs, deadlineMs - 1)).toBeNull();
    }
    expect(filmstripAdvance(segments, "clock", deadlineMs, 0)).toBeNull();

    // Once real time has actually reached the deadline, it advances —
    // regardless of the fact that it was checked 500 times first without
    // effect, and the new deadline is a fresh dwell for the new card.
    expect(filmstripAdvance(segments, "clock", deadlineMs, deadlineMs)).toEqual({
      cardId: "pomodoro",
      deadlineMs: filmstripDeadline(deadlineMs, 20),
    });
    // Checking arbitrarily far past the deadline still advances to the
    // same next card — "due" is a threshold, not a narrow window that can
    // be missed by a slow or delayed check.
    const late = filmstripAdvance(segments, "clock", deadlineMs, deadlineMs + 999_999);
    expect(late?.cardId).toBe("pomodoro");
    expect(late?.deadlineMs).toBe(filmstripDeadline(deadlineMs + 999_999, 20));
  });

  test("filmstripAdvance wraps past the last segment and resolves against whatever segments it is given, so a mid-play reorder is honoured on the next check", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = addCard(config, "weather").config;
    config = addEntry(addEntry(config, "workday", "pomodoro"), "workday", "weather");
    const segments = filmstripSegments(config);
    // pomodoro and weather swapped, leaving clock (the active/current card) exactly where
    // it was — isolating the reorder's effect to "what comes after a".
    const reordered = filmstripSegments(moveEntry(config, "workday", 1, 2));

    // In the original order, clock advances to pomodoro...
    expect(filmstripAdvance(segments, "clock", 1_000, 1_000)?.cardId).toBe("pomodoro");
    // ...but once the playlist is reordered mid-play, the very
    // same due check for the very same active card resolves against the
    // NEW order instead of a stale one — because the caller passes the
    // latest segments in on every check rather than one captured once at
    // play-start.
    expect(filmstripAdvance(reordered, "clock", 1_000, 1_000)?.cardId).toBe("weather");

    // Wrapping past the last segment still returns to the first.
    expect(filmstripAdvance(segments, "weather", 1_000, 1_000)?.cardId).toBe("clock");
  });

  test("filmstripAdvance returns null with no segments or nothing to advance to", () => {
    expect(filmstripAdvance([], "a", 1_000, 1_000)).toBeNull();
  });

  test("formatDuration renders whole-second durations in the ribbon's units, dropping leading zero units", () => {
    expect(formatDuration(65)).toBe("1 min 5 s");
    expect(formatDuration(0)).toBe("0 s");
    expect(formatDuration(45)).toBe("45 s");
    expect(formatDuration(3725)).toBe("1 hr 2 min 5 s");
  });

  test("maps backend validation paths to their inline editor section", () => {
    const issues: ValidationIssue[] = [
      { path: "cards[0].title", code: "empty", message: "required" },
      { path: "cards[1].source.value", code: "invalid-source", message: "invalid" },
    ];
    expect(issuesForPath(issues, "cards[1].source")).toEqual([issues[1]]);
    expect(issuesForPath(issues, "cards[0].size")).toEqual([]);
  });

  test("issues follow the card across a reorder", () => {
    const base = initialConfig();
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

  test("cardsContainerIssues isolates issues whose path is exactly cards", () => {
    const issues: ValidationIssue[] = [
      { path: "cards", code: "out-of-range", message: "at least one card is required" },
      { path: "cards[0].title", code: "empty", message: "required" },
      { path: "playlists[0].entries", code: "out-of-range", message: "add an entry" },
    ];
    expect(cardsContainerIssues(issues)).toEqual([issues[0]]);
    expect(cardsContainerIssues([])).toEqual([]);
    expect(cardsContainerIssues(issues.slice(1))).toEqual([]);
  });

  test("numberValue parses a numeric input, defaulting non-finite input to 0", () => {
    expect(numberValue("42")).toBe(42);
    expect(numberValue("3.5")).toBe(3.5);
    expect(numberValue("")).toBe(0);
    expect(numberValue("not a number")).toBe(0);
  });

  test("issuesForCard returns nothing for a card that no longer exists", () => {
    const config = initialConfig();
    const issues: ValidationIssue[] = [
      { path: "cards[0].title", code: "empty", message: "required" },
    ];
    expect(issuesForCard(issues, config, "removed-card")).toEqual([]);
  });

  test("issuesForField matches a leaf field but not its dotted children", () => {
    const cardIssues: ValidationIssue[] = [
      { path: "cards[0].alert", code: "out-of-range", message: "bad alert" },
      { path: "cards[0].alert.lead_minutes", code: "out-of-range", message: "bad lead" },
    ];
    expect(issuesForField(cardIssues, "alert")).toEqual([cardIssues[0]]);
    expect(issuesForField(cardIssues, "alert.lead_minutes")).toEqual([cardIssues[1]]);
  });

  test("withAlert updates only the card's alert", () => {
    const card: CardSettings = {
      ...addCard(initialConfig(), "pomodoro").config.cards[0],
      alert: { kind: "on-timer-finish", hold: { kind: "until-dismissed" } },
    };
    const next = withAlert(card, { kind: "none" });
    expect(next.alert).toEqual({ kind: "none" });
    expect(next).toEqual({ ...card, alert: { kind: "none" } });
  });

  test("tapActionDescription states plain-language tap behaviour per action", () => {
    const clock = initialConfig().cards[0];
    const withPomodoro = addCard(initialConfig(), "pomodoro");
    const pomodoro = withPomodoro.config.cards.find((card) => card.id === withPomodoro.cardId);
    if (!pomodoro) {
      throw new Error("addCard did not append the pomodoro card");
    }
    expect(tapActionDescription(clock)).toBe("Tapping this card does nothing.");
    expect(tapActionDescription(pomodoro)).toBe("Tapping this card starts or pauses its timer.");
  });

  test("contract fixtures expose cards, not widgets or screens", () => {
    const config = ipcContractFixtures.snapshot.config;
    expect(config.schema_version).toBe(6);
    expect(Array.isArray(config.cards)).toBe(true);
    expect(Array.isArray(config.playlists)).toBe(true);
    expect("widgets" in config).toBe(false);
    expect("screens" in config).toBe(false);
    expect("carousel" in config).toBe(false);
    expect(config.cards[0]).not.toHaveProperty("size");
    expect(config.cards[0]).not.toHaveProperty("presence");
    expect(config.cards[0]).toHaveProperty("alert");
  });

  test("every alert and playlist advance variant is represented in the contract", () => {
    const alertKinds = ipcContractFixtures.card_alerts.map((a) => a.kind).sort();
    expect(alertKinds).toEqual(["before-event", "none", "on-timer-finish"]);

    const advanceKinds = ipcContractFixtures.carousel_advances.map((a) => a.kind).sort();
    expect(advanceKinds).toEqual(["manual", "timed"]);
  });
});

describe("moveCard", () => {
  test("moves a card down", () => {
    const config = cardsConfig(["a", "b", "c"]);
    const next = moveCard(config, "a", 1);
    expect(next.cards.map((card) => card.id)).toEqual(["b", "a", "c"]);
  });

  test("moves a card up", () => {
    const config = cardsConfig(["a", "b", "c"]);
    const next = moveCard(config, "c", 1);
    expect(next.cards.map((card) => card.id)).toEqual(["a", "c", "b"]);
  });

  test("moves a card to index 0", () => {
    const config = cardsConfig(["a", "b", "c"]);
    const next = moveCard(config, "c", 0);
    expect(next.cards.map((card) => card.id)).toEqual(["c", "a", "b"]);
  });

  test("clamps a target index beyond the end to the last position", () => {
    const config = cardsConfig(["a", "b", "c"]);
    const next = moveCard(config, "a", 99);
    expect(next.cards.map((card) => card.id)).toEqual(["b", "c", "a"]);
  });

  test("is a no-op with a single card", () => {
    const config = cardsConfig(["only"]);
    expect(moveCard(config, "only", 5)).toBe(config);
  });

  test("is a no-op when the target index matches the source index", () => {
    const config = cardsConfig(["a", "b", "c"]);
    expect(moveCard(config, "b", 1)).toBe(config);
  });

  test("is a no-op for an unknown card id", () => {
    const config = cardsConfig(["a", "b", "c"]);
    expect(moveCard(config, "missing", 0)).toBe(config);
  });
});

describe("cardMoveFromKey", () => {
  test("keyboard reorder requires Alt plus an arrow", () => {
    expect(cardMoveFromKey("ArrowUp", true)).toBe(-1);
    expect(cardMoveFromKey("ArrowDown", true)).toBe(1);
    expect(cardMoveFromKey("ArrowDown", false)).toBe(0);
    expect(cardMoveFromKey("Enter", true)).toBe(0);
  });
});
