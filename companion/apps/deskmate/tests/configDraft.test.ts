import { describe, expect, test } from "bun:test";

import {
  addCard,
  cardMoveFromKey,
  firstSelectableCard,
  issuesForPath,
  loopSeconds,
  moveCard,
  needsFirstRunGuidance,
  nonRotationCards,
  removeCard,
  rotationCards,
} from "../src/lib/configDraft";
import type { AppConfig, ValidationIssue } from "../src/lib/types";
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
    expect(needsFirstRunGuidance(complete.config)).toBe(false);
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

  test("maps backend validation paths to their inline editor section", () => {
    const issues: ValidationIssue[] = [
      { path: "cards[0].title", code: "empty", message: "required" },
      { path: "cards[1].source.value", code: "invalid-source", message: "invalid" },
    ];
    expect(issuesForPath(issues, "cards[1].source")).toEqual([issues[1]]);
    expect(issuesForPath(issues, "cards[0].size")).toEqual([]);
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
