// What a face is, as far as everything outside its own file is concerned.
//
// A face is a server-side producer: it fetches whatever it needs, draws a 448x368
// SVG, and the server pushes the rastered frame to an ordinary picture card. It is
// never a card kind. Adding one is adding a file under `src/faces/` and a line in
// `registry.ts` -- no Rust change, no schema change, no binary redeploy.

/** One setting the browser companion offers for a face. */
export type FieldSpec =
  | { type: "text"; key: string; label: string; placeholder: string; default?: string }
  | { type: "url"; key: string; label: string; placeholder: string; default?: string }
  | {
      type: "enum";
      key: string;
      label: string;
      default: string;
      options: { value: string; label: string }[];
    };

/**
 * Settings as the server stores them: the descriptor's fields as strings, plus
 * anything hand-edited into `data-cards.json` that no descriptor offers (a token's
 * `api_key`). Values the owner never typed are absent, not empty.
 */
export type Settings = Readonly<Record<string, unknown>>;

/** A view a face can draw. Stable, opaque to the server, unique within a face. */
export type ViewId = string;

/** One coalesced panel-tap event. */
export interface TapEvent {
  taps: number;
  point: { x: number; y: number } | null;
}

/** What a face is told beyond its settings: where it left off, and why it is drawing now. */
export interface RenderContext {
  /** Whatever this face returned as `state` last time, or undefined. */
  state?: unknown;
  /**
   * Absent for a scheduled refresh. `taps` is how many taps this render answers --
   * they coalesce, so three quick taps can arrive as one render with taps: 3.
   * `point` is always null until the wire carries one (C2); a face that wants to
   * hit-test gets the point in the same 448x368 space it drew in.
   */
  event?: TapEvent;
  /** Which view to draw. Absent means the resting view, which is views()[0]. */
  view?: ViewId;
  /**
   * The owner's IANA zone, when the caller has one to send. Absent for an older
   * caller (additive) or when the caller has none configured; a face that needs a
   * zone -- currently only a plugin, via `runPlugin` -- falls back to the host
   * process's own zone and then to UTC. The server sending the owner's configured
   * zone here, rather than the host's, is still owed.
   */
  timezone?: string;
}

/** A face returns a document, or a document plus the state it wants back next time. */
export type RenderResult = string | { svg: string; state?: unknown };

export interface FaceDefinition {
  /** The stable identifier stored in `data-cards.json`. */
  kind: string;
  /** What the add menu calls it. */
  label: string;
  fields: FieldSpec[];
  /** What the window tells the owner a tap does. A face without it ignores taps. */
  tap?: string;
  /**
   * The views this face offers, in priority order; the first is the resting view.
   * Pure: no fetch, no draw. A face without it has exactly one view and keeps
   * today's behaviour.
   */
  views?(settings: Settings, state?: unknown): ViewId[];
  /**
   * Which view a tap selects, and the state to store. Pure: no fetch, no draw,
   * and no clock of its own -- `now` is passed in, so a face that stamps when it
   * was tapped can be tested at a fixed time and cannot disagree with the `now`
   * that `render` later compares that stamp against.
   * A face with views() must have this, and the reverse.
   */
  onTap?(
    settings: Settings,
    state: unknown,
    event: TapEvent,
    now: Date,
  ): { view: ViewId; state?: unknown };
  /**
   * How often this face wants pushing, in seconds. Only a plugin sets this today, from
   * its manifest; a built-in face is silent and the server's own default applies. Read
   * by the server ONLY when creating a new card's spec (`data_cards.rs`, Task 10) --
   * changing a plugin's declared cadence after a card already exists does not move it.
   */
  refreshSeconds?: number;
  /** Fetches and draws. Resolves to the SVG document, optionally with new state. */
  render(
    settings: Settings,
    now: Date,
    context?: RenderContext,
  ): Promise<RenderResult> | RenderResult;
}

/**
 * The owner has to change a setting before this can work: retrying will not help.
 * A denied address is one of these, not an I/O error, because nothing about it
 * changes on a retry -- the message sends the owner to the URL they typed.
 */
export class ConfigurationError extends Error {
  override name = "ConfigurationError";
}

/** The world did not cooperate this time. The stored frame stays; try again later. */
export class TransientError extends Error {
  override name = "TransientError";
}

/** A setting as trimmed text, or "" when absent or not a string. */
export function text(settings: Settings, key: string): string {
  const value = settings[key];
  return typeof value === "string" ? value.trim() : "";
}

/** Truncates to at most `bytes` UTF-8 bytes without splitting a code point. */
export function truncateUtf8(value: string, bytes: number): string {
  const encoder = new TextEncoder();
  let used = 0;
  let result = "";
  for (const character of value) {
    used += encoder.encode(character).length;
    if (used > bytes) {
      break;
    }
    result += character;
  }
  return result;
}
