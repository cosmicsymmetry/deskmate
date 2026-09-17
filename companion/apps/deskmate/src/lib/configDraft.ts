import type {
  AddableCardKind,
  AppConfig,
  CardSettings,
  CarouselAdvance,
  ValidationIssue,
} from "./types";


export const MAX_CARDS = 8;
export const MAX_IMAGE_SOURCES = 8;

export function copyConfig(config: AppConfig): AppConfig {
  return {
    ...config,
    preferences: { ...config.preferences },
    cards: config.cards.map((card) => ({
      ...card,
      ...(card.kind === "picture" ? {} : { template: { ...card.template } }),
      tap_action: { ...card.tap_action },
      refresh: { ...card.refresh },
      alert: { ...card.alert },
    })) as CardSettings[],
    image_sources: config.image_sources.map((source) => ({ ...source })),
    assets: config.assets.map((asset) => ({
      ...asset,
      source: { ...asset.source },
      kind:
        asset.kind.kind === "icon-font"
          ? { ...asset.kind, glyphs: asset.kind.glyphs.map((glyph) => ({ ...glyph })) }
          : { ...asset.kind },
    })),
    advance: { ...config.advance },
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
    case "picture":
      return card.title || "Picture";
  }
}

/**
 * What a card is called on every surface that identifies one: the loop tile, the
 * ring legend, the editor heading, the picker.
 *
 * It is the template's name, not the owner's title, by explicit owner direction: a
 * person meeting a card called "Office" or "Desk" for the first time learns nothing
 * from it, where "Air quality" and "Digital clock" say what the thing is. The owner's own
 * words survive as `cardTitle` — a quiet second line beside the label, never the
 * thing that names the card.
 *
 */
export function cardLabel(card: CardSettings): string {
  if (card.kind === "picture") {
    return "Picture";
  }
  return cardKindName(card.kind);
}

/**
 * The owner's words for this card, if they typed any. Null rather than a fallback,
 * because a secondary line repeating the label it sits under is worse than no line.
 */
export function cardTitle(card: CardSettings): string | null {
  const typed = card.kind === "pomodoro" ? card.label : card.title;
  return typed.trim() === "" ? null : typed;
}

export function cardKindName(kind: AddableCardKind): string {
  switch (kind) {
    case "clock":
      return "Digital clock";
    case "pomodoro":
      return "Pomodoro";
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

/**
 * What the caller is asking to add. Picture sources are minted asynchronously,
 * while built-ins need only their kind.
 */
/// The next free name for a card of `label`, so the owner never has to type one.
///
/// First is the bare label ("Weather"), then "Weather 2", "Weather 3". Numbering
/// looks for the lowest FREE slot rather than counting existing cards: after
/// deleting "Weather 2" of three, the next card is "Weather 2" again, not a
/// second "Weather 4". Counting by length is what produced two sources both
/// called "Picture 2" in the live store.
export function nextCardName(config: AppConfig, label: string): string {
  const taken = new Set<string>();
  for (const card of config.cards) {
    const typed = cardTitle(card);
    if (typed) {
      taken.add(typed);
    }
  }
  for (const source of config.image_sources) {
    taken.add(source.name);
  }
  if (!taken.has(label)) {
    return label;
  }
  for (let suffix = 2; suffix < 1_000; suffix += 1) {
    const candidate = `${label} ${suffix}`;
    if (!taken.has(candidate)) {
      return candidate;
    }
  }
  return label;
}

export type AddCardRequest =
  | AddableCardKind
  | { kind: "picture"; sourceId: string; sourceName: string };

/// Appends a new card with sane defaults for its kind and enrols it at the end
/// of the active loop in the same draft. Both card-library limits are checked before
/// either collection changes, so adding is atomic even when a legacy card
/// outside the loop has filled only one limit.
export function addCard(
  config: AppConfig,
  request: AddCardRequest,
): {
  config: AppConfig;
  cardId: string | null;
} {
  // One bound, because there is one list: `cards` IS the loop.
  if (config.cards.length >= MAX_CARDS) {
    return { config, cardId: null };
  }
  const used = new Set(config.cards.map((card) => card.id));
  // Server-created cards use their kind as the id stem, never the source id: that
  // external id has different bounds, and two cards may legitimately share one.
  const cardId = nextId(typeof request === "string" ? request : request.kind, used);
  const common = {
    id: cardId,
    tap_action: { kind: "none" } as const,
    alert: { kind: "none" } as const,
    dwell_seconds: null,
  };

  let card: CardSettings;
  if (typeof request !== "string") {
    card = {
      kind: request.kind,
      ...common,
      // The name the caller already chose, not blank. A card added from the menu
      // arrived with an empty title and leaned on the tile falling back to the
      // source name, which left the Name field empty and put the owner's only
      // visible name on the SOURCE rather than on the card.
      title: request.sourceName,
      source_id: request.sourceId,
      refresh: { kind: "manual" },
    };
  } else {
    switch (request) {
      case "clock":
        card = {
          kind: request,
          ...common,
          title: "Desk",
          show_seconds: true,
          template: { kind: "digital-clock" },
          refresh: { kind: "device-local" },
        };
        break;
      case "pomodoro":
        card = {
          kind: request,
          ...common,
          label: "Focus",
          duration_seconds: 25 * 60,
          template: { kind: "progress-ring" },
          tap_action: { kind: "start-pause" },
          refresh: { kind: "device-local" },
          alert: { kind: "on-timer-finish", hold: { kind: "until-dismissed" } },
        };
        break;
    }
  }

  // Spread the COPIED config's own `cards` array, not the original `config.cards` —
  // otherwise the `cards` key here overwrites `copyConfig`'s deep copy with a shallow
  // spread of the original elements, silently making that deep copy dead work on this
  // path (every pre-existing card in the returned draft would alias the live snapshot's
  // card objects instead of being an independent copy).
  const copied = copyConfig(config);
  const image_sources =
    typeof request !== "string" &&
    request.kind === "picture" &&
    !copied.image_sources.some((source) => source.id === request.sourceId)
      ? [...copied.image_sources, { id: request.sourceId, name: request.sourceName }]
      : copied.image_sources;
  // Appending to `cards` IS joining the loop.
  return {
    config: { ...copied, cards: [...copied.cards, card], image_sources },
    cardId,
  };
}

export function updateWidget(
  config: AppConfig,
  cardId: string,
  replacement: CardSettings,
): AppConfig {
  return {
    ...config,
    cards: config.cards.map((card) => (card.id === cardId ? replacement : card)),
  };
}

export function removeCard(config: AppConfig, cardId: string): AppConfig {
  if (!config.cards.some((card) => card.id === cardId)) {
    return config;
  }
  return {
    ...config,
    cards: config.cards.filter((card) => card.id !== cardId),
  };
}

export interface LoopEntry {
  index: number;
  card: CardSettings;
}

/// The loop, in order. Since schema v10 that is simply the card list: there is
/// no separate playlist, so a loop position with no card behind it -- the
/// "Missing card" tile this window used to render -- is unrepresentable.
export function loopEntries(config: AppConfig): LoopEntry[] {
  return config.cards.map((card, index) => ({ index, card }));
}

export function moveEntry(config: AppConfig, from: number, to: number): AppConfig {
  const cards = config.cards;
  if (
    !Number.isInteger(from) ||
    from < 0 ||
    from >= cards.length ||
    !Number.isFinite(to) ||
    cards.length < 2
  ) {
    return config;
  }
  const target = Math.max(0, Math.min(Math.trunc(to), cards.length - 1));
  if (from === target) {
    return config;
  }
  const next = [...cards];
  const [moved] = next.splice(from, 1);
  next.splice(target, 0, moved);
  return { ...copyConfig(config), cards: next };
}

export function setCardDwell(
  config: AppConfig,
  cardId: string,
  dwell: number | null,
): AppConfig {
  const index = config.cards.findIndex((card) => card.id === cardId);
  if (index < 0 || config.cards[index].dwell_seconds === dwell) {
    return config;
  }
  return {
    ...copyConfig(config),
    cards: config.cards.map((card, cardIndex) =>
      cardIndex === index ? { ...card, dwell_seconds: dwell } : card,
    ),
  };
}

export function setAdvance(config: AppConfig, advance: CarouselAdvance): AppConfig {
  if (config.advance.kind === "manual" && advance.kind === "manual") {
    return config;
  }
  if (
    config.advance.kind === "timed" &&
    advance.kind === "timed" &&
    config.advance.default_dwell_seconds === advance.default_dwell_seconds
  ) {
    return config;
  }
  return { ...copyConfig(config), advance };
}

/// Total time for one pass through the loop, in seconds, inheriting the
/// document's default dwell for cards that do not override it. `null` under
/// manual advance, where there is no loop length to speak of.
export function loopSeconds(config: AppConfig): number | null {
  if (config.advance.kind !== "timed") {
    return null;
  }
  const advance = config.advance;
  return config.cards.reduce(
    (total, card) => total + (card.dwell_seconds ?? advance.default_dwell_seconds),
    0,
  );
}

/// One ribbon segment: a card plus its resolved dwell and the
/// proportional width/offset (both 0-100) that dwell earns in the loop
/// ribbon. Pure and independent of any DOM/flex mechanics so the width math
/// — the whole point of the ribbon — can be unit-tested without rendering
/// anything.
export interface LoopSegment {
  cardId: string;
  name: string;
  /// The owner's own words for this card, or null when they typed none. Two cards
  /// can share a kind, so the loop needs something besides `name` to tell them apart.
  title: string | null;
  dwellSeconds: number;
  widthPercent: number;
  offsetPercent: number;
}

/// Builds the ribbon's segments from the card list, which IS the loop. Under
/// manual advance there is no dwell to speak of, so every segment is given
/// equal width instead of a zero-width one, which is what lets the ribbon
/// still show order (just not timing) in that mode.
export function loopSegments(config: AppConfig): LoopSegment[] {
  const advance = config.advance;
  const dwellSeconds = config.cards.map((card) =>
    advance.kind === "timed" ? (card.dwell_seconds ?? advance.default_dwell_seconds) : 0,
  );
  const total = dwellSeconds.reduce((sum, seconds) => sum + seconds, 0);
  const equalShare = config.cards.length > 0 ? 100 / config.cards.length : 0;
  let offset = 0;
  return config.cards.map((card, index) => {
    const widthPercent = total > 0 ? (dwellSeconds[index] / total) * 100 : equalShare;
    const segment: LoopSegment = {
      cardId: card.id,
      name: cardLabel(card),
      title: cardTitle(card),
      dwellSeconds: dwellSeconds[index],
      widthPercent,
      offsetPercent: offset,
    };
    offset += widthPercent;
    return segment;
  });
}

/// The segment the ribbon's play control should move to next, wrapping past
/// the end. Returns `null` only when there is nothing to advance to.
export function nextLoopCardId(
  segments: LoopSegment[],
  currentCardId: string | null,
): string | null {
  if (segments.length === 0) {
    return null;
  }
  const index = segments.findIndex((segment) => segment.cardId === currentCardId);
  const nextIndex = index < 0 ? 0 : (index + 1) % segments.length;
  return segments[nextIndex].cardId;
}

/// The absolute deadline (matching `Date.now()`'s epoch) at which a segment
/// begun at `startedAtMs` finishes its dwell. A one-second floor keeps a
/// zero/negative dwell from producing an immediately-due, tight-loop
/// advance. Kept as its own function so a caller can compute a deadline
/// once and then only ever compare it against "now" — never re-derive it
/// from a relative duration on every check, which is what let the ribbon's
/// old relative `setTimeout` get silently re-armed by unrelated re-renders
/// before it ever had a chance to fire.
export function loopDeadline(startedAtMs: number, dwellSeconds: number): number {
  return startedAtMs + Math.max(1, dwellSeconds) * 1000;
}

export interface LoopAdvance {
  cardId: string;
  deadlineMs: number;
}

/// The one place "has enough real time elapsed to advance" is decided — a
/// pure function of `deadlineMs` and `nowMs` alone, never of how many times
/// a caller has re-rendered or re-checked it. Returns `null` before the
/// deadline (nothing to do yet). Once `nowMs` has reached it, returns the
/// next card in the loop together with the deadline for THAT card's own
/// dwell, so a caller can just feed the previous result's `deadlineMs`
/// back in on every tick without tracking anything else.
export function loopAdvance(
  segments: LoopSegment[],
  activeCardId: string | null,
  deadlineMs: number,
  nowMs: number,
): LoopAdvance | null {
  if (nowMs < deadlineMs) {
    return null;
  }
  const nextCardId = nextLoopCardId(segments, activeCardId);
  if (!nextCardId) {
    return null;
  }
  const nextSegment = segments.find((segment) => segment.cardId === nextCardId);
  return {
    cardId: nextCardId,
    deadlineMs: loopDeadline(nowMs, nextSegment?.dwellSeconds ?? 0),
  };
}

/// Renders a whole-second duration as "1 hr 2 min 3 s", dropping leading
/// zero units (but never the trailing seconds, so `0` still reads as "0 s"
/// rather than an empty string). Used for the ribbon's total loop length;
/// tabular-numeral styling is applied by the caller's CSS, not here.
export function formatDuration(totalSeconds: number): string {
  const whole = Math.max(0, Math.round(totalSeconds));
  const hours = Math.floor(whole / 3600);
  const minutes = Math.floor((whole % 3600) / 60);
  const seconds = whole % 60;
  const parts: string[] = [];
  if (hours > 0) {
    parts.push(`${hours} hr`);
  }
  if (hours > 0 || minutes > 0) {
    parts.push(`${minutes} min`);
  }
  parts.push(`${seconds} s`);
  return parts.join(" ");
}

export function issuesForPath(issues: ValidationIssue[], path: string): ValidationIssue[] {
  return issues.filter(
    (issue) =>
      issue.path === path || issue.path.startsWith(`${path}.`) || issue.path.startsWith(`${path}[`),
  );
}

/// Issues whose path is exactly `cards` — the container-level rules such as "at least
/// one cards entry is required" — as opposed to
/// any per-card `cards[i]*` issue, which `issuesForCard` already resolves to a row.
/// Nothing rendered these before this helper existed: `issuesForCard` only ever matches
/// `cards[i]` paths, so a bare `cards` issue blocked Save with no highlighted control
/// anywhere in the UI. The
/// card list header is the natural place to show it, since it names the whole
/// collection rather than any one row.
export function cardsContainerIssues(issues: ValidationIssue[]): ValidationIssue[] {
  return issues.filter((issue) => issue.path === "cards");
}

/// Every issue an existing surface already claims and renders: the cards container
/// banner (`cardsContainerIssues`), each card's own row-scoped issues (`issuesForCard`,
/// checked for every card in the draft — not just whichever one is currently selected,
/// since selection is a UI-only concern this must not depend on), the active loop's
/// entries and pacing control, and the timezone field. Returns the actual issue objects (by
/// reference into `issues`) rather than paths, so `unclaimedIssues` can compute an exact
/// set difference without re-deriving path-matching rules of its own.
function claimedIssues(issues: ValidationIssue[], config: AppConfig): ValidationIssue[] {
  const claimed = new Set<ValidationIssue>();
  const claim = (matched: ValidationIssue[]) => {
    for (const issue of matched) {
      claimed.add(issue);
    }
  };
  claim(cardsContainerIssues(issues));
  for (const card of config.cards) {
    claim(issuesForCard(issues, config, card.id));
  }
  claim(issuesForPath(issues, "advance"));
  claim(issuesForPath(issues, "advance.default_dwell_seconds"));
  claim(issuesForPath(issues, "preferences.timezone"));
  return [...claimed];
}

/// Issues no existing surface renders anywhere — the fallback set a whole-app banner
/// shows so a validation issue can never again silently vanish just because its path
/// predates whatever surface would normally claim it. This is exactly how
/// `device.capabilities` went missing: capability-aware validation started emitting
/// issues on that path before any surface knew to look for it, Save stayed disabled,
/// and nothing on screen said why. Defined as the complement of `claimedIssues` (a set
/// difference), not as a list of paths this function itself excludes, so a *future* new
/// path falls through to the fallback automatically instead of requiring another
/// hard-coded case here.
export function unclaimedIssues(issues: ValidationIssue[], config: AppConfig): ValidationIssue[] {
  const claimed = new Set(claimedIssues(issues, config));
  return issues.filter((issue) => !claimed.has(issue));
}

/// Parses a numeric `<input>` value into a finite number, defaulting to `0` for
/// anything else (an empty string mid-edit, a stray non-numeric paste). Shared by every
/// numeric field editor so the "empty box while typing" case is handled identically
/// everywhere rather than reimplemented per field.
export function numberValue(value: string): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
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
/// unrelated fields never share a dotted prefix (e.g. `alert` and
/// `alert.lead_minutes` are deliberately queried separately so a nested
/// error is not double-reported at the parent field too).
export function issuesForField(cardIssues: ValidationIssue[], field: string): ValidationIssue[] {
  return cardIssues.filter((issue) => {
    const dot = issue.path.indexOf(".");
    const suffix = dot < 0 ? "" : issue.path.slice(dot + 1);
    return suffix === field;
  });
}

/// A plain-language statement of what tapping this card does, for the
/// editor's gesture disclosure. Three gestures share one physical screen —
/// tap runs the card's own action, swipe moves through the loop, and a tap
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
  if (key === "ArrowUp" || key === "ArrowLeft") {
    return -1;
  }
  if (key === "ArrowDown" || key === "ArrowRight") {
    return 1;
  }
  return 0;
}

export function firstSelectableCard(config: AppConfig): string | null {
  return config.cards[0]?.id ?? null;
}

/// The first-run checklist is about the one-loop workflow and never
/// demands any specific card kind. The caller supplies whether the current
/// draft has been saved.
export function firstRunSteps(
  config: AppConfig,
  saved: boolean,
): { label: string; done: boolean }[] {
  return [
    { label: "Add a card", done: config.cards.length > 0 },
    { label: "Save your settings", done: saved },
  ];
}
