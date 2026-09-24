// The one way a face reaches the network.
//
// The Rust server's egress guard does not cover a subprocess, so its properties are
// restated here: http(s) only; no private, loopback, link-local, CGNAT or metadata
// destination; the check repeated on every redirect hop, because a public feed can
// redirect inward; a body cap; a wall-clock budget.
//
// And the resolved address is PINNED. A name is resolved once, every address it
// returned is validated, and the request then dials one of those addresses as an IP
// literal -- with the original name in the `Host` header and as the TLS server name,
// so the certificate is still checked against the name. Validating a name and then
// handing the same name to `fetch` would resolve it a second time, and a hostile DNS
// server answers the two lookups differently (rebinding): public for the check,
// 192.168.x.x for the connection. This host is a homelab VM with neighbours worth
// protecting, and a feed URL is only as trustworthy as the feed's DNS.

import { lookup } from "node:dns/promises";
import { isIP } from "node:net";
import { ConfigurationError, TransientError } from "../face";

/**
 * `fetch` sends no useful User-Agent by default, and a Cloudflare-fronted API answers
 * that with 403 before it reads the path. One name for every request.
 */
export const USER_AGENT = "deskmate-faces/1";

const MAX_BODY_BYTES = 1_048_576;
const TIMEOUT_MS = 15_000;
const MAX_REDIRECTS = 4;

export function isPrivateAddress(address: string): boolean {
  if (isIP(address) === 4) {
    const [a = 0, b = 0] = address.split(".").map(Number);
    return (
      a === 0 ||
      a === 10 ||
      a === 127 ||
      (a === 100 && b >= 64 && b <= 127) ||
      (a === 169 && b === 254) ||
      (a === 172 && b >= 16 && b <= 31) ||
      (a === 192 && b === 168) ||
      a >= 224
    );
  }
  const lower = address.toLowerCase();
  if (lower.startsWith("::ffff:")) {
    return isPrivateAddress(lower.slice(7));
  }
  return (
    lower === "::" ||
    lower === "::1" ||
    lower.startsWith("fc") ||
    lower.startsWith("fd") ||
    lower.startsWith("fe8") ||
    lower.startsWith("fe9") ||
    lower.startsWith("fea") ||
    lower.startsWith("feb") ||
    lower.startsWith("ff")
  );
}

/** Every address a name resolves to. Injectable so the refusals are testable offline. */
export type Resolve = (host: string) => Promise<string[]>;

const systemResolve: Resolve = async (host) =>
  (await lookup(host, { all: true })).map((entry) => entry.address);

/**
 * The one address this hop will dial. EVERY resolved address must be public -- a name
 * that answers with one public and one private address is refused outright rather
 * than trusted to pick the good one.
 */
export async function pinnedAddress(url: URL, resolve: Resolve = systemResolve): Promise<string> {
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new ConfigurationError(`${url.protocol} URLs are not fetched; use http or https`);
  }
  const host = url.hostname.replace(/^\[|\]$/g, "");
  let addresses: string[];
  if (isIP(host) !== 0) {
    addresses = [host];
  } else {
    try {
      addresses = await resolve(host);
    } catch {
      throw new TransientError(`${host} did not resolve`);
    }
  }
  if (addresses.length === 0) {
    throw new TransientError(`${host} did not resolve`);
  }
  if (addresses.some(isPrivateAddress)) {
    throw new ConfigurationError(`${host} is a private or local address and is not fetched`);
  }
  // IPv4 first: the deployment host has no IPv6 route, and a v6-first pin would turn
  // every dual-stack API into a timeout.
  return addresses.find((address) => isIP(address) === 4) ?? addresses[0] ?? host;
}

/** A request URL that fails to parse is always the owner's mistake, never transient. */
function parseUrl(address: string): URL {
  try {
    return new URL(address);
  } catch {
    throw new ConfigurationError(`${address} is not a URL`);
  }
}

/** The extra request shape `dial` accepts on top of the address it is told to hit. */
export type DialInit = { method?: string; headers?: Record<string, string>; body?: string };

/**
 * Header names the guard sets itself and a caller must never be able to take back, matched
 * case-insensitively. `Host` is the one that matters: address pinning validates the IP and
 * TLS SNI is computed from `url.hostname`, so a caller-supplied `Host` cannot redirect the
 * socket or defeat certificate checking -- but it CAN reach a validated, pinned public IP
 * while claiming to be an arbitrary internal hostname, which is exactly the primitive a
 * Host-routed internal service is vulnerable to. Caller headers with one of these names
 * (in any casing) are dropped before the guard's own headers are added, not merged with
 * them.
 */
const RESERVED_HEADERS = new Set(["host"]);

function callerHeaders(headers: Record<string, string> | undefined): Record<string, string> {
  if (headers === undefined) {
    return {};
  }
  const allowed: Record<string, string> = {};
  for (const [name, value] of Object.entries(headers)) {
    if (!RESERVED_HEADERS.has(name.toLowerCase())) {
      allowed[name] = value;
    }
  }
  return allowed;
}

/**
 * One request to `address`, speaking for `url`'s host. The URL handed to `fetch`
 * carries the IP literal, so nothing resolves the name again; the name travels in
 * `Host` and as the TLS server name, which is what the certificate is checked against.
 */
export function dial(
  url: URL,
  address: string,
  signal: AbortSignal,
  init?: DialInit,
): Promise<Response> {
  const target = new URL(url);
  target.hostname = isIP(address) === 6 ? `[${address}]` : address;
  const named = isIP(url.hostname.replace(/^\[|\]$/g, "")) === 0;
  return fetch(target, {
    method: init?.method,
    body: init?.body,
    redirect: "manual",
    signal,
    headers: {
      ...callerHeaders(init?.headers),
      // The guard's own headers are added last, and `Host` was already stripped out of
      // whatever the caller sent -- so nothing above this line can win.
      Host: url.host,
      "User-Agent": USER_AGENT,
      Accept: "*/*",
    },
    // An IP literal has no server name to present; a name always does.
    ...(url.protocol === "https:" && named ? { tls: { serverName: url.hostname } } : {}),
  } as RequestInit);
}

/**
 * `dial`'s shape, injectable the same way `Resolve` already is: `createRequest` takes one
 * as an optional second argument, defaulting to the real `dial`, so a test can prove the
 * guard runs (or is bypassed by a private address) before the transport is ever reached --
 * without a production flag that could be set outside a test.
 */
export type DialFn = (
  url: URL,
  address: string,
  signal: AbortSignal,
  init?: DialInit,
) => Promise<Response>;

/** For tests: a face takes its fetcher as a parameter, and `fetchText` is the real one. */
export type FetchText = (url: string) => Promise<string>;

export function createFetchText(resolve: Resolve = systemResolve): FetchText {
  return async (address) => {
    let url: URL = parseUrl(address);
    const deadline = AbortSignal.timeout(TIMEOUT_MS);
    try {
      for (let hop = 0; hop <= MAX_REDIRECTS; hop += 1) {
        const response = await dial(url, await pinnedAddress(url, resolve), deadline);
        if (response.status >= 300 && response.status < 400) {
          const location = response.headers.get("location");
          if (location === null) {
            throw new TransientError(`${url.host} redirected without a location`);
          }
          // Resolved against the NAMED url, never the dialled one: a relative
          // redirect must stay on the host, not on its IP literal.
          url = new URL(location, url);
          continue;
        }
        if (!response.ok) {
          throw new TransientError(`${url.host} returned HTTP ${response.status}`);
        }
        return await cappedText(response);
      }
      throw new TransientError("the redirect limit was exceeded");
    } catch (error) {
      if (error instanceof ConfigurationError || error instanceof TransientError) {
        throw error;
      }
      if (
        error instanceof Error &&
        (error.name === "TimeoutError" || error.name === "AbortError")
      ) {
        throw new TransientError(`${url.host} timed out`);
      }
      // The message, never the URL: a query string can carry an API key.
      throw new TransientError(`${url.host} could not be fetched`);
    }
  };
}

export const fetchText: FetchText = createFetchText();

/** The response body, read up to `max` bytes; over that, the owner never sees the rest. */
async function readCapped(response: Response, max: number): Promise<Uint8Array> {
  const reader = response.body?.getReader();
  if (reader === undefined) {
    return new Uint8Array(0);
  }
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    total += value.length;
    if (total > max) {
      await reader.cancel();
      throw new TransientError("the response is too large");
    }
    chunks.push(value);
  }
  return Buffer.concat(chunks);
}

async function cappedText(response: Response): Promise<string> {
  return new TextDecoder("utf-8").decode(await readCapped(response, MAX_BODY_BYTES));
}

export interface HttpReply {
  status: number;
  body: string | Uint8Array;
  json?: unknown;
}

export interface HttpRequest {
  url: string;
  method?: "GET" | "POST";
  headers?: Record<string, string>;
  body?: string;
  as: "json" | "text" | "bytes";
}

export type RequestFn = (input: HttpRequest) => Promise<HttpReply>;

/**
 * `dial`'s response, decoded the way `as` asked. Exported alongside `dial` for tests: a
 * request that must actually complete against a real server cannot be driven through
 * `createRequest` itself, because that would mean pinning a loopback address, which the
 * guard exists to refuse -- exactly like this file's existing `dial`-level tests, which
 * hit a real local server without going through `pinnedAddress`.
 */
export async function replyFrom(response: Response, as: HttpRequest["as"]): Promise<HttpReply> {
  const bytes = await readCapped(response, MAX_BODY_BYTES);
  if (as === "bytes") {
    return { status: response.status, body: bytes };
  }
  const text = new TextDecoder("utf-8").decode(bytes);
  if (as === "text") {
    return { status: response.status, body: text };
  }
  try {
    return { status: response.status, body: text, json: JSON.parse(text) };
  } catch {
    return { status: response.status, body: text };
  }
}

/**
 * A single request behind the same guard as `fetchText`: address pinning, the
 * `Host`/TLS-name split and the body cap are all `pinnedAddress`/`dial`/`readCapped`,
 * unchanged. Unlike `fetchText` this does not follow redirects and does not throw on a
 * non-ok status -- a plugin's 404 or redirect is the plugin's business to interpret.
 *
 * `dialFn` defaults to the real `dial`; only a test passes anything else, which is what
 * makes it safe to inject -- there is no flag to misconfigure in production.
 */
export function createRequest(resolve: Resolve = systemResolve, dialFn: DialFn = dial): RequestFn {
  return async ({ url, method = "GET", headers = {}, body, as }) => {
    const target = parseUrl(url);
    try {
      const address = await pinnedAddress(target, resolve);
      const response = await dialFn(target, address, AbortSignal.timeout(TIMEOUT_MS), {
        method,
        headers,
        body,
      });
      return await replyFrom(response, as);
    } catch (error) {
      if (error instanceof ConfigurationError || error instanceof TransientError) {
        throw error;
      }
      if (
        error instanceof Error &&
        (error.name === "TimeoutError" || error.name === "AbortError")
      ) {
        throw new TransientError(`${target.host} timed out`);
      }
      // The message, never the URL: a query string can carry an API key.
      throw new TransientError(`${target.host} could not be fetched`);
    }
  };
}

export const request: RequestFn = createRequest();
