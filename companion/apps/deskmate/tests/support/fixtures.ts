import type { AppConfig, CardSettings } from "../../src/lib/types";
import { apiContractFixtures } from "../../src/lib/types.contract";
export const snapshot = apiContractFixtures.snapshot;
export const cards = snapshot.config.cards;

export function clockCard(
  id: string,
  title = `Card ${id}`,
  alert: CardSettings["alert"] = { kind: "none" },
): Extract<CardSettings, { kind: "clock" }> {
  return {
    kind: "clock",
    id,
    title,
    show_seconds: true,
    template: { kind: "digital-clock" },
    tap_action: { kind: "none" },
    refresh: { kind: "device-local" },
    alert,
    dwell_seconds: null,
  };
}

export function pictureCard(id = "picture-card"): Extract<CardSettings, { kind: "picture" }> {
  return {
    kind: "picture",
    id,
    title: "Limits",
    source_id: "limits-source",
    tap_action: { kind: "none" },
    refresh: { kind: "manual" },
    alert: { kind: "none" },
    dwell_seconds: null,
  };
}

/**
 * A pomodoro with a distinguishable label.
 *
 * Tests that need two tellable-apart cards use this rather than two clocks:
 * since the Name field went, every clock is called "Clock" and two of them are
 * genuinely indistinguishable on every surface. The timer label is the one name
 * an owner can still type -- and it is drawn on the panel.
 */
export function pomodoroCard(
  id: string,
  label: string,
): Extract<CardSettings, { kind: "pomodoro" }> {
  return {
    kind: "pomodoro",
    id,
    label,
    duration_seconds: 1500,
    template: { kind: "progress-ring" },
    tap_action: { kind: "start-pause" },
    refresh: { kind: "device-local" },
    alert: { kind: "none" },
    dwell_seconds: null,
  };
}

export function cardListConfig(cardList: CardSettings[]): AppConfig {
  return {
    schema_version: snapshot.config.schema_version,
    preferences: {
      timezone: "UTC",
      autostart: false,
      paused: false,
      orientation: "landscape",
      brightness: 78,
    },
    cards: cardList,
    image_sources: cardList.flatMap((card) =>
      card.kind === "picture" ? [{ id: card.source_id, name: "Claude limits" }] : [],
    ),
    assets: [],
    advance: { kind: "timed", default_dwell_seconds: 20 },
    updater: { channel: "stable", checks: "notify" },
  };
}
