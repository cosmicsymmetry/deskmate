import type {
  AppConfig,
  CardKind,
  CardPresence,
  CardSettings,
  CarouselAdvance,
  ValidationIssue,
} from "./types";

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

/// A card's user-facing name: its own title/label when set, otherwise a
/// sensible fallback. Never the card id — ids are wire identifiers, not
/// something a person chose or should see.
export function cardName(card: CardSettings): string {
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
      return card.title || cardKindName(card.kind);
  }
}

export function cardKindName(kind: CardKind): string {
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

const DEFAULT_PRESENCE: CardPresence = { kind: "in-rotation", dwell_seconds: null };

/// Appends a new card with sane defaults for its kind. Supports all six card
/// kinds — weather, JSON feed and RSS are reachable here, not just
/// clock/pomodoro/calendar. A card is a screen; this never touches any
/// separate screen list because there isn't one.
export function addCard(
  config: AppConfig,
  kind: CardKind,
): {
  config: AppConfig;
  cardId: string;
} {
  const used = new Set(config.cards.map((card) => card.id));
  const cardId = nextId(kind, used);
  const common = {
    id: cardId,
    tap_action: { kind: "none" } as const,
    presence: DEFAULT_PRESENCE,
    alert: { kind: "none" } as const,
  };

  let card: CardSettings;
  switch (kind) {
    case "clock":
      card = {
        kind,
        ...common,
        title: "Desk",
        show_seconds: true,
        template: { kind: "digital-clock" },
        refresh: { kind: "device-local" },
      };
      break;
    case "pomodoro":
      card = {
        kind,
        ...common,
        label: "Focus",
        duration_seconds: 25 * 60,
        template: { kind: "progress-ring" },
        tap_action: { kind: "start-pause" },
        refresh: { kind: "device-local" },
        alert: { kind: "on-timer-finish", hold: { kind: "until-dismissed" } },
      };
      break;
    case "calendar":
      card = {
        kind,
        ...common,
        title: "Up next",
        source: { kind: "url", value: "" },
        template: { kind: "row-list" },
        refresh: { kind: "interval", minutes: 15 },
      };
      break;
    case "weather":
      card = {
        kind,
        ...common,
        title: "Weather",
        location: "",
        units: "metric",
        template: { kind: "big-number-label" },
        refresh: { kind: "interval", minutes: 30 },
      };
      break;
    case "json-feed":
      card = {
        kind,
        ...common,
        title: "Feed",
        url: "",
        mappings: [],
        template: { kind: "big-number-label" },
        refresh: { kind: "interval", minutes: 15 },
      };
      break;
    case "rss":
      card = {
        kind,
        ...common,
        title: "Headlines",
        url: "",
        max_items: 3,
        template: { kind: "row-list" },
        refresh: { kind: "interval", minutes: 30 },
      };
      break;
  }

  return { config: { ...copyConfig(config), cards: [...config.cards, card] }, cardId };
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

export function removeCard(config: AppConfig, cardId: string): AppConfig {
  return {
    ...config,
    cards: config.cards.filter((card) => card.id !== cardId),
  };
}

/// Cards with a carousel position, in loop order. Order is meaningful here —
/// it IS the carousel order.
export function rotationCards(config: AppConfig): CardSettings[] {
  return config.cards.filter((card) => card.presence.kind === "in-rotation");
}

/// Cards with no carousel position (`alert-only` or `off`). Unordered:
/// numbering them would claim a position they don't have.
export function nonRotationCards(config: AppConfig): CardSettings[] {
  return config.cards.filter((card) => card.presence.kind !== "in-rotation");
}

/// Total time for one pass through the rotation, in seconds, inheriting the
/// carousel's default dwell for cards that don't override it. `null` under
/// manual advance, where there is no loop length to speak of.
export function loopSeconds(config: AppConfig): number | null {
  const advance: CarouselAdvance = config.carousel.advance;
  if (advance.kind !== "timed") {
    return null;
  }
  const fallback = advance.default_dwell_seconds;
  return rotationCards(config).reduce((total, card) => {
    const presence = card.presence;
    return total + (presence.kind === "in-rotation" ? (presence.dwell_seconds ?? fallback) : 0);
  }, 0);
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

export function firstSelectableCard(config: AppConfig): string | null {
  return config.cards[0]?.id ?? null;
}

export function needsFirstRunGuidance(config: AppConfig): boolean {
  const kinds = new Set(config.cards.map((card) => card.kind));
  return !kinds.has("pomodoro") || !kinds.has("calendar");
}
