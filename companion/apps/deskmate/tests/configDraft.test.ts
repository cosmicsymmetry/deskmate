import { describe, expect, test } from "bun:test";

import {
  addWidget,
  firstSelectableWidget,
  issuesForPath,
  moveScreen,
  needsFirstRunGuidance,
  removeWidget,
  screenMoveFromKey,
} from "../src/lib/configDraft";
import type { AppConfig, ScreenSettings, ValidationIssue } from "../src/lib/types";

function initialConfig(): AppConfig {
  return {
    schema_version: 2,
    preferences: {
      timezone: "UTC",
      autostart: false,
      paused: false,
      orientation: "landscape",
    },
    widgets: [
      {
        kind: "clock",
        id: "clock",
        size: "full",
        title: "Desk",
        show_seconds: true,
        template: { kind: "digital-clock" },
        tap_action: { kind: "none" },
        refresh: { kind: "device-local" },
        interrupt_policy: "disabled",
      },
    ],
    screens: [{ id: "clock-screen", layout: { kind: "single", widget_id: "clock" } }],
    assets: [],
    carousel: { auto_advance_seconds: null },
    updater: { channel: "stable", checks: "notify" },
  };
}

describe("configuration draft helpers", () => {
  test("adds the full M3 demo with stable unique widget and screen IDs", () => {
    const withSecondClock = addWidget(initialConfig(), "clock");
    expect(withSecondClock.widgetId).toBe("clock-2");
    expect(withSecondClock.config.screens.at(-1)).toEqual({
      id: "clock-2-screen",
      layout: { kind: "single", widget_id: "clock-2" },
    });

    const withPomodoro = addWidget(withSecondClock.config, "pomodoro");
    const complete = addWidget(withPomodoro.config, "calendar");
    expect(complete.config.widgets.map((widget) => widget.kind)).toEqual([
      "clock",
      "clock",
      "pomodoro",
      "calendar",
    ]);
    expect(new Set(complete.config.widgets.map((widget) => widget.id)).size).toBe(4);
    expect(new Set(complete.config.screens.map((screen) => screen.id)).size).toBe(4);
    expect(needsFirstRunGuidance(complete.config)).toBe(false);
  });

  test("removing a widget also removes only its screen reference", () => {
    const withTimer = addWidget(initialConfig(), "pomodoro").config;
    const next = removeWidget(withTimer, "clock");
    expect(next.widgets.map((widget) => widget.id)).toEqual(["pomodoro"]);
    expect(next.screens).toEqual([
      { id: "pomodoro-screen", layout: { kind: "single", widget_id: "pomodoro" } },
    ]);
    expect(firstSelectableWidget(next)).toBe("pomodoro");
  });

  test("reorders screens without changing IDs or mutating the input", () => {
    const screens: ScreenSettings[] = [
      { id: "one", layout: { kind: "single", widget_id: "clock" } },
      { id: "two", layout: { kind: "single", widget_id: "pomodoro" } },
      { id: "three", layout: { kind: "single", widget_id: "calendar" } },
    ];
    const reordered = moveScreen(screens, "one", 2);
    expect(reordered.map((screen) => screen.id)).toEqual(["two", "three", "one"]);
    expect(screens.map((screen) => screen.id)).toEqual(["one", "two", "three"]);
    expect(moveScreen(screens, "missing", 1)).toBe(screens);
  });

  test("keyboard reorder requires Alt plus an arrow", () => {
    expect(screenMoveFromKey("ArrowUp", true)).toBe(-1);
    expect(screenMoveFromKey("ArrowDown", true)).toBe(1);
    expect(screenMoveFromKey("ArrowDown", false)).toBe(0);
    expect(screenMoveFromKey("Enter", true)).toBe(0);
  });

  test("maps backend validation paths to their inline editor section", () => {
    const issues: ValidationIssue[] = [
      { path: "widgets[0].title", code: "empty", message: "required" },
      { path: "widgets[1].source.value", code: "invalid-source", message: "invalid" },
    ];
    expect(issuesForPath(issues, "widgets[1].source")).toEqual([issues[1]]);
    expect(issuesForPath(issues, "widgets[0].size")).toEqual([]);
  });
});
