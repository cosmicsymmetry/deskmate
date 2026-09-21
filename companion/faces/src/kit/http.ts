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

/**
 * One request to `address`, speaking for `url`'s host. The URL handed to `fetch`
 * carries the IP literal, so nothing resolves the name again; the name travels in
 * `Host` and as the TLS server name, which is what the certificate is checked against.
 */
export function dial(url: URL, address: string, signal: AbortSignal): Promise<Response> {
  const target = new URL(url);
  target.hostname = isIP(address) === 6 ? `[${address}]` : address;
  const named = isIP(url.hostname.replace(/^\[|\]$/g, "")) === 0;
  return fetch(target, {
    redirect: "manual",
    signal,
    headers: { Host: url.host, "User-Agent": USER_AGENT, Accept: "*/*" },
    // An IP literal has no server name to present; a name always does.
    ...(url.protocol === "https:" && named ? { tls: { serverName: url.hostname } } : {}),
  } as RequestInit);
}

/** For tests: a face takes its fetcher as a parameter, and `fetchText` is the real one. */
export type FetchText = (url: string) => Promise<string>;

export function createFetchText(resolve: Resolve = systemResolve): FetchText {
  return async (address) => {
    let url: URL;
    try {
      url = new URL(address);
    } catch {
      throw new ConfigurationError(`${address} is not a URL`);
    }
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

async function cappedText(response: Response): Promise<string> {
  const reader = response.body?.getReader();
  if (reader === undefined) {
    return "";
  }
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    total += value.length;
    if (total > MAX_BODY_BYTES) {
      await reader.cancel();
      throw new TransientError("the response is too large");
    }
    chunks.push(value);
  }
  return new TextDecoder("utf-8").decode(Buffer.concat(chunks));
}
