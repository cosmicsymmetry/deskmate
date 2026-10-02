// The rounds: `plan` -> validate -> perform -> `plan` again, up to three times, then
// `render` once, then the card becomes the SVG the server pushes to a picture card.
// Everything else under `src/plugins/` is a piece (the manifest, the sandbox, local
// time, the fetch/measure guard, the card converter); this file is the function that
// assembles them into the pure-function contract
// `docs/superpowers/specs/2026-09-23-deskmate-plugin-contract-design.md` describes: a
// plugin never calls anything itself, it only declares what it needs, and the host
// performs it.

import { ConfigurationError, type Settings, type TapEvent, TransientError } from "../face";
import type { RequestFn } from "../kit/http";
import { INGEST_CAP_BYTES } from "../kit/limits";
import { CardError, cardToSvg } from "./card";
import { FORMAT_SOURCE, type NowContext, buildNow } from "./context";
import type { PluginManifest } from "./manifest";
import {
  type Answer,
  type Budget,
  isMeasureRequest,
  performRequests,
  validateRequests,
} from "./requests";
import { SandboxError, runInSandbox } from "./sandbox";

/** What `plan` is handed. `answers` accumulates across rounds -- empty on the first
 * call, growing on the second and third -- which is what lets a plugin that got what
 * it needed on round one return `[]` and skip the rest, and what lets a paging plugin
 * decide what to fetch next from where it left off. */
export interface PlanContext {
  settings: Settings;
  now: NowContext;
  state?: unknown;
  event?: TapEvent;
  answers: Answer[];
}

/** What `render` is handed: the same context, with every answer collected across
 * every round that ran. */
export type RenderContext = PlanContext;

export interface RunPluginInput {
  manifest: PluginManifest;
  source: string;
  settings: Settings;
  now: Date;
  timezone: string;
  state?: unknown;
  event?: TapEvent;
  secrets: Record<string, string>;
  /** Absent only when a plugin is known to make no network request at all -- a
   * request declared with none supplied fails that one request, not the whole run. */
  request?: RequestFn;
  /** Host diagnostics, including notices before a later render failure. */
  onNotice?: (message: string) => void;
}

export interface RunPluginResult {
  svg: string;
  state?: unknown;
  log: string[];
}

/** `docs/superpowers/specs/2026-09-23-deskmate-plugin-contract-design.md`'s sandbox
 * table: at most 3 rounds of `plan`, 8 requests, 4 MB of response body and 64
 * measurements PER RENDER -- not per round. The budget below is spent across rounds,
 * not reset each time `plan` runs again. */
const MAX_ROUNDS = 3;
const MAX_REQUESTS = 8;
// Four times the ingest cap, and deliberately tighter than 8 requests x that cap --
// the spec's own "N MB total, tighter than count x per-item" pattern. Expressed in
// terms of the shared constant so a change to the ingest cap moves all three together.
const MAX_BYTES = 4 * INGEST_CAP_BYTES;
const MAX_MEASUREMENTS = 64;

/** Same table: state returned beside a card is capped at 16 KB encoded. */
const STATE_CAP_BYTES = 16 * 1024;

/** A plugin's own `log` array, capped the way the spec requires: at most 10 lines of
 * 200 characters each. A host-generated notice (the state cap, below) is appended
 * after this cap runs, because the cap is on what the plugin wrote, not on the total. */
const LOG_MAX_LINES = 10;
const LOG_MAX_CHARS = 200;

/**
 * The error taxonomy this task exists to build. `SandboxError` covers several distinct
 * situations under one type -- the deadline fired, the memory cap fired, the host
 * refused to run the plugin at all, or the plugin's own `throw` -- and the one signal
 * that survives to tell them apart is `configuration`, which means DETERMINISTIC: the
 * same refusal next minute, so the owner must be told rather than the render quietly
 * repeated forever. Everything else a sandboxed call can fail with (a timeout, an
 * allocation stop, an ordinary `throw new Error(...)`) is worth retrying: the owner
 * did not do anything the render can name and ask them to fix.
 */
function classifySandboxError(error: SandboxError): ConfigurationError | TransientError {
  // `SandboxError.configuration` now means "deterministic", set by the plugin's own
  // thrown object AND by every host-side refusal that cannot come good on a retry
  // (see that class's comment). Everything else -- the deadline, the memory cap, an
  // ordinary throw -- stays transient.
  return error.configuration
    ? new ConfigurationError(error.message)
    : new TransientError(error.message);
}

function invokeSandbox<T>(source: string, fn: "plan" | "render", context: unknown): T {
  try {
    return runInSandbox<T>(source, fn, context);
  } catch (error) {
    if (error instanceof SandboxError) throw classifySandboxError(error);
    throw error;
  }
}

/**
 * An ANSI escape sequence: CSI (`ESC [ ... final`), OSC (`ESC ] ... BEL`/`ST`) and the
 * short two-character forms. A plugin's log line reaches OUR stderr, and a terminal
 * reading it acts on these -- colour is the harmless end of a range that runs through
 * cursor movement to overwriting lines already printed. Stripped whole, so no orphaned
 * introducer is left behind for the control-character pass below to half-remove.
 */
// biome-ignore lint/suspicious/noControlCharactersInRegex: matching control characters is the point
const ANSI = /\u001b(?:\[[0-?]*[ -/]*[@-~]|\][^\u0007\u001b]*(?:\u0007|\u001b\\)?|[@-Z\\-_])/g;
/** C0 (including newline and tab), DEL, and C1. */
// biome-ignore lint/suspicious/noControlCharactersInRegex: matching control characters is the point
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/g;

/**
 * A plugin's log line, "capped AND sanitized" as the spec puts it. Capping alone was
 * not enough: the line is written to our stderr, where a newline forges a second log
 * entry and an escape sequence drives the reader's terminal. Sanitized first, then
 * capped, so truncation cannot leave a half-sequence behind.
 */
function capLog(raw: unknown): { lines: string[]; overLines: boolean; overChars: boolean } {
  const strings = Array.isArray(raw)
    ? raw.filter((line): line is string => typeof line === "string")
    : [];
  const lines: string[] = [];
  let overChars = false;
  for (const [index, line] of strings.entries()) {
    const sanitized = line.replace(ANSI, "").replace(CONTROL, " ").trim();
    if (sanitized.length > LOG_MAX_CHARS) overChars = true;
    if (index < LOG_MAX_LINES) lines.push(sanitized.slice(0, LOG_MAX_CHARS));
  }
  return { lines, overLines: strings.length > LOG_MAX_LINES, overChars };
}

/**
 * Runs one plugin's contract end to end: up to three rounds of `plan` (each validated
 * against the manifest's declared hosts and the render's budget, then performed), one
 * `render`, then the card conversion. `FORMAT_SOURCE` is prepended to the plugin's own
 * source before EVERY sandbox evaluation -- `plan` and `render` alike -- because the
 * sandbox has no `Intl` and a plugin that formats a number or a date needs `format` to
 * exist as an ordinary global, not something it has to ask the host for.
 *
 * Every failure this function can raise is exactly one of `ConfigurationError` (the
 * owner must change something; retrying cannot help) or `TransientError` (worth
 * retrying). Nothing else escapes: a `CardError` becomes a `ConfigurationError`, a
 * `SandboxError` becomes whichever of the two `classifySandboxError` decides, and an
 * allowlist refusal from `validateRequests` is already a `ConfigurationError` and is
 * simply left to propagate.
 */
export async function runPlugin(input: RunPluginInput): Promise<RunPluginResult> {
  const source = `${FORMAT_SOURCE}\n${input.source}`;
  const now = buildNow(input.now, input.timezone);
  // A missing request function only matters if a round actually declares a network
  // request; `performRequests` catches whatever this throws and turns it into a
  // per-request `Answer { ok: false }` rather than failing the whole render, so this
  // is a safety net, not a path any of this function's own error handling needs to
  // classify.
  const requestFn: RequestFn =
    input.request ??
    (async () => {
      throw new Error(
        "this plugin declared a network request, but no request function was configured",
      );
    });

  const answers: Answer[] = [];
  let remainingRequests = MAX_REQUESTS;
  let remainingBytes = MAX_BYTES;
  let remainingMeasurements = MAX_MEASUREMENTS;
  const notices: string[] = [];
  const notice = (line: string) => {
    notices.push(line);
    input.onNotice?.(line);
  };

  const context = (answers: Answer[]): PlanContext => ({
    settings: input.settings,
    now,
    ...(input.state === undefined ? {} : { state: input.state }),
    ...(input.event === undefined ? {} : { event: input.event }),
    answers,
  });

  for (let round = 0; round < MAX_ROUNDS; round += 1) {
    const planContext = context([...answers]);
    const planned = invokeSandbox<unknown>(source, "plan", planContext);

    const budget: Budget = {
      requests: remainingRequests,
      bytes: remainingBytes,
      measurements: remainingMeasurements,
    };
    // Throws ConfigurationError when `planned` is not an array, when a request names
    // an undeclared host, or when this round's declarations exceed what is left of
    // the render's budget -- exactly the allowlist and shape refusals this function
    // does not need to reclassify.
    const validated = validateRequests(planned, input.manifest, budget);
    if (validated.length === 0) break;
    if (round === MAX_ROUNDS - 1) {
      notice(
        `Planning stopped at the ${MAX_ROUNDS}-round limit; return [] as soon as all answers are available.`,
      );
    }

    const performed = await performRequests(
      validated,
      input.manifest,
      input.secrets,
      requestFn,
      budget,
      notice,
    );
    answers.push(...performed.answers);

    remainingRequests -= validated.filter((request) => !isMeasureRequest(request)).length;
    remainingMeasurements -= validated
      .filter(isMeasureRequest)
      .reduce((sum, request) => sum + request.measure.length, 0);
    // The spend `performRequests` MEASURED, never a re-derivation from the answers:
    // an answer is redacted, refused or absent, and each of those understates what
    // crossed the wire -- see `PerformedRequests.bytesSpent`.
    remainingBytes -= performed.bytesSpent;
  }

  const renderContext = context(answers);
  const rendered = invokeSandbox<unknown>(source, "render", renderContext);
  const record =
    typeof rendered === "object" && rendered !== null ? (rendered as Record<string, unknown>) : {};

  let svg: string;
  try {
    // `rendered` itself, not `record` -- `cardToSvg` reads `layout`/`svg`/`png` off
    // whatever the plugin returned and is the one place that decides "this plugin
    // returned no card" is a CardError, not this function's problem to pre-empt.
    svg = await cardToSvg(rendered);
  } catch (error) {
    // `CardError.configuration` tells one of this file's own explicit cap/shape checks
    // (the plugin author's to fix) from a foreign satori exception we merely wrapped
    // (possibly ours). Mapping every CardError to ConfigurationError told the owner to
    // change a setting that could not help, and left the card waiting for an edit.
    if (error instanceof CardError) {
      throw error.configuration
        ? new ConfigurationError(error.message)
        : new TransientError(error.message);
    }
    throw error;
  }

  const { lines: log, overLines, overChars } = capLog(record.log);
  if (overLines) notice(`Plugin log was limited to ${LOG_MAX_LINES} lines.`);
  if (overChars) notice(`Plugin log lines were shortened to ${LOG_MAX_CHARS} characters.`);
  log.push(...notices);
  let state: unknown = record.state;
  if (state !== undefined) {
    const encoded = Buffer.byteLength(JSON.stringify(state), "utf8");
    if (encoded > STATE_CAP_BYTES) {
      state = undefined;
      log.push(`this plugin's state was ${encoded} bytes, over the 16 KB cap, and was not stored`);
    }
  }

  return { svg, ...(state === undefined ? {} : { state }), log };
}
