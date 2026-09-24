// The rounds: `plan` -> validate -> perform -> `plan` again, up to three times, then
// `render` once, then the card becomes the SVG the server pushes to a picture card.
// Everything else under `src/plugins/` is a piece (the manifest, the sandbox, local
// time, the fetch/measure guard, the card converter); this file is the function that
// assembles them into the pure-function contract
// `docs/superpowers/specs/2026-09-23-deskmate-plugin-contract-design.md` describes: a
// plugin never calls anything itself, it only declares what it needs, and the host
// performs it.

import { ConfigurationError, type Settings, TransientError } from "../face";
import type { RequestFn } from "../kit/http";
import { CardError, cardToSvg } from "./card";
import { FORMAT_SOURCE, type NowContext, buildNow } from "./context";
import type { PluginManifest } from "./manifest";
import {
  type Answer,
  type Budget,
  type PluginMeasureRequest,
  type ValidatedRequest,
  performRequests,
  validateRequests,
} from "./requests";
import { SandboxError, runInSandbox } from "./sandbox";

/** A tap, coalesced by the host: three quick taps can arrive as one render with
 * `taps: 3`. `point` is always `null` until a later track carries one. */
export interface TapEvent {
  taps: number;
  point: { x: number; y: number } | null;
}

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
export interface RenderContext {
  settings: Settings;
  now: NowContext;
  state?: unknown;
  event?: TapEvent;
  answers: Answer[];
}

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
const MAX_BYTES = 4 * 1024 * 1024;
const MAX_MEASUREMENTS = 64;

/** Same table: state returned beside a card is capped at 16 KB encoded. */
const STATE_CAP_BYTES = 16 * 1024;

/** A plugin's own `log` array, capped the way the spec requires: at most 10 lines of
 * 200 characters each. A host-generated notice (the state cap, below) is appended
 * after this cap runs, because the cap is on what the plugin wrote, not on the total. */
const LOG_MAX_LINES = 10;
const LOG_MAX_CHARS = 200;

function isMeasure(request: ValidatedRequest): request is PluginMeasureRequest {
  return "measure" in request;
}

/** How many response bytes one answer actually spent, for the cross-round budget.
 * A measure answer performed no I/O and spent none of it. */
function answerBytes(answer: Answer): number {
  if ("measurements" in answer) return 0;
  if (!answer.ok) return 0;
  if (answer.base64 !== undefined) return Buffer.byteLength(answer.base64, "base64");
  if (answer.json !== undefined) return Buffer.byteLength(JSON.stringify(answer.json), "utf8");
  if (answer.text !== undefined) return Buffer.byteLength(answer.text, "utf8");
  return 0;
}

/**
 * The error taxonomy this task exists to build. `SandboxError` covers three distinct
 * situations under one type -- the deadline fired, the memory cap fired, or the
 * plugin's own `throw` -- and the only signal that survives to tell them apart is
 * `configuration`, set true only when the plugin's own thrown error carried a truthy
 * `configuration` property. Everything else a sandboxed call can fail with (a timeout,
 * an allocation stop, an ordinary `throw new Error(...)`) is worth retrying: the owner
 * did not do anything the render can name and ask them to fix.
 */
function classifySandboxError(error: SandboxError): ConfigurationError | TransientError {
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

function capLog(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .filter((line): line is string => typeof line === "string")
    .slice(0, LOG_MAX_LINES)
    .map((line) => line.slice(0, LOG_MAX_CHARS));
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

  for (let round = 0; round < MAX_ROUNDS; round += 1) {
    const planContext: PlanContext = {
      settings: input.settings,
      now,
      ...(input.state === undefined ? {} : { state: input.state }),
      ...(input.event === undefined ? {} : { event: input.event }),
      answers: [...answers],
    };
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

    const performed = await performRequests(
      validated,
      input.manifest,
      input.secrets,
      requestFn,
      budget,
    );
    answers.push(...performed);

    remainingRequests -= validated.filter((request) => !isMeasure(request)).length;
    remainingMeasurements -= validated
      .filter(isMeasure)
      .reduce((sum, request) => sum + request.measure.length, 0);
    remainingBytes -= performed.reduce((sum, answer) => sum + answerBytes(answer), 0);
  }

  const renderContext: RenderContext = {
    settings: input.settings,
    now,
    ...(input.state === undefined ? {} : { state: input.state }),
    ...(input.event === undefined ? {} : { event: input.event }),
    answers,
  };
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
    if (error instanceof CardError) throw new ConfigurationError(error.message);
    throw error;
  }

  const log = capLog(record.log);
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
