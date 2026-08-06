import type {
  AppConfig,
  CardAlert,
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

/// Resolves `cardId` to its CURRENT index in `config.cards` and returns only
/// the issues whose backend path targets that card (`cards[i]` itself, or
/// any `cards[i].*` field beneath it). Backend validation paths are
/// array-index based (`cards[2].title`) and `cards[]` is user-reorderable,
/// so an index captured at validation time is not a stable identity — this
/// re-resolves the index from the current config on every call instead of
/// trusting a caller-supplied index, which is what keeps an issue attached
/// to the same card across a drag reorder rather than sliding onto whatever
/// card now occupies its old slot. Returns an empty array for an unknown
/// card id (e.g. one just removed) rather than throwing.
export function issuesForCard(
  issues: ValidationIssue[],
  config: AppConfig,
  cardId: string,
): ValidationIssue[] {
  const index = config.cards.findIndex((card) => card.id === cardId);
  if (index < 0) {
    return [];
  }
  return issuesForPath(issues, `cards[${index}]`);
}

/// Narrows a card's already-scoped issues (see `issuesForCard`) down to a
/// single field, matched on the path SUFFIX after the `cards[i].` prefix
/// rather than the absolute path. This is what lets `CardEditor` look up
/// per-field errors without ever knowing the card's numeric index — the
/// index-stripping already happened in `issuesForCard`, so the field name
/// alone is enough to identify the right issues regardless of where the
/// card currently sits in `config.cards`. Matching is exact (not
/// prefix-based) because every backend field path used here is a leaf: two
/// unrelated fields never share a dotted prefix (e.g. `presence` and
/// `presence.dwell_seconds` are deliberately queried separately so a dwell
/// error is not double-reported at the parent field too).
export function issuesForField(cardIssues: ValidationIssue[], field: string): ValidationIssue[] {
  return cardIssues.filter((issue) => {
    const dot = issue.path.indexOf(".");
    const suffix = dot < 0 ? "" : issue.path.slice(dot + 1);
    return suffix === field;
  });
}

/// Applies a new alert to a card. If this disables the card's only trigger
/// (`{ kind: "none" }`) while its presence is `alert-only`, also resets
/// presence to `in-rotation` — otherwise unchecking the alert checkbox
/// would silently recreate the dead-card combination (alert-only with no
/// alert) that the disabled "Alert only" radio exists to prevent the user
/// from ever selecting in the first place.
export function withAlert(card: CardSettings, alert: CardAlert): CardSettings {
  const presence: CardPresence =
    alert.kind === "none" && card.presence.kind === "alert-only"
      ? { kind: "in-rotation", dwell_seconds: null }
      : card.presence;
  return { ...card, alert, presence };
}

/// A plain-language statement of what tapping this card does, for the
/// editor's gesture disclosure. Three gestures share one physical screen —
/// tap runs the card's own action, swipe navigates the rotation, and a tap
/// while an alert is showing dismisses it instead — and nothing else in the
/// app states this, so the editor is where a person can find out what their
/// tap will actually do before they rely on it.
export function tapActionDescription(card: CardSettings): string {
  switch (card.tap_action.kind) {
    case "none":
      return "Tapping this card does nothing.";
    case "start-pause":
      return "Tapping this card starts or pauses its timer.";
    case "reset":
      return "Tapping this card resets it.";
    case "dismiss":
      return "Tapping this card dismisses it.";
    case "open-url":
      return "Tapping this card opens a web address.";
    case "open-application":
      return "Tapping this card opens an application.";
  }
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

/// Moves `cardId` to `targetRotationIndex`, a position expressed relative
/// to `rotationCards(config)` — i.e. "this card should become the Nth
/// in-rotation card" — NOT an index into `config.cards`. This is the
/// mapping `CardList` needs: rotation rows are drag/keyboard-reordered
/// among themselves, but `alert-only`/`off` cards can be interspersed
/// anywhere in the underlying `cards[]` array, so a naive rotation-index
/// passed straight to `moveCard` would land the card in the wrong slot
/// whenever a non-rotation card sits between source and target.
///
/// Resolves `targetRotationIndex` (clamped into `[0, rotationCards.length
/// - 1]`) to the in-rotation card currently occupying that slot, then
/// delegates the actual splice to `moveCard` using THAT card's real index
/// in `config.cards` — so the two cards end up swapped in rotation order
/// exactly as if the interspersed cards weren't there. No-ops (same
/// `config` reference) when there is no rotation card at the resolved
/// slot (empty rotation) or when `moveCard` itself would no-op.
export function moveCardWithinRotation(
  config: AppConfig,
  cardId: string,
  targetRotationIndex: number,
): AppConfig {
  const rotation = rotationCards(config);
  if (rotation.length === 0) {
    return config;
  }
  const bounded = Math.max(0, Math.min(targetRotationIndex, rotation.length - 1));
  const anchor = rotation[bounded];
  const targetIndex = config.cards.findIndex((card) => card.id === anchor.id);
  return moveCard(config, cardId, targetIndex);
}

export function firstSelectableCard(config: AppConfig): string | null {
  return config.cards[0]?.id ?? null;
}

export function needsFirstRunGuidance(config: AppConfig): boolean {
  const kinds = new Set(config.cards.map((card) => card.kind));
  return !kinds.has("pomodoro") || !kinds.has("calendar");
}
