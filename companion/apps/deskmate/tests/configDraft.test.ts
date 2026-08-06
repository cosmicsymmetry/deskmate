import { describe, expect, test } from "bun:test";

import {
  addCard,
  cardFields,
  cardMoveFromKey,
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
  loopSeconds,
  moveCard,
  moveCardWithinRotation,
  nextFilmstripCardId,
  nonRotationCards,
  numberValue,
  removeCard,
  rotationCards,
  tapActionDescription,
  withAlert,
} from "../src/lib/configDraft";
import type { AppConfig, CardDataSnapshot, CardSettings, ValidationIssue } from "../src/lib/types";
import { ipcContractFixtures } from "../src/lib/types.contract";

function initialConfig(): AppConfig {
  return {
    schema_version: 3,
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
        presence: { kind: "in-rotation", dwell_seconds: null },
        alert: { kind: "none" },
      },
    ],
    assets: [],
    carousel: { advance: { kind: "manual" } },
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
  test("adds the full M3 demo with stable unique card IDs", () => {
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
    // The calendar card was added with an empty ICS source, so the checklist
    // still flags it even though every demanded card kind is present.
    expect(firstRunSteps(complete.config, true).some((step) => !step.done)).toBe(true);
  });

  test("first-run guidance clears without demanding specific card kinds", () => {
    // A user who wants only a clock and a weather card is fully set up.
    const config = addCard(addCard(initialConfig(), "clock").config, "weather").config;
    const configured: AppConfig = {
      ...config,
      cards: config.cards.map((card) =>
        card.kind === "weather" ? { ...card, location: "Tbilisi" } : card,
      ),
    };
    expect(firstRunSteps(configured, true).every((step) => step.done)).toBe(true);
  });

  test("guidance still flags a card that is missing its source", () => {
    const config = addCard(initialConfig(), "calendar").config; // empty ICS url
    expect(firstRunSteps(config, true).some((step) => !step.done)).toBe(true);
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
  // json-feed to `big-number-label`, a template `validate()` accepts but
  // `wire_config()` (in `config.rs`) cannot lower, so a freshly-added card of either
  // kind validated cleanly yet always failed to save. Every default here must stay
  // within the templates `wire_config()` actually implements today: `digital-clock`,
  // `progress-ring`, `row-list` — see `companion/crates/app-core/tests/config.rs`'s
  // `every_freshly_added_card_kind_validates_and_compiles` for the Rust-side proof
  // that these exact defaults both validate AND compile.
  test("every freshly-added card kind defaults to a template the wire actually implements", () => {
    const compilableTemplates = new Set(["digital-clock", "progress-ring", "row-list"]);
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

  test("removing a card removes only that card", () => {
    const withTimer = addCard(initialConfig(), "pomodoro").config;
    const next = removeCard(withTimer, "clock");
    expect(next.cards.map((card) => card.id)).toEqual(["pomodoro"]);
    expect(firstSelectableCard(next)).toBe("pomodoro");
  });

  test("cards split into rotation and non-rotation sections", () => {
    const base = initialConfig();
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
    const base = initialConfig();
    const config: AppConfig = {
      ...base,
      carousel: { advance: { kind: "timed", default_dwell_seconds: 20 } },
      cards: [
        {
          ...addCard(base, "clock").config.cards[0],
          id: "a",
          presence: { kind: "in-rotation", dwell_seconds: 45 },
        },
        {
          ...addCard(base, "clock").config.cards[0],
          id: "b",
          presence: { kind: "in-rotation", dwell_seconds: null },
        },
        { ...addCard(base, "clock").config.cards[0], id: "c", presence: { kind: "off" } },
      ],
    };
    expect(loopSeconds(config)).toBe(65);
    expect(loopSeconds({ ...config, carousel: { advance: { kind: "manual" } } })).toBeNull();
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

  test("filmstripSegments proportions each in-rotation card's width to its resolved dwell, inheriting the carousel default, and excludes alert-only/off cards", () => {
    const base = initialConfig();
    const config: AppConfig = {
      ...base,
      carousel: { advance: { kind: "timed", default_dwell_seconds: 20 } },
      cards: [
        {
          ...addCard(base, "clock").config.cards[0],
          id: "a",
          title: "Desk",
          presence: { kind: "in-rotation", dwell_seconds: 45 },
        },
        {
          ...addCard(base, "clock").config.cards[0],
          id: "b",
          title: "Up next",
          presence: { kind: "in-rotation", dwell_seconds: null }, // inherits the 20s default
        },
        {
          ...addCard(base, "clock").config.cards[0],
          id: "c",
          title: "Muted",
          presence: { kind: "off" },
        },
      ],
    };
    const segments = filmstripSegments(config);
    expect(segments.map((segment) => segment.cardId)).toEqual(["a", "b"]);
    expect(segments[0].dwellSeconds).toBe(45);
    expect(segments[1].dwellSeconds).toBe(20);
    // 45 of 65 total seconds, and 20 of 65 — proportional to dwell, not count.
    expect(segments[0].widthPercent).toBeCloseTo((45 / 65) * 100, 5);
    expect(segments[1].widthPercent).toBeCloseTo((20 / 65) * 100, 5);
    expect(segments[0].offsetPercent).toBe(0);
    expect(segments[1].offsetPercent).toBeCloseTo((45 / 65) * 100, 5);
  });

  test("filmstripSegments gives every card an equal share under manual advance, where there is no dwell to encode", () => {
    const base = initialConfig();
    const config: AppConfig = {
      ...base,
      carousel: { advance: { kind: "manual" } },
      cards: [
        { ...addCard(base, "clock").config.cards[0], id: "a" },
        { ...addCard(base, "clock").config.cards[0], id: "b" },
      ],
    };
    const segments = filmstripSegments(config);
    expect(segments.map((segment) => segment.dwellSeconds)).toEqual([0, 0]);
    expect(segments[0].widthPercent).toBe(50);
    expect(segments[1].widthPercent).toBe(50);
  });

  test("nextFilmstripCardId wraps past the last segment", () => {
    const base = initialConfig();
    const config: AppConfig = {
      ...base,
      carousel: { advance: { kind: "timed", default_dwell_seconds: 20 } },
      cards: [
        { ...addCard(base, "clock").config.cards[0], id: "a" },
        { ...addCard(base, "clock").config.cards[0], id: "b" },
      ],
    };
    const segments = filmstripSegments(config);
    expect(nextFilmstripCardId(segments, "a")).toBe("b");
    expect(nextFilmstripCardId(segments, "b")).toBe("a");
    expect(nextFilmstripCardId(segments, "unknown-id")).toBe("a");
    expect(nextFilmstripCardId([], "a")).toBeNull();
  });

  test("filmstripDeadline offsets from the given start time by the dwell, flooring a non-positive dwell to one second", () => {
    expect(filmstripDeadline(1_000, 45)).toBe(1_000 + 45_000);
    expect(filmstripDeadline(1_000, 0)).toBe(1_000 + 1_000);
    expect(filmstripDeadline(1_000, -5)).toBe(1_000 + 1_000);
  });

  test("filmstripAdvance's due/not-due decision is a pure function of elapsed time, not of how many times it is checked", () => {
    const base = initialConfig();
    const config: AppConfig = {
      ...base,
      carousel: { advance: { kind: "timed", default_dwell_seconds: 20 } },
      cards: [
        {
          ...addCard(base, "clock").config.cards[0],
          id: "a",
          presence: { kind: "in-rotation", dwell_seconds: 45 },
        },
        {
          ...addCard(base, "clock").config.cards[0],
          id: "b",
          presence: { kind: "in-rotation", dwell_seconds: 20 },
        },
      ],
    };
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
      expect(filmstripAdvance(segments, "a", deadlineMs, deadlineMs - 1)).toBeNull();
    }
    expect(filmstripAdvance(segments, "a", deadlineMs, 0)).toBeNull();

    // Once real time has actually reached the deadline, it advances —
    // regardless of the fact that it was checked 500 times first without
    // effect, and the new deadline is a fresh dwell for the new card.
    expect(filmstripAdvance(segments, "a", deadlineMs, deadlineMs)).toEqual({
      cardId: "b",
      deadlineMs: filmstripDeadline(deadlineMs, 20),
    });
    // Checking arbitrarily far past the deadline still advances to the
    // same next card — "due" is a threshold, not a narrow window that can
    // be missed by a slow or delayed check.
    const late = filmstripAdvance(segments, "a", deadlineMs, deadlineMs + 999_999);
    expect(late?.cardId).toBe("b");
    expect(late?.deadlineMs).toBe(filmstripDeadline(deadlineMs + 999_999, 20));
  });

  test("filmstripAdvance wraps past the last segment and resolves against whatever segments it is given, so a mid-play reorder is honoured on the next check", () => {
    const base = initialConfig();
    const config: AppConfig = {
      ...base,
      carousel: { advance: { kind: "timed", default_dwell_seconds: 20 } },
      cards: [
        { ...addCard(base, "clock").config.cards[0], id: "a" },
        { ...addCard(base, "clock").config.cards[0], id: "b" },
        { ...addCard(base, "clock").config.cards[0], id: "c" },
      ],
    };
    const segments = filmstripSegments(config);
    // b and c swapped, leaving "a" (the active/current card) exactly where
    // it was — isolating the reorder's effect to "what comes after a".
    const reordered = filmstripSegments({
      ...config,
      cards: [config.cards[0], config.cards[2], config.cards[1]],
    });

    // In the original order (a, b, c), "a" advances to "b"...
    expect(filmstripAdvance(segments, "a", 1_000, 1_000)?.cardId).toBe("b");
    // ...but once the rotation is reordered mid-play (now a, c, b), the very
    // same due check for the very same active card resolves against the
    // NEW order instead of a stale one — because the caller passes the
    // latest segments in on every check rather than one captured once at
    // play-start.
    expect(filmstripAdvance(reordered, "a", 1_000, 1_000)?.cardId).toBe("c");

    // Wrapping past the last segment still returns to the first.
    expect(filmstripAdvance(segments, "c", 1_000, 1_000)?.cardId).toBe("a");
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
      { path: "cards", code: "out-of-range", message: "at least one card must be in the rotation" },
      { path: "cards[0].title", code: "empty", message: "required" },
      { path: "cards[1].presence", code: "out-of-range", message: "dead card" },
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
      { path: "cards[0].presence", code: "out-of-range", message: "dead card" },
      { path: "cards[0].presence.dwell_seconds", code: "out-of-range", message: "bad dwell" },
    ];
    expect(issuesForField(cardIssues, "presence")).toEqual([cardIssues[0]]);
    expect(issuesForField(cardIssues, "presence.dwell_seconds")).toEqual([cardIssues[1]]);
  });

  test("unchecking a card's only alert also lifts it out of alert-only, so the dead combination never re-forms", () => {
    const alertOnlyCard: CardSettings = {
      ...addCard(initialConfig(), "pomodoro").config.cards[0],
      presence: { kind: "alert-only" },
      alert: { kind: "on-timer-finish", hold: { kind: "until-dismissed" } },
    };
    const next = withAlert(alertOnlyCard, { kind: "none" });
    expect(next.alert).toEqual({ kind: "none" });
    expect(next.presence).toEqual({ kind: "in-rotation", dwell_seconds: null });
  });

  test("withAlert leaves presence untouched when the card isn't alert-only", () => {
    const rotationCard: CardSettings = {
      ...addCard(initialConfig(), "pomodoro").config.cards[0],
      presence: { kind: "in-rotation", dwell_seconds: 20 },
    };
    const next = withAlert(rotationCard, { kind: "none" });
    expect(next.presence).toEqual({ kind: "in-rotation", dwell_seconds: 20 });
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
    expect(config.schema_version).toBe(3);
    expect(Array.isArray(config.cards)).toBe(true);
    expect("widgets" in config).toBe(false);
    expect("screens" in config).toBe(false);
    expect(config.cards[0]).not.toHaveProperty("size");
    expect(config.cards[0]).toHaveProperty("presence");
    expect(config.cards[0]).toHaveProperty("alert");
  });

  test("every presence and alert variant is represented in the contract", () => {
    // card_presences also covers both `in-rotation` dwell sub-variants (explicit
    // seconds and the default null), so kinds are deduped before comparing.
    const presenceKinds = [
      ...new Set(ipcContractFixtures.card_presences.map((p) => p.kind)),
    ].sort();
    expect(presenceKinds).toEqual(["alert-only", "in-rotation", "off"]);

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

describe("moveCardWithinRotation", () => {
  // Rotation cards "a", "b", "c" with a non-rotation card sitting between
  // each pair, so any implementation that forgets to map a rotation-local
  // index back onto config.cards (and instead hands moveCard a raw
  // rotation index) lands cards in the wrong slot. This is exactly the
  // shape CardList's drag/keyboard reorder produces once alert-only/off
  // cards exist alongside a rotation.
  function interleavedConfig(): AppConfig {
    const base = initialConfig();
    return {
      ...base,
      cards: [
        { ...base.cards[0], id: "a", presence: { kind: "in-rotation", dwell_seconds: null } },
        {
          ...base.cards[0],
          id: "x",
          presence: { kind: "alert-only" },
          alert: { kind: "on-timer-finish", hold: { kind: "until-dismissed" } },
        },
        { ...base.cards[0], id: "b", presence: { kind: "in-rotation", dwell_seconds: null } },
        { ...base.cards[0], id: "y", presence: { kind: "off" } },
        { ...base.cards[0], id: "c", presence: { kind: "in-rotation", dwell_seconds: null } },
      ],
    };
  }

  test("moves the first rotation card to the last rotation slot, across interspersed non-rotation cards", () => {
    const config = interleavedConfig();
    const next = moveCardWithinRotation(config, "a", 2);
    expect(rotationCards(next).map((card) => card.id)).toEqual(["b", "c", "a"]);
    // Non-rotation cards are untouched and still unordered/present.
    expect(
      nonRotationCards(next)
        .map((card) => card.id)
        .sort(),
    ).toEqual(["x", "y"]);
  });

  test("moves the last rotation card to the first rotation slot, across interspersed non-rotation cards", () => {
    const config = interleavedConfig();
    const next = moveCardWithinRotation(config, "c", 0);
    expect(rotationCards(next).map((card) => card.id)).toEqual(["c", "a", "b"]);
  });

  test("moves a middle rotation card across a single interspersed non-rotation card", () => {
    const config = interleavedConfig();
    // "a" (rotation index 0) targets rotation index 1 (where "b" sits),
    // crossing over "x" which sits between them in config.cards.
    const next = moveCardWithinRotation(config, "a", 1);
    expect(rotationCards(next).map((card) => card.id)).toEqual(["b", "a", "c"]);
  });

  test("clamps a target rotation index below the start to the first rotation slot", () => {
    const config = interleavedConfig();
    const next = moveCardWithinRotation(config, "c", -5);
    expect(rotationCards(next).map((card) => card.id)).toEqual(["c", "a", "b"]);
  });

  test("clamps a target rotation index past the end to the last rotation slot", () => {
    const config = interleavedConfig();
    const next = moveCardWithinRotation(config, "a", 99);
    expect(rotationCards(next).map((card) => card.id)).toEqual(["b", "c", "a"]);
  });

  test("is a no-op when there are no rotation cards", () => {
    const config: AppConfig = {
      ...initialConfig(),
      cards: [{ ...initialConfig().cards[0], id: "only", presence: { kind: "off" } }],
    };
    expect(moveCardWithinRotation(config, "only", 0)).toBe(config);
  });
});
