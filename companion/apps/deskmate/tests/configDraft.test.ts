import { describe, expect, test } from "bun:test";

import {
  addWidget,
  firstSelectableWidget,
  issuesForPath,
  needsFirstRunGuidance,
  removeWidget,
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

describe("configuration draft helpers", () => {
  test("adds the full M3 demo with stable unique widget IDs", () => {
    const withSecondClock = addWidget(initialConfig(), "clock");
    expect(withSecondClock.widgetId).toBe("clock-2");

    const withPomodoro = addWidget(withSecondClock.config, "pomodoro");
    const complete = addWidget(withPomodoro.config, "calendar");
    expect(complete.config.cards.map((card) => card.kind)).toEqual([
      "clock",
      "clock",
      "pomodoro",
      "calendar",
    ]);
    expect(new Set(complete.config.cards.map((card) => card.id)).size).toBe(4);
    expect(needsFirstRunGuidance(complete.config)).toBe(false);
  });

  test("removing a widget removes only that card", () => {
    const withTimer = addWidget(initialConfig(), "pomodoro").config;
    const next = removeWidget(withTimer, "clock");
    expect(next.cards.map((card) => card.id)).toEqual(["pomodoro"]);
    expect(firstSelectableWidget(next)).toBe("pomodoro");
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
