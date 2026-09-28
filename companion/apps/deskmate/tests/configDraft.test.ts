import { describe, expect, test } from "bun:test";

import {
  addCard,
  cardIdentity,
  cardLabel,
  cardMoveFromKey,
  cardsContainerIssues,
  firstRunSteps,
  firstSelectableCard,
  formatDuration,
  issuesForCard,
  issuesForField,
  issuesForPath,
  loopAdvance,
  loopDeadline,
  loopSeconds,
  loopSegments,
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
import type { AppConfig, CardSettings, ValidationIssue } from "../src/lib/types";
import { cardListConfig, clockCard } from "./support/fixtures";
import { apiContractFixtures } from "../src/lib/types.contract";

function initialConfig(): AppConfig {
  return {
    ...cardListConfig([clockCard("clock", "Desk")]),
    advance: { kind: "manual" },
  };
}

function cardsConfig(ids: string[]): AppConfig {
  const base = initialConfig();
  return {
    ...base,
    cards: ids.map((id) => ({ ...base.cards[0], id })),
  };
}

test("a picture card uses its source identity while retaining its frozen title", () => {
  const card: CardSettings = {
    kind: "picture",
    id: "shot",
    title: "Frozen legacy title",
    source_id: "limits",
    tap_action: { kind: "none" },
    refresh: { kind: "manual" },
    alert: { kind: "none" },
    dwell_seconds: null,
  };
  expect(cardLabel(card)).toBe("Picture");
  expect(cardIdentity(card, [{ id: "limits", name: "Claude limits" }])).toBe("Claude limits");
  expect(card.title).toBe("Frozen legacy title");
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
      // The schema title is frozen at creation; visible identity comes from the source.
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
    const compilableTemplates = new Set(["digital-clock", "analog-clock", "progress-ring"]);
    const expectedTemplateKind: Record<"clock" | "pomodoro", "digital-clock" | "progress-ring"> = {
      clock: "digital-clock",
      pomodoro: "progress-ring",
    };
    const kinds = ["clock", "pomodoro"] as const;
    let config: AppConfig = { ...initialConfig(), cards: [] };
    for (const kind of kinds) {
      const { config: next, cardId } = addCard(config, kind);
      config = next;
      const card = config.cards.find((candidate) => candidate.id === cardId);
      if (!card || card.kind !== kind) {
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
    const config = addCard(initialConfig(), "pomodoro").config;
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

    expect(loopAdvance(segments, "clock", deadlineMs, deadlineMs - 1)).toBeNull();
    expect(loopAdvance(segments, "clock", deadlineMs, 0)).toBeNull();

    // At the deadline, the next card gets a fresh dwell.
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
    // ...but once the loop is reordered mid-play, the very
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

  test("routes newly validated index paths after reordering", () => {
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

    // After moving "b" to the front, use issues from validation of that new order.
    const reordered = moveEntry(config, 1, 0);
    const revalidatedIssues: ValidationIssue[] = [
      { path: "cards[0].title", code: "out-of-range", message: "card title is required" },
    ];
    expect(issuesForCard(revalidatedIssues, reordered, "b")).toHaveLength(1);
    expect(issuesForCard(revalidatedIssues, reordered, "a")).toHaveLength(0);
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
    // A face that answers taps speaks for itself; a blank sentence is not a face.
    expect(tapActionDescription(clock, "Tap the panel for the coming days.")).toBe(
      "Tap the panel for the coming days.",
    );
    expect(tapActionDescription(clock, null)).toBe("Tapping this card does nothing.");
    expect(tapActionDescription(clock, "   ")).toBe("Tapping this card does nothing.");
    expect(tapActionDescription(pomodoro)).toBe("Tapping this card starts or pauses its timer.");
  });

  test("unknown future paths fall through to the unclaimed fallback", () => {
    const base = initialConfig();
    const config: AppConfig = {
      ...base,
      advance: { kind: "manual" },
    };
    const issue: ValidationIssue = {
      path: "future.setting",
      code: "empty",
      message: "A future setting is required.",
    };
    expect(unclaimedIssues([issue], config)).toEqual([issue]);
  });

  test("contract fixtures expose cards, not widgets or screens", () => {
    const config = apiContractFixtures.snapshot.config;
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

  test("every alert and loop advance variant is represented in the contract", () => {
    const alertKinds = apiContractFixtures.card_alerts.map((a) => a.kind).sort();
    expect(alertKinds).toEqual(["none", "on-timer-finish"]);

    const advanceKinds = apiContractFixtures.carousel_advances.map((a) => a.kind).sort();
    expect(advanceKinds).toEqual(["manual", "timed"]);
  });

  test("the contract represents every supported card and known device capability", () => {
    expect(apiContractFixtures.card_settings.map((card) => card.kind).sort()).toEqual([
      "clock",
      "picture",
      "pomodoro",
    ]);
    expect(apiContractFixtures.device_capabilities).toContain("volatile-assets");
  });
});

describe("moveEntry", () => {
  test.each([
    { name: "moves a card down", from: 0, to: 1, expected: ["b", "a", "c"] },
    { name: "moves a card up", from: 2, to: 1, expected: ["a", "c", "b"] },
    { name: "moves a card to index 0", from: 2, to: 0, expected: ["c", "a", "b"] },
    {
      name: "clamps a target index beyond the end to the last position",
      from: 0,
      to: 99,
      expected: ["b", "c", "a"],
    },
  ])("$name", ({ from, to, expected }) => {
    const config = cardsConfig(["a", "b", "c"]);
    const next = moveEntry(config, from, to);
    expect(next.cards.map((card) => card.id)).toEqual([...expected]);
  });

  test.each([
    { name: "is a no-op with a single card", ids: ["only"], from: 0, to: 5 },
    {
      name: "is a no-op when the target index matches the source index",
      ids: ["a", "b", "c"],
      from: 1,
      to: 1,
    },
    { name: "is a no-op for an invalid source index", ids: ["a", "b", "c"], from: -1, to: 0 },
  ])("$name", ({ ids, from, to }) => {
    const config = cardsConfig([...ids]);
    expect(moveEntry(config, from, to)).toBe(config);
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

  test("invisible clock and picture titles do not reserve names or get rewritten", () => {
    const config = initialConfig();
    const clock = config.cards[0];
    if (clock.kind !== "clock") throw new Error("missing clock fixture");
    clock.title = "Weather";
    const picture: CardSettings = {
      kind: "picture",
      id: "legacy-picture",
      title: "Picture",
      source_id: "legacy-source",
      tap_action: { kind: "none" },
      refresh: { kind: "manual" },
      alert: { kind: "none" },
      dwell_seconds: null,
    };
    config.cards.push(picture);
    config.image_sources.push({ id: "legacy-source", name: "Different source name" });

    expect(nextCardName(config, "Weather")).toBe("Weather");
    expect(nextCardName(config, "Picture")).toBe("Picture");
    expect(clock.title).toBe("Weather");
    expect(picture.title).toBe("Picture");
    expect(config.image_sources[0].name).toBe("Different source name");
  });

  test("a nonblank pomodoro label still reserves its visible name", () => {
    const config = addCard(initialConfig(), "pomodoro").config;
    const pomodoro = config.cards.find((card) => card.kind === "pomodoro");
    if (pomodoro?.kind !== "pomodoro") throw new Error("missing pomodoro fixture");
    pomodoro.label = "Weather";
    expect(nextCardName(config, "Weather")).toBe("Weather 2");
  });

  test("removing a picture card frees its source, and keeps one another card shares", () => {
    // The bug this pins accumulated real credentials on the live server:
    // "Weather 2" and "Weather 3" were sources no card referenced, still
    // declared in the document, and offered back in the add menu as things to
    // reuse. The document is what the server reconciles against, so a source it
    // keeps declaring is a source that can never be collected.
    const withFirst = addCard(initialConfig(), {
      kind: "picture",
      sourceId: "image-weather",
      sourceName: "Weather",
    });
    const withSecond = addCard(withFirst.config, {
      kind: "picture",
      sourceId: "image-news",
      sourceName: "Hacker News",
    });
    const config = withSecond.config;
    expect(config.image_sources.map((source) => source.id)).toEqual([
      "image-weather",
      "image-news",
    ]);

    if (withFirst.cardId === null) {
      throw new Error("addCard did not append the expected card");
    }
    const afterRemoval = removeCard(config, withFirst.cardId);
    expect(afterRemoval.image_sources.map((source) => source.id)).toEqual(["image-news"]);

    // Two cards can legitimately share one source; the last one out frees it.
    const shared = addCard(config, {
      kind: "picture",
      sourceId: "image-news",
      sourceName: "Hacker News",
    });
    if (withSecond.cardId === null) {
      throw new Error("addCard did not append the expected card");
    }
    const stillShared = removeCard(shared.config, withSecond.cardId);
    expect(stillShared.image_sources.map((source) => source.id)).toContain("image-news");
    if (shared.cardId === null) {
      throw new Error("addCard did not append the expected card");
    }
    const nowFree = removeCard(stillShared, shared.cardId);
    expect(nowFree.image_sources.map((source) => source.id)).not.toContain("image-news");
  });

  test("removing a built-in card touches no source", () => {
    const withPicture = addCard(initialConfig(), {
      kind: "picture",
      sourceId: "image-weather",
      sourceName: "Weather",
    });
    const withClock = addCard(withPicture.config, "clock");
    if (withClock.cardId === null) {
      throw new Error("addCard did not append the expected card");
    }
    const afterRemoval = removeCard(withClock.config, withClock.cardId);
    expect(afterRemoval.image_sources.map((source) => source.id)).toEqual(["image-weather"]);
  });
});

test("an issue on a path no card, loop, or preference surface claims (e.g. a missing-capability issue) is not silently dropped", () => {
  const config = cardListConfig([clockCard("only-card")]);
  const capabilityIssue: ValidationIssue = {
    path: "device.capabilities",
    code: "requires-capability",
    message:
      "the connected firmware does not support asset transfer. Update the firmware, or remove the cards and settings that need it.",
  };
  expect(unclaimedIssues([capabilityIssue], config)).toEqual([capabilityIssue]);
});
