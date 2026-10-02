import { beforeAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { ConfigurationError, TransientError } from "../../src/face";
import { type HttpRequest, replyFrom } from "../../src/kit/http";
import { pngFromSvg } from "../../src/kit/raster";
import { parseManifest } from "../../src/plugins/manifest";
import { type RunPluginInput, runPlugin } from "../../src/plugins/run";
import { warmSandbox } from "../../src/plugins/sandbox";

beforeAll(warmSandbox);
const root = join(import.meta.dir, "../../plugins");
const fixture = JSON.parse(readFileSync(join(root, "xkcd/check.json"), "utf8"));
const png = Buffer.from(fixture.cases[0].responses[1].base64, "base64");
const now = new Date("2026-10-02T12:00:00Z");
const official = "https://assets.amuniversal.com/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const another = "https://assets.amuniversal.com/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
function input(id: string, extra: Partial<RunPluginInput> = {}): RunPluginInput {
  return {
    manifest: parseManifest(JSON.parse(readFileSync(join(root, id, "plugin.json"), "utf8")), id),
    source: readFileSync(join(root, id, "index.js"), "utf8"),
    settings: {},
    now,
    timezone: "UTC",
    secrets: {},
    ...extra,
  };
}
async function response(request: HttpRequest, body: string | Uint8Array, status = 200) {
  return replyFrom(new Response(body, { status }), request.as);
}
const metadata = { num: 149, img: "https://imgs.xkcd.com/comics/sandwich.png" };
const artist = {
  artist_name: "Sample Artist",
  artist_href: "https://example.org/artist",
  source_url: "https://example.org/source",
  url: "https://nekos.best/api/v2/neko/sample.png",
  dimensions: { width: 400, height: 600 },
};

describe("xkcd media plugin", () => {
  test("real sandbox fetches metadata then image once and coalesces taps", async () => {
    const seen: string[] = [];
    const result = await runPlugin(
      input("xkcd", {
        state: { version: 1, mode: "compact", index: 4 },
        event: { taps: 3, point: null },
        request: async (r) => {
          seen.push(r.url);
          return response(r, r.as === "json" ? JSON.stringify(metadata) : png);
        },
      }),
    );
    expect(seen).toEqual(["https://xkcd.com/149/info.0.json", metadata.img]);
    expect(result.state).toEqual({ version: 1, mode: "compact", index: 0, number: 149 });
    expect(pngFromSvg(result.svg).length).toBeGreaterThan(100);
  });
  test("latest uses its official endpoint and drops stale compact selection", async () => {
    const seen: string[] = [];
    await runPlugin(
      input("xkcd", {
        settings: { mode: "latest" },
        state: { index: -5 },
        request: async (r) => {
          seen.push(r.url);
          return response(r, r.as === "json" ? JSON.stringify(metadata) : png);
        },
      }),
    );
    expect(seen[0]).toBe("https://xkcd.com/info.0.json");
  });
  test("unknown metadata hosts are never requested", async () => {
    let requests = 0;
    await expect(
      runPlugin(
        input("xkcd", {
          request: async (r) => {
            requests++;
            return response(
              r,
              JSON.stringify({ ...metadata, img: "https://unknown.example/image.png" }),
            );
          },
        }),
      ),
    ).rejects.toBeInstanceOf(TransientError);
    expect(requests).toBe(1);
  });
  for (const status of [301, 404, 429, 503])
    test(`HTTP ${status} keeps the previous frame by failing without a replacement`, async () => {
      const state = { version: 1, mode: "compact", index: 0, number: 149 };
      await expect(
        runPlugin(
          input("xkcd", { state, request: async (r) => response(r, "Unavailable", status) }),
        ),
      ).rejects.toBeInstanceOf(TransientError);
      expect(state.index).toBe(0);
    });
  test("corrupt image and unsupported setting have useful typed failures", async () => {
    await expect(runPlugin(input("xkcd", { settings: { mode: "other" } }))).rejects.toBeInstanceOf(
      ConfigurationError,
    );
    await expect(
      runPlugin(
        input("xkcd", {
          request: async (r) =>
            response(r, r.as === "json" ? JSON.stringify(metadata) : "not an image"),
        }),
      ),
    ).rejects.toBeInstanceOf(TransientError);
  });
});

describe("supplied Calvin and Hobbes strips", () => {
  test("empty configuration is actionable and has no outbound calls", async () => {
    await expect(runPlugin(input("calvin-and-hobbes"))).rejects.toThrow(
      "Add an authorized Calvin and Hobbes image URL",
    );
  });
  test("only exact official CDN URLs are accepted", async () => {
    for (const strips of [
      "https://assets.amuniversal.com.evil.example/aaa",
      "http://assets.amuniversal.com/aaa",
      "https://www.gocomics.com/calvinandhobbes",
      Array(9).fill(official).join(" "),
    ]) {
      await expect(
        runPlugin(input("calvin-and-hobbes", { settings: { strips } })),
      ).rejects.toBeInstanceOf(ConfigurationError);
    }
  });
  test("tap advances supplied list and changed settings reset old selection", async () => {
    const seen: string[] = [];
    const result = await runPlugin(
      input("calvin-and-hobbes", {
        settings: { strips: `${official} ${another}` },
        state: { version: 1, url: official },
        event: { taps: 3, point: null },
        request: async (r) => {
          seen.push(r.url);
          return response(r, png);
        },
      }),
    );
    expect(seen).toEqual([another]);
    expect(result.state).toEqual({ version: 1, url: another });
    const reset = await runPlugin(
      input("calvin-and-hobbes", {
        settings: { strips: official },
        state: result.state,
        request: (r) => response(r, png),
      }),
    );
    expect(reset.state).toEqual({ version: 1, url: official });
  });
  test("upstream failure throws so the host retains its frame", async () => {
    await expect(
      runPlugin(
        input("calvin-and-hobbes", {
          settings: { strips: official },
          request: (r) => response(r, "No strip", 403),
        }),
      ),
    ).rejects.toBeInstanceOf(TransientError);
  });
});

describe("Anime Images", () => {
  test("curated taps choose a distinct image, keep credits and small state", async () => {
    const headers: unknown[] = [];
    const request = async (r: HttpRequest) => {
      headers.push(r.headers);
      return response(r, png);
    };
    const first = await runPlugin(input("anime-images", { request }));
    const next = await runPlugin(
      input("anime-images", { request, state: first.state, event: { taps: 1, point: null } }),
    );
    expect(next.state).not.toEqual(first.state);
    expect(JSON.stringify(next.state).length).toBeLessThan(2000);
    expect(headers[0]).toEqual({
      "User-Agent": "Deskmate (https://github.com/cosmicsymmetry/deskmate)",
    });
    expect(pngFromSvg(next.svg).length).toBeGreaterThan(100);
  });
  test("fresh candidate preserves creator metadata without fetching artist links", async () => {
    const seen: string[] = [];
    const result = await runPlugin(
      input("anime-images", {
        settings: { selection: "fresh", fit: "cover" },
        request: async (r) => {
          seen.push(r.url);
          return response(r, r.as === "json" ? JSON.stringify({ results: [artist] }) : png);
        },
      }),
    );
    expect(seen).toEqual(["https://nekos.best/api/v2/neko?amount=8", artist.url]);
    expect(result.state).toMatchObject({ artist: artist.artist_name, source: artist.source_url });
  });
  test("oversized candidate uses one bounded fallback in the final planning round", async () => {
    let requests = 0;
    const result = await runPlugin(
      input("anime-images", {
        settings: { selection: "fresh" },
        request: async (r) => {
          requests++;
          return response(
            r,
            r.as === "json"
              ? JSON.stringify({ results: [artist] })
              : r.url === artist.url
                ? new Uint8Array(1_048_577)
                : png,
          );
        },
      }),
    );
    expect(requests).toBe(3);
    expect(result.state).not.toMatchObject({ url: artist.url });
    expect(result.log.join(" ")).not.toContain("state was");
  });
  test("unknown image hosts never receive a request, instead use collection", async () => {
    const seen: string[] = [];
    await runPlugin(
      input("anime-images", {
        settings: { selection: "fresh" },
        request: async (r) => {
          seen.push(r.url);
          return response(
            r,
            r.as === "json"
              ? JSON.stringify({ results: [{ ...artist, url: "https://evil.example/a.png" }] })
              : png,
          );
        },
      }),
    );
    expect(seen).toHaveLength(2);
    expect(seen.every((url) => url.startsWith("https://nekos.best/"))).toBe(true);
  });
  test("bad settings fail early and total upstream failure keeps frame", async () => {
    await expect(
      runPlugin(input("anime-images", { settings: { fit: "stretch" } })),
    ).rejects.toBeInstanceOf(ConfigurationError);
    await expect(
      runPlugin(
        input("anime-images", {
          state: { version: 99, url: "https://bad.example" },
          request: (r) => response(r, "Error", 503),
        }),
      ),
    ).rejects.toBeInstanceOf(TransientError);
  });
});

test("Anime accepts real JPEG bytes under a provider PNG URL", async () => {
  const recorded = JSON.parse(readFileSync(join(root, "anime-images/check.json"), "utf8"));
  const example = recorded.cases.find((entry: { name: string }) => entry.name.includes("JPEG"));
  const jpeg = Buffer.from(example.responses[1].base64, "base64");
  const result = await runPlugin(
    input("anime-images", {
      settings: { selection: "fresh" },
      request: (r) => response(r, r.as === "json" ? JSON.stringify({ results: [artist] }) : jpeg),
    }),
  );
  expect(result.svg).toContain("data:image/jpeg;base64,");
  expect(pngFromSvg(result.svg).length).toBeGreaterThan(100);
});

test("Anime skips one unavailable curated entry without losing the last frame", async () => {
  let requests = 0;
  const result = await runPlugin(
    input("anime-images", {
      request: (r) => response(r, ++requests === 1 ? "Removed" : png, requests === 1 ? 404 : 200),
    }),
  );
  expect(requests).toBe(2);
  expect(result.state).toMatchObject({ artist: "Hong" });
});

test("Calvin accepts the current official CDN without following the legacy redirect", async () => {
  const url = "https://featureassets.gocomics.com/assets/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const seen: string[] = [];
  const result = await runPlugin(
    input("calvin-and-hobbes", {
      settings: { strips: url },
      request: (r) => {
        seen.push(r.url);
        return response(r, png);
      },
    }),
  );
  expect(seen).toEqual([url]);
  expect(result.state).toEqual({ version: 1, url });
});
