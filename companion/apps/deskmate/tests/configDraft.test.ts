import { describe, expect, test } from "bun:test";

import {
  addCard,
  cardLabel,
  cardMoveFromKey,
  cardsContainerIssues,
  cardTitle,
  copyConfig,
  firstRunSteps,
  firstSelectableCard,
  formatDuration,
  issuesForCard,
  issuesForField,
  issuesForPath,
  loopAdvance,
  loopDeadline,
  loopEntries,
  loopSeconds,
  loopSegments,
  moveCard,
  moveEntry,
  nextCardName,
  nextLoopCardId,
  numberValue,
  removeCard,
  setAdvance,
  setCardDwell,
  tapActionDescription,
  unclaimedIssues,
} from "../src/lib/configDraft";
import type {
  AppConfig,
  CardSettings,
  ValidationIssue,
} from "../src/lib/types";
import { ipcContractFixtures } from "../src/lib/types.contract";

function initialConfig(): AppConfig {
  return {
    schema_version: ipcContractFixtures.snapshot.config.schema_version,
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
        dwell_seconds: null,
      },
    ],
    image_sources: [],
    assets: [],
    advance: { kind: "manual" },
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

test("a picture card is called Picture, with the owner's words beside it", () => {
  const card: CardSettings = {
    kind: "picture",
    id: "shot",
    title: "Limits",
    source_id: "limits",
    tap_action: { kind: "none" },
    refresh: { kind: "manual" },
    alert: { kind: "none" },
    dwell_seconds: null,
  };
  expect(cardLabel(card)).toBe("Picture");
  expect(cardTitle(card)).toBe("Limits");
});

describe("configuration draft helpers", () => {
  test("adds cards with stable unique card IDs", () => {
    const withSecondClock = addCard(initialConfig(), "clock");
    expect(withSecondClock.cardId).toBe("clock-2");

    const withPomodoro = addCard(withSecondClock.config, "pomodoro");
    const complete = addCard(withPomodoro.config, "clock");
    expect(complete.cardId).toBe("clock-3");
    expect(complete.config.cards.map((card) => card.kind)).toEqual([
      "clock",
      "clock",
      "pomodoro",
      "clock",
    ]);
    expect(new Set(complete.config.cards.map((card) => card.id)).size).toBe(4);
  });

  test("first-run guidance follows add a card, then save without demanding a card kind", () => {
    const empty: AppConfig = {
      ...initialConfig(),
      cards: [],
      advance: { kind: "manual" },
    };
    expect(firstRunSteps(empty, false).map((step) => step.done)).toEqual([false, false]);

    const withPomodoro = addCard(empty, "pomodoro").config;
    expect(firstRunSteps(withPomodoro, false).map((step) => step.done)).toEqual([true, false]);
    expect(firstRunSteps(withPomodoro, true).every((step) => step.done)).toBe(true);
  });

  test("adding a card appends it to the loop, because the card list IS the loop", () => {
    const { config, cardId } = addCard(initialConfig(), "pomodoro");
    expect(config.cards.map((card) => card.kind)).toContain("pomodoro");
    expect(cardId).toBe("pomodoro");
    expect(config.cards.at(-1)?.id).toBe("pomodoro");
    expect(config.cards.at(-1)?.dwell_seconds).toBeNull();
    expect("screens" in config).toBe(false);
    expect("playlists" in config).toBe(false);
  });

  test("adding a card is gated by one bound, because there is one list", () => {
    const cardFull = cardsConfig(["a", "b", "c", "d", "e", "f", "g", "h"]);
    expect(addCard(cardFull, "clock")).toEqual({ config: cardFull, cardId: null });
  });

  test("adding a picture card enrols it in the loop", () => {
    const { config, cardId } = addCard(initialConfig(), {
      kind: "picture",
      sourceId: "limits",
      sourceName: "Claude limits",
    });

    expect(cardId).toBe("picture");
    expect(config.cards.at(-1)).toEqual({
      kind: "picture",
      id: "picture",
      // The card carries the name, not just the source. A blank title left the
      // Name field empty and made the source the only thing actually named.
      title: "Claude limits",
      source_id: "limits",
      tap_action: { kind: "none" },
      refresh: { kind: "manual" },
      alert: { kind: "none" },
      dwell_seconds: null,
    });
    expect(config.image_sources).toEqual([{ id: "limits", name: "Claude limits" }]);
  });

  test("adding a picture card declares its source exactly once", () => {
    const first = addCard(initialConfig(), {
      kind: "picture",
      sourceId: "shared",
      sourceName: "Shared picture",
    });
    const second = addCard(first.config, {
      kind: "picture",
      sourceId: "shared",
      sourceName: "Shared picture",
    });

    expect(second.config.cards.filter((card) => card.kind === "picture")).toHaveLength(2);
    expect(second.config.image_sources).toEqual([{ id: "shared", name: "Shared picture" }]);
  });

  test("adding every kind produces a reachable, addable card", () => {
    const kinds = ["clock", "pomodoro"] as const;
    let config: AppConfig = { ...initialConfig(), cards: [] };
    for (const kind of kinds) {
      config = addCard(config, kind).config;
    }
    expect(config.cards.map((card) => card.kind)).toEqual([...kinds]);
  });

  test("every freshly-added card kind defaults to a template the wire actually implements", () => {
    const compilableTemplates = new Set([
      "digital-clock",
      "analog-clock",
      "progress-ring",
    ]);
    const expectedTemplateKind: Record<string, string> = {
      clock: "digital-clock",
      pomodoro: "progress-ring",
    };
    const kinds = ["clock", "pomodoro"] as const;
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

  test("the loop is the card list, in order", () => {
    const withCards = addCard(addCard(initialConfig(), "pomodoro").config, "clock").config;
    expect(loopEntries(withCards)).toEqual(
      withCards.cards.map((card, index) => ({ index, card })),
    );
  });

  test("moves cards with clamped targets and index-safe source boundaries", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = addCard(config, "clock").config;
    expect(config.cards.map((card) => card.id)).toEqual(["clock", "pomodoro", "clock-2"]);

    const toEnd = moveEntry(config, 0, 99);
    expect(toEnd.cards.map((card) => card.id)).toEqual(["pomodoro", "clock-2", "clock"]);
    const toStart = moveEntry(toEnd, 2, -99);
    expect(toStart.cards.map((card) => card.id)).toEqual(["clock", "pomodoro", "clock-2"]);
    expect(moveEntry(config, -1, 1)).toBe(config);
    expect(moveEntry(config, 3, 1)).toBe(config);
  });

  test("sets one card's dwell without disturbing the others", () => {
    const config = addCard(initialConfig(), "pomodoro").config;
    const next = setCardDwell(config, "pomodoro", 45);
    expect(next.cards.map((card) => card.dwell_seconds)).toEqual([null, 45]);
    expect(setCardDwell(next, "no-such-card", 30)).toBe(next);
    expect(setCardDwell(next, "pomodoro", 45)).toBe(next);
  });

  test("sets the loop's advance mode", () => {
    const config: AppConfig = { ...initialConfig(), advance: { kind: "manual" } };
    const next = setAdvance(config, { kind: "timed", default_dwell_seconds: 30 });
    expect(next.advance).toEqual({ kind: "timed", default_dwell_seconds: 30 });
    // An identical write is a no-op, so a redundant edit cannot dirty the draft.
    expect(setAdvance(next, { kind: "timed", default_dwell_seconds: 30 })).toBe(next);
    expect(setAdvance(config, { kind: "manual" })).toBe(config);
  });

  test("loop length sums each card's dwell with the timed default, and is null for manual", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = setCardDwell(config, "pomodoro", 45);
    config = setAdvance(config, { kind: "timed", default_dwell_seconds: 20 });
    // The clock takes the 20 s default; the pomodoro overrides it with 45.
    expect(loopSeconds(config)).toBe(65);
    expect(loopSeconds(setAdvance(config, { kind: "manual" }))).toBeNull();
  });

  test("removing a card removes it from the loop, because they are one list", () => {
    const config = addCard(initialConfig(), "pomodoro").config;
    const next = removeCard(config, "pomodoro");
    expect(next.cards.map((card) => card.id)).toEqual(["clock"]);
    expect(firstSelectableCard(next)).toBe("clock");
  });

  test("loopSegments proportions every card by its resolved dwell", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = setCardDwell(config, "clock", 45);
    config = setAdvance(config, { kind: "timed", default_dwell_seconds: 20 });
    const segments = loopSegments(config);
    expect(segments.map((segment) => segment.cardId)).toEqual(["clock", "pomodoro"]);
    expect(segments[0].dwellSeconds).toBe(45);
    expect(segments[1].dwellSeconds).toBe(20);
    // 45 of 65 total seconds, and 20 of 65 — proportional to dwell, not count.
    expect(segments[0].widthPercent).toBeCloseTo((45 / 65) * 100, 5);
    expect(segments[1].widthPercent).toBeCloseTo((20 / 65) * 100, 5);
    expect(segments[0].offsetPercent).toBe(0);
    expect(segments[1].offsetPercent).toBeCloseTo((45 / 65) * 100, 5);
  });

  test("loopSegments gives every card an equal share under manual advance, where there is no dwell to encode", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = setCardDwell(config, "clock", 45);
    const segments = loopSegments(config);
    expect(segments.map((segment) => segment.dwellSeconds)).toEqual([0, 0]);
    expect(segments[0].widthPercent).toBe(50);
    expect(segments[1].widthPercent).toBe(50);
  });

  test("nextLoopCardId wraps past the last segment", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    const segments = loopSegments(config);
    expect(nextLoopCardId(segments, "clock")).toBe("pomodoro");
    expect(nextLoopCardId(segments, "pomodoro")).toBe("clock");
    expect(nextLoopCardId(segments, "unknown-id")).toBe("clock");
    expect(nextLoopCardId([], "clock")).toBeNull();
  });

  test("loopDeadline offsets from the given start time by the dwell, flooring a non-positive dwell to one second", () => {
    expect(loopDeadline(1_000, 45)).toBe(1_000 + 45_000);
    expect(loopDeadline(1_000, 0)).toBe(1_000 + 1_000);
    expect(loopDeadline(1_000, -5)).toBe(1_000 + 1_000);
  });

  test("loopAdvance's due/not-due decision is a pure function of elapsed time, not of how many times it is checked", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = setCardDwell(config, "clock", 45);
    config = setCardDwell(config, "pomodoro", 20);
    config = setAdvance(config, {
      kind: "timed",
      default_dwell_seconds: 20,
    });
    const segments = loopSegments(config);
    const startedAtMs = 0;
    const deadlineMs = loopDeadline(startedAtMs, 45); // 45_000

    // Simulates a re-render-happy caller re-checking the very same deadline
    // hundreds of times before it is actually due — this is exactly the
    // shape of the bug being regression-tested: a snapshot-driven re-render
    // roughly once a second must never itself cause an advance. However
    // many times this is checked before the deadline, the answer must stay
    // "not yet".
    for (let check = 0; check < 500; check += 1) {
      expect(loopAdvance(segments, "clock", deadlineMs, deadlineMs - 1)).toBeNull();
    }
    expect(loopAdvance(segments, "clock", deadlineMs, 0)).toBeNull();

    // Once real time has actually reached the deadline, it advances —
    // regardless of the fact that it was checked 500 times first without
    // effect, and the new deadline is a fresh dwell for the new card.
    expect(loopAdvance(segments, "clock", deadlineMs, deadlineMs)).toEqual({
      cardId: "pomodoro",
      deadlineMs: loopDeadline(deadlineMs, 20),
    });
    // Checking arbitrarily far past the deadline still advances to the
    // same next card — "due" is a threshold, not a narrow window that can
    // be missed by a slow or delayed check.
    const late = loopAdvance(segments, "clock", deadlineMs, deadlineMs + 999_999);
    expect(late?.cardId).toBe("pomodoro");
    expect(late?.deadlineMs).toBe(loopDeadline(deadlineMs + 999_999, 20));
  });

  test("loopAdvance wraps past the last segment and resolves against whatever segments it is given, so a mid-play reorder is honoured on the next check", () => {
    let config = addCard(initialConfig(), "pomodoro").config;
    config = addCard(config, "clock").config;

    const segments = loopSegments(config);
    // pomodoro and the second clock swapped, leaving clock (the active/current card) exactly where
    // it was — isolating the reorder's effect to "what comes after a".
    const reordered = loopSegments(moveEntry(config, 1, 2));

    // In the original order, clock advances to pomodoro...
    expect(loopAdvance(segments, "clock", 1_000, 1_000)?.cardId).toBe("pomodoro");
    // ...but once the playlist is reordered mid-play, the very
    // same due check for the very same active card resolves against the
    // NEW order instead of a stale one — because the caller passes the
    // latest segments in on every check rather than one captured once at
    // play-start.
    expect(loopAdvance(reordered, "clock", 1_000, 1_000)?.cardId).toBe("clock-2");

    // Wrapping past the last segment still returns to the first.
    expect(loopAdvance(segments, "clock-2", 1_000, 1_000)?.cardId).toBe("clock");
  });

  test("loopAdvance returns null with no segments or nothing to advance to", () => {
    expect(loopAdvance([], "a", 1_000, 1_000)).toBeNull();
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
        { ...addCard(base, "clock").config.cards[0], id: "b" },
      ],
    };
    const issues: ValidationIssue[] = [
      { path: "cards[1].title", code: "out-of-range", message: "card title is required" },
    ];

    expect(issuesForCard(issues, config, "b")).toHaveLength(1);
    expect(issuesForCard(issues, config, "a")).toHaveLength(0);

    // After dragging "b" to the front, the SAME issue list must still attach to "b".
    const reordered = moveCard(config, "b", 0);
    const rebased: ValidationIssue[] = [
      { path: "cards[0].title", code: "out-of-range", message: "card title is required" },
    ];
    expect(issuesForCard(rebased, reordered, "b")).toHaveLength(1);
    expect(issuesForCard(rebased, reordered, "a")).toHaveLength(0);
  });

  test("cardsContainerIssues isolates issues whose path is exactly cards", () => {
    const issues: ValidationIssue[] = [
      { path: "cards", code: "out-of-range", message: "at least one card is required" },
      { path: "cards[0].title", code: "empty", message: "required" },
      { path: "advance.default_dwell_seconds", code: "out-of-range", message: "bad dwell" },
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

  test("inactive-playlist issues fall through to the unclaimed fallback", () => {
    const base = initialConfig();
    const config: AppConfig = {
      ...base,
      advance: { kind: "manual" },
    };
    const issue: ValidationIssue = {
      path: "playlists[1].name",
      code: "empty",
      message: "Playlist name is required.",
    };
    expect(unclaimedIssues([issue], config)).toEqual([issue]);
  });

  test("contract fixtures expose cards, not widgets or screens", () => {
    const config = ipcContractFixtures.snapshot.config;
    expect(config.schema_version).toBe(10);
    expect(Array.isArray(config.cards)).toBe(true);
    expect("playlists" in config).toBe(false);
    expect("widgets" in config).toBe(false);
    expect("screens" in config).toBe(false);
    expect("carousel" in config).toBe(false);
    expect(config.cards[0]).not.toHaveProperty("size");
    expect(config.cards[0]).not.toHaveProperty("presence");
    expect(config.cards[0]).toHaveProperty("alert");
  });

  test("every alert and playlist advance variant is represented in the contract", () => {
    const alertKinds = ipcContractFixtures.card_alerts.map((a) => a.kind).sort();
    expect(alertKinds).toEqual(["none", "on-timer-finish"]);

    const advanceKinds = ipcContractFixtures.carousel_advances.map((a) => a.kind).sort();
    expect(advanceKinds).toEqual(["manual", "timed"]);
  });

  test("the contract represents every supported card and known device capability", () => {
    expect(ipcContractFixtures.card_settings.map((card) => card.kind).sort()).toEqual([
      "clock",
      "picture",
      "pomodoro",
    ]);
    expect(ipcContractFixtures.device_capabilities).toContain("volatile-assets");
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
    expect(cardMoveFromKey("ArrowLeft", true)).toBe(-1);
    expect(cardMoveFromKey("ArrowDown", true)).toBe(1);
    expect(cardMoveFromKey("ArrowRight", true)).toBe(1);
    expect(cardMoveFromKey("ArrowDown", false)).toBe(0);
    expect(cardMoveFromKey("Enter", true)).toBe(0);
  });
});

describe("automatic card naming", () => {
  test("the first card takes the bare label and later ones are numbered", () => {
    const config = initialConfig();
    expect(nextCardName(config, "Weather")).toBe("Weather");

    const withOne = {
      ...config,
      image_sources: [{ id: "a", name: "Weather" }],
    };
    expect(nextCardName(withOne, "Weather")).toBe("Weather 2");

    const withTwo = {
      ...config,
      image_sources: [
        { id: "a", name: "Weather" },
        { id: "b", name: "Weather 2" },
      ],
    };
    expect(nextCardName(withTwo, "Weather")).toBe("Weather 3");
  });

  test("a freed number is reused instead of counting upward forever", () => {
    // Counting existing entries is what produced two sources both called
    // "Picture 2" in the live store: delete the middle one and the next mint
    // collides with a name that is still taken.
    const config = {
      ...initialConfig(),
      image_sources: [
        { id: "a", name: "Weather" },
        { id: "c", name: "Weather 3" },
      ],
    };
    expect(nextCardName(config, "Weather")).toBe("Weather 2");
  });

  test("a name typed on a card is respected, not just source names", () => {
    const config = initialConfig();
    expect(nextCardName(config, "Desk")).toBe("Desk 2");
  });
});
