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

export interface FaceDefinition {
  /** The stable identifier stored in `data-cards.json`. */
  kind: string;
  /** What the add menu calls it. */
  label: string;
  fields: FieldSpec[];
  /** Fetches and draws. Resolves to the SVG document. */
  render(settings: Settings, now: Date): Promise<string>;
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
