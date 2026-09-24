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

/**
 * A secret is substituted ONLY into a request whose host is that secret's own.
 * An unmatched placeholder stays as literal text: leaking the value is the one
 * outcome that must never happen, and a visibly wrong header is debuggable.
 */
function applySecrets(
  target: URL,
  headers: Record<string, string>,
  manifest: PluginManifest,
  secrets: Record<string, string>,
): Record<string, string> {
  const host = target.hostname.toLowerCase();
  const out: Record<string, string> = {};
  for (const [name, value] of Object.entries(headers)) {
    out[name] = value.replace(PLACEHOLDER, (literal, key: string) => {
      const spec = manifest.secrets.find((secret) => secret.key === key);
      if (spec === undefined || spec.host.toLowerCase() !== host) return literal;
      const stored = secrets[key];
      if (stored === undefined) return literal;
      return spec.send_as === "bearer" ? `Bearer ${stored}` : stored;
    });
  }
  return out;
}

/**
 * The same substitution rule as `applySecrets`, applied to the request's own query
 * parameters instead of its headers -- a plugin may write `{{secret:key}}` into
 * either. Same host check, same "unmatched stays literal" rule.
 */
function applySecretsToQuery(
  target: URL,
  manifest: PluginManifest,
  secrets: Record<string, string>,
): URL {
  const host = target.hostname.toLowerCase();
  const substituted = new URL(target);
  const params = new URLSearchParams();
  for (const [key, value] of target.searchParams.entries()) {
    params.append(
      key,
      value.replace(PLACEHOLDER, (literal, secretKey: string) => {
        const spec = manifest.secrets.find((secret) => secret.key === secretKey);
        if (spec === undefined || spec.host.toLowerCase() !== host) return literal;
        const stored = secrets[secretKey];
        if (stored === undefined) return literal;
        return spec.send_as === "bearer" ? `Bearer ${stored}` : stored;
      }),
    );
  }
  substituted.search = params.toString();
  return substituted;
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

function toAnswer(reply: { status: number; body: string | Uint8Array; json?: unknown }): Answer {
  if (reply.body instanceof Uint8Array) {
    return { ok: true, status: reply.status, base64: Buffer.from(reply.body).toString("base64") };
  }
  if (reply.json !== undefined) {
    return { ok: true, status: reply.status, json: reply.json };
  }
  return { ok: true, status: reply.status, text: reply.body };
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
      answers.push({ ok: false, error: "the byte budget for this refresh is spent" });
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
      answers.push(toAnswer(reply));
    } catch (error) {
      answers.push({ ok: false, error: error instanceof Error ? error.message : String(error) });
    }
  }
  return answers;
}
