import type { AppConfig, CardSettings, ValidationIssue } from "./types";

export type WidgetKind = "clock" | "pomodoro" | "calendar";

export function copyConfig(config: AppConfig): AppConfig {
  return {
    ...config,
    preferences: { ...config.preferences },
    cards: config.cards.map((card) => ({
      ...card,
      ...(card.kind === "calendar" ? { source: { ...card.source } } : {}),
      ...(card.kind === "json-feed"
        ? { mappings: card.mappings.map((mapping) => ({ ...mapping })) }
        : {}),
      template: { ...card.template },
      tap_action: { ...card.tap_action },
      refresh: { ...card.refresh },
      presence: { ...card.presence },
      alert: { ...card.alert },
    })) as CardSettings[],
    assets: config.assets.map((asset) => ({
      ...asset,
      source: { ...asset.source },
      kind:
        asset.kind.kind === "icon"
          ? { ...asset.kind }
          : {
              ...asset.kind,
              glyph_ranges: asset.kind.glyph_ranges.map((range) => ({ ...range })),
            },
    })),
    carousel: { ...config.carousel, advance: { ...config.carousel.advance } },
    updater: { ...config.updater },
  };
}

export function widgetName(card: CardSettings): string {
  switch (card.kind) {
    case "clock":
      return card.title || "Digital clock";
    case "pomodoro":
      return card.label || "Pomodoro";
    case "calendar":
      return card.title || "Calendar";
    case "weather":
    case "json-feed":
    case "rss":
      return card.title || card.kind;
  }
}

export function widgetKindName(kind: CardSettings["kind"]): string {
  switch (kind) {
    case "clock":
      return "Digital clock";
    case "pomodoro":
      return "Pomodoro";
    case "calendar":
      return "ICS calendar";
    case "weather":
      return "Weather";
    case "json-feed":
      return "JSON feed";
    case "rss":
      return "RSS feed";
  }
}

function nextId(prefix: string, used: Set<string>): string {
  if (!used.has(prefix)) {
    return prefix;
  }
  let suffix = 2;
  while (used.has(`${prefix}-${suffix}`)) {
    suffix += 1;
  }
  return `${prefix}-${suffix}`;
}

export function addWidget(
  config: AppConfig,
  kind: WidgetKind,
): {
  config: AppConfig;
  widgetId: string;
} {
  const next = copyConfig(config);
  const cardIds = new Set(next.cards.map((card) => card.id));
  const widgetId = nextId(kind, cardIds);

  let card: CardSettings;
  switch (kind) {
    case "clock":
      card = {
        kind,
        id: widgetId,
        title: "Desk",
        show_seconds: true,
        template: { kind: "digital-clock" },
        tap_action: { kind: "none" },
        refresh: { kind: "device-local" },
        presence: { kind: "in-rotation", dwell_seconds: null },
        alert: { kind: "none" },
      };
      break;
    case "pomodoro":
      card = {
        kind,
        id: widgetId,
        label: "Focus",
        duration_seconds: 25 * 60,
        template: { kind: "progress-ring" },
        tap_action: { kind: "start-pause" },
        refresh: { kind: "device-local" },
        presence: { kind: "in-rotation", dwell_seconds: null },
        alert: { kind: "none" },
      };
      break;
    case "calendar":
      card = {
        kind,
        id: widgetId,
        title: "Up next",
        source: { kind: "url", value: "" },
        template: { kind: "row-list" },
        tap_action: { kind: "none" },
        refresh: { kind: "interval", minutes: 15 },
        presence: { kind: "in-rotation", dwell_seconds: null },
        alert: { kind: "none" },
      };
      break;
  }

  next.cards.push(card);
  return { config: next, widgetId };
}

export function updateWidget(
  config: AppConfig,
  widgetId: string,
  replacement: CardSettings,
): AppConfig {
  return {
    ...config,
    cards: config.cards.map((card) => (card.id === widgetId ? replacement : card)),
  };
}

export function removeWidget(config: AppConfig, widgetId: string): AppConfig {
  return {
    ...config,
    cards: config.cards.filter((card) => card.id !== widgetId),
  };
}

export function issuesForPath(issues: ValidationIssue[], path: string): ValidationIssue[] {
  return issues.filter((issue) => issue.path === path || issue.path.startsWith(`${path}.`));
}

/// Reorders `config.cards` by moving the card identified by `cardId` to
/// `targetIndex`, clamped into range. No-ops (returning the same `config`
/// reference) when the card is unknown, there are fewer than two cards, or
/// the target index resolves to the card's current position. Pure: never
/// mutates `config` or its `cards` array.
export function moveCard(config: AppConfig, cardId: string, targetIndex: number): AppConfig {
  const cards = config.cards;
  const sourceIndex = cards.findIndex((card) => card.id === cardId);
  if (sourceIndex < 0 || cards.length < 2) {
    return config;
  }
  const boundedTarget = Math.max(0, Math.min(targetIndex, cards.length - 1));
  if (sourceIndex === boundedTarget) {
    return config;
  }
  const reordered = [...cards];
  const [moved] = reordered.splice(sourceIndex, 1);
  reordered.splice(boundedTarget, 0, moved);
  return { ...config, cards: reordered };
}

export function cardMoveFromKey(key: string, altKey: boolean): -1 | 0 | 1 {
  if (!altKey) {
    return 0;
  }
  if (key === "ArrowUp") {
    return -1;
  }
  if (key === "ArrowDown") {
    return 1;
  }
  return 0;
}

export function firstSelectableWidget(config: AppConfig): string | null {
  return config.cards[0]?.id ?? null;
}

export function needsFirstRunGuidance(config: AppConfig): boolean {
  const kinds = new Set(config.cards.map((card) => card.kind));
  return !kinds.has("pomodoro") || !kinds.has("calendar");
}
