// Private server/package IPC, never the device wire. Only pure selection is
// exposed here: a render cannot occupy the selector's process or request queue.
import { ConfigurationError } from "./face";

export const MAX_REQUEST_BYTES = 64 * 1024;
export const MAX_RESPONSE_BYTES = 16 * 1024;
export const MAX_REQUESTS = 256;
export const MAX_RSS_BYTES = 128 * 1024 * 1024;
export const MAX_AGE_MS = 60_000;

export async function serveTapWorker(
  select: (input: string) => Promise<string>,
  limits = { requests: MAX_REQUESTS, rssBytes: MAX_RSS_BYTES, ageMs: MAX_AGE_MS },
): Promise<void> {
  // A faces-only rsync is picked up within the existing one-minute catalog
  // cadence, even with no taps. Do not retain request/state objects between calls.
  const lifetime = setTimeout(() => process.exit(0), limits.ageMs);
  let pending = Buffer.alloc(0);
  let requests = 0;
  try {
    for await (const chunk of Bun.stdin.stream()) {
      pending = Buffer.concat([pending, chunk]);
      while (true) {
        const newline = pending.indexOf(10);
        if (newline === -1) break;
        if (newline > MAX_REQUEST_BYTES) throw new Error("selector request too large");
        const input = pending.subarray(0, newline).toString("utf8");
        pending = pending.subarray(newline + 1);
        let result: unknown;
        let code = 0;
        let message: string | undefined;
        try {
          result = JSON.parse(await select(input));
        } catch (error) {
          code = error instanceof ConfigurationError ? 2 : 1;
          message = (error instanceof Error ? error.message : String(error)).slice(0, 200);
        }
        requests += 1;
        const retire = requests >= limits.requests || process.memoryUsage().rss >= limits.rssBytes;
        const answer = `${JSON.stringify({ code, result, error: message, retire })}\n`;
        if (Buffer.byteLength(answer) > MAX_RESPONSE_BYTES) {
          throw new Error("selector response too large");
        }
        await Bun.write(Bun.stdout, answer);
        if (retire) return;
      }
      if (pending.length > MAX_REQUEST_BYTES) throw new Error("selector request too large");
    }
  } finally {
    clearTimeout(lifetime);
  }
}
