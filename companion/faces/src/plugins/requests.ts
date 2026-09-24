// What a plugin is allowed to reach, and with which credential.
//
// A plugin never performs a request itself: it declares one (a URL, headers, what to
// do with the body), this module validates that declaration against the plugin's own
// manifest, substitutes the owner's stored secrets, and only then performs it through
// the package's guarded fetch (`RequestFn`, `src/kit/http.ts`). The plugin never sees
// a secret value -- it sees the placeholder it wrote, or nothing.
//
// Two rules carry the security weight here:
//   - A request may only go to a host the manifest declared, matched EXACTLY on the
//     lowercased hostname -- no suffix matching, no wildcards -- and an IP-literal host
//     is refused here too, even though the manifest parser already refuses one in
//     `hosts`. This check must not depend on another file having been careful.
//   - A secret is substituted ONLY when the request's host equals that secret's own
//     registered host. An unmatched placeholder stays as literal text. Leaking a value
//     is the one outcome that must never happen.

import { isIP } from "node:net";
import { ConfigurationError } from "../face";
import type { HttpRequest, RequestFn } from "../kit/http";
import type { PluginManifest } from "./manifest";

export interface Budget {
  requests: number;
  bytes: number;
  measurements: number;
}

export type Answer =
  | { ok: true; status: number; json?: unknown; text?: string; base64?: string }
  | { ok: false; status?: number; error: string };

/** A plugin's own declaration of one request. Validated, never trusted as-is. */
export interface PluginRequest {
  url: string;
  method?: "GET" | "POST";
  headers?: Record<string, string>;
  body?: string;
  as: "json" | "text" | "bytes";
}

const PLACEHOLDER = /\{\{secret:([a-z0-9_]+)\}\}/gi;
/** Existence check only (no capture group iteration state to corrupt): is there a
 * placeholder anywhere in this string at all? Kept separate from `PLACEHOLDER`
 * because that regex is `g` and `.test()` on a shared global regex would advance
 * (and eventually wrap) its `lastIndex` across unrelated calls. */
const HAS_PLACEHOLDER = /\{\{secret:[a-z0-9_]+\}\}/i;

/** Where a placeholder was written: a header value, or a query-parameter value. */
type Position = "header" | "query";

/**
 * The one place the substitution rule lives, for both a header value and a query
 * value: a secret is substituted ONLY when (a) the request's host equals that
 * secret's own registered host, AND (b) `position` matches where its manifest
 * `send_as` says it belongs -- `"bearer"`/`"header"` substitute only in a header,
 * `"query"` substitutes only in a query parameter. A placeholder that fails either
 * check stays as literal text: an unmatched placeholder is visible and debuggable,
 * and leaking the value -- or silently relocating it to a URL that gets logged,
 * cached and forwarded in a Referer header -- is the one outcome that must never
 * happen.
 */
function resolveSecret(
  key: string,
  host: string,
  position: Position,
  manifest: PluginManifest,
  secrets: Record<string, string>,
): string | undefined {
  const spec = manifest.secrets.find((secret) => secret.key === key);
  if (spec === undefined || spec.host.toLowerCase() !== host) return undefined;
  const belongsInQuery = spec.send_as === "query";
  if (position === "query" ? !belongsInQuery : belongsInQuery) return undefined;
  const stored = secrets[key];
  if (stored === undefined) return undefined;
  return spec.send_as === "bearer" ? `Bearer ${stored}` : stored;
}

/** Substitutes `{{secret:<key>}}` in each header value, via `resolveSecret`. */
function applySecrets(
  target: URL,
  headers: Record<string, string>,
  manifest: PluginManifest,
  secrets: Record<string, string>,
): Record<string, string> {
  const host = target.hostname.toLowerCase();
  const out: Record<string, string> = {};
  for (const [name, value] of Object.entries(headers)) {
    out[name] = value.replace(
      PLACEHOLDER,
      (literal, key: string) => resolveSecret(key, host, "header", manifest, secrets) ?? literal,
    );
  }
  return out;
}

/**
 * The same substitution rule, applied to the request's own query parameters instead
 * of its headers -- a plugin may write `{{secret:key}}` into either, and `send_as`
 * decides which one actually substitutes. A request whose query has no placeholder
 * at all is returned untouched: rebuilding through `URLSearchParams` re-encodes
 * (`+` for space, its own escaping of reserved characters), and a request that never
 * touches a secret must reach the wire exactly as the plugin wrote it.
 */
function applySecretsToQuery(
  target: URL,
  manifest: PluginManifest,
  secrets: Record<string, string>,
): URL {
  if (!HAS_PLACEHOLDER.test(target.search)) return target;
  const host = target.hostname.toLowerCase();
  const substituted = new URL(target);
  const params = new URLSearchParams();
  for (const [key, value] of target.searchParams.entries()) {
    params.append(
      key,
      value.replace(
        PLACEHOLDER,
        (literal, secretKey: string) =>
          resolveSecret(secretKey, host, "query", manifest, secrets) ?? literal,
      ),
    );
  }
  substituted.search = params.toString();
  return substituted;
}

/**
 * Every outward-facing string this module returns -- an error message, and (below) a
 * `text`/`json` response body -- passes through here before it is stored in an
 * `Answer`. This does not trust a `RequestFn`, or the host it fetched, to have been
 * careful about what comes back: a value this call was handed is scrubbed regardless
 * of where it came from. That matters beyond a thrown error, because the module's
 * actual guarantee is that the plugin never sees the credential -- and a plugin does
 * not need a network failure to break that. A declared host that echoes request
 * details in an ordinary 200 (plenty of APIs reflect headers, or partially-mask a key
 * in a diagnostic body) is how a hostile plugin would actually try to read its user's
 * secret back out.
 *
 * What this does NOT catch, left as a documented gap rather than fixed, because
 * neither is reachable through the real fetch today: a percent-encoded rendering of a
 * secret (`ghp%5Fsecret`) does not exact-match the raw stored value, and a secret
 * split across non-adjacent text (chunked, or interleaved with other content) defeats
 * a substring search entirely. Closing either needs a decode-then-scan or a streaming
 * matcher, not a bigger regex -- revisit if a real face ever needs one. Do not read
 * this function as "no secret can appear in any form" on the strength of its name.
 */
function redactSecrets(text: string, secrets: Record<string, string>): string {
  let out = text;
  for (const stored of Object.values(secrets)) {
    if (stored === "") continue;
    out = out.split(stored).join("[redacted]");
  }
  return out;
}

/** `redactSecrets`, applied to every string leaf of a JSON value at any depth -- a
 * secret hiding two or three fields deep in a response must not escape a scan that
 * only looked at the top level. Numbers, booleans and `null` pass through unchanged. */
function redactJsonValue(value: unknown, secrets: Record<string, string>): unknown {
  if (typeof value === "string") return redactSecrets(value, secrets);
  if (Array.isArray(value)) return value.map((item) => redactJsonValue(item, secrets));
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [key, val] of Object.entries(value as Record<string, unknown>)) {
      out[key] = redactJsonValue(val, secrets);
    }
    return out;
  }
  return value;
}

/**
 * Scrubbing binary is meaningless, so a `bytes` answer is not redacted -- it is
 * refused outright when the raw bytes contain a stored secret's UTF-8 form, or its
 * base64 form (a common way a credential ends up embedded in a response, on purpose
 * or by accident). An honest refusal is better than a silent leak, and a legitimate
 * image or other binary payload never contains the user's own API key.
 */
function bytesContainSecret(bytes: Uint8Array, secrets: Record<string, string>): boolean {
  const haystack = Buffer.from(bytes);
  for (const stored of Object.values(secrets)) {
    if (stored === "") continue;
    if (haystack.includes(stored, 0, "utf-8")) return true;
    const asBase64 = Buffer.from(stored, "utf-8").toString("base64");
    if (haystack.includes(asBase64, 0, "utf-8")) return true;
  }
  return false;
}

/** Strips brackets from an IPv6 literal the way `URL.hostname` presents it. */
function bareHost(hostname: string): string {
  return hostname.replace(/^\[|\]$/g, "");
}

/**
 * The host check that stands between a plugin and the cloud-metadata address. It does
 * not trust the manifest parser to have been careful: an IP literal is refused here
 * too, and the match against `manifest.hosts` is exact on the lowercased hostname --
 * no suffix matching, no wildcards.
 */
function checkDeclaredHost(target: URL, manifest: PluginManifest): void {
  const raw = bareHost(target.hostname);
  if (isIP(raw) !== 0) {
    throw new ConfigurationError(
      `${raw} is an IP literal and is not a request this plugin may make`,
    );
  }
  const host = raw.toLowerCase();
  const allowed = new Set(manifest.hosts.map((declared) => declared.toLowerCase()));
  if (!allowed.has(host)) {
    throw new ConfigurationError(`${host} is not a host this plugin declared in its manifest`);
  }
}

function validateOne(raw: unknown, index: number): PluginRequest {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    throw new ConfigurationError(`request ${index} is not an object`);
  }
  const source = raw as Record<string, unknown>;

  const url = source.url;
  if (typeof url !== "string" || url.trim() === "") {
    throw new ConfigurationError(`request ${index} url must be a non-empty string`);
  }
  let target: URL;
  try {
    target = new URL(url);
  } catch {
    throw new ConfigurationError(`request ${index} url ${JSON.stringify(url)} is not a URL`);
  }
  if (target.protocol !== "http:" && target.protocol !== "https:") {
    throw new ConfigurationError(`${target.protocol} URLs are not fetched; use http or https`);
  }

  const as = source.as;
  if (as !== "json" && as !== "text" && as !== "bytes") {
    throw new ConfigurationError(`request ${index} as must be "json", "text", or "bytes"`);
  }

  const method = source.method;
  if (method !== undefined && method !== "GET" && method !== "POST") {
    throw new ConfigurationError(`request ${index} method must be "GET" or "POST"`);
  }

  const headers = source.headers;
  if (headers !== undefined) {
    if (typeof headers !== "object" || headers === null || Array.isArray(headers)) {
      throw new ConfigurationError(`request ${index} headers must be an object`);
    }
    for (const value of Object.values(headers as Record<string, unknown>)) {
      if (typeof value !== "string") {
        throw new ConfigurationError(`request ${index} header values must be strings`);
      }
    }
  }

  const body = source.body;
  if (body !== undefined && typeof body !== "string") {
    throw new ConfigurationError(`request ${index} body must be a string`);
  }

  return {
    url: target.toString(),
    as,
    ...(method !== undefined ? { method: method as "GET" | "POST" } : {}),
    ...(headers !== undefined ? { headers: headers as Record<string, string> } : {}),
    ...(body !== undefined ? { body } : {}),
  };
}

/**
 * Validates a plugin's declared requests against its own manifest and the sandbox's
 * budget: the shape of each request, the scheme, the request count, and -- the check
 * that matters -- that every host was declared. Throws `ConfigurationError` on the
 * first violation; the plugin never gets to perform a request this did not approve.
 */
export function validateRequests(
  raw: unknown,
  manifest: PluginManifest,
  budget: Budget,
): PluginRequest[] {
  if (!Array.isArray(raw)) {
    throw new ConfigurationError("requests must be an array");
  }
  if (raw.length > budget.requests) {
    throw new ConfigurationError(
      `a plugin may issue at most ${budget.requests} requests per refresh, not ${raw.length}`,
    );
  }
  return raw.map((item, index) => {
    const validated = validateOne(item, index);
    checkDeclaredHost(new URL(validated.url), manifest);
    return validated;
  });
}

function byteSize(body: string | Uint8Array): number {
  return typeof body === "string" ? Buffer.byteLength(body, "utf-8") : body.length;
}

/**
 * Turns a completed reply into an `Answer`, scrubbed. Every branch that can return
 * `ok: true` passes its payload through the redaction above first -- `toAnswer` is the
 * one place a reply becomes something the plugin can read, so it is the one place this
 * has to happen.
 */
function toAnswer(
  reply: { status: number; body: string | Uint8Array; json?: unknown },
  secrets: Record<string, string>,
): Answer {
  if (reply.body instanceof Uint8Array) {
    if (bytesContainSecret(reply.body, secrets)) {
      return {
        ok: false,
        status: reply.status,
        error: "the response echoed a stored credential and was refused",
      };
    }
    return { ok: true, status: reply.status, base64: Buffer.from(reply.body).toString("base64") };
  }
  if (reply.json !== undefined) {
    return { ok: true, status: reply.status, json: redactJsonValue(reply.json, secrets) };
  }
  return { ok: true, status: reply.status, text: redactSecrets(reply.body, secrets) };
}

/**
 * Performs every already-validated request: substitutes secrets (never handing the
 * plugin one back, win or lose), calls the guarded `request`, and turns a failure of
 * any single request into an `Answer` rather than an exception -- the plugin decides
 * what a failed request means. Only a violation of the contract itself (the byte
 * budget being spent) short-circuits a later request without performing it.
 */
export async function performRequests(
  requests: PluginRequest[],
  manifest: PluginManifest,
  secrets: Record<string, string>,
  request: RequestFn,
  budget: Budget,
): Promise<Answer[]> {
  const answers: Answer[] = [];
  let bytesLeft = budget.bytes;
  for (const declared of requests) {
    if (bytesLeft <= 0) {
      answers.push({
        ok: false,
        error: redactSecrets("the byte budget for this refresh is spent", secrets),
      });
      continue;
    }
    const target = new URL(declared.url);
    const headers = applySecrets(target, declared.headers ?? {}, manifest, secrets);
    const substituted = applySecretsToQuery(target, manifest, secrets);
    try {
      const input: HttpRequest = {
        url: substituted.toString(),
        as: declared.as,
        ...(declared.method !== undefined ? { method: declared.method } : {}),
        headers,
        ...(declared.body !== undefined ? { body: declared.body } : {}),
      };
      const reply = await request(input);
      bytesLeft -= byteSize(reply.body);
      answers.push(toAnswer(reply, secrets));
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      answers.push({ ok: false, error: redactSecrets(message, secrets) });
    }
  }
  return answers;
}
