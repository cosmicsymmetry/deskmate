import { beforeAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { TransientError } from "../../src/face";
import type { HttpReply } from "../../src/kit/http";
import { textWidth } from "../../src/kit/raster";
import { renderRequest, tapRequest, viewsRequest } from "../../src/main";
import { buildNow } from "../../src/plugins/context";
import { discoverPlugins } from "../../src/plugins/discovery";
import type { parseManifest } from "../../src/plugins/manifest";
import { type RunPluginInput, runPlugin } from "../../src/plugins/run";
import { runInSandbox, warmSandbox } from "../../src/plugins/sandbox";
import { shippedPlugin } from "./test_support";

const root = join(import.meta.dir, "../../plugins");
const ids = ["this-day-in-history", "dad-jokes", "word-of-the-day"] as const;
type Id = (typeof ids)[number];
const fixtures = Object.fromEntries(
  ids.map((id) => {
    const { folder, source, manifest } = shippedPlugin(id);
    const checks = JSON.parse(readFileSync(join(folder, "check.json"), "utf8"));
    const body = checks.cases[0].responses[0].body as string;
    return [id, { source, manifest, json: JSON.parse(body) as unknown, body }];
  }),
) as Record<
  Id,
  { source: string; manifest: ReturnType<typeof parseManifest>; json: unknown; body: string }
>;
const instant = new Date("2026-10-02T12:00:00Z");
beforeAll(warmSandbox);
function reply(json: unknown): HttpReply {
  return { status: 200, body: JSON.stringify(json), json };
}
function state(result: { state?: unknown }): Record<string, unknown> {
  return result.state as Record<string, unknown>;
}
function draw(id: Id, options: Partial<RunPluginInput> = {}) {
  return runPlugin({
    ...fixtures[id],
    settings: {},
    now: instant,
    timezone: "UTC",
    secrets: {},
    request: async () => reply(fixtures[id].json),
    ...options,
  });
}
for (const id of ids)
  describe(id, () => {
    test("empty-context discovery needs no clock, settings or credentials", () => {
      expect(runInSandbox<unknown[]>(fixtures[id].source, "plan", {})).toEqual([]);
      expect(fixtures[id].manifest.secrets).toEqual([]);
      expect(fixtures[id].manifest.hosts).toHaveLength(1);
    });
    test("recorded success uses one HTTP request and fits state/round limits", async () => {
      let calls = 0;
      const result = await draw(id, {
        request: async (request) => {
          calls++;
          expect(request.headers?.Accept).toBe("application/json");
          expect(request.headers?.["User-Agent"]).toContain("Deskmate");
          return reply(fixtures[id].json);
        },
      });
      expect(calls).toBe(1);
      expect(result.log).toEqual([]);
      expect(Buffer.byteLength(JSON.stringify(result.state))).toBeLessThan(16384);
      expect(state(result).date).toBe("2026-10-02");
    });
    test("same-day refresh retains selected content without fetching", async () => {
      const initial = await draw(id);
      const refreshed = await draw(id, {
        state: initial.state,
        request: async () => {
          throw new Error("Must use daily cache");
        },
      });
      expect(refreshed).toEqual(initial);
    });
    test.each([429, 503, 301])(
      "HTTP %i is transient and does not replace saved content",
      async (status) => {
        const initial = await draw(id);
        const original = JSON.stringify(initial.state);
        await expect(
          draw(id, {
            state: initial.state,
            now: new Date("2026-10-03T12:00:00Z"),
            request: async () => ({ status, body: "unavailable" }),
          }),
        ).rejects.toThrow(TransientError);
        expect(JSON.stringify(initial.state)).toBe(original);
      },
    );
    test.each([{}, null, { en: [], selected: [] }, { selected: [{ text: "x", year: null }] }])(
      "malformed upstream data is not a successful placeholder",
      async (json) => {
        await expect(draw(id, { request: async () => reply(json) })).rejects.toThrow(
          TransientError,
        );
      },
    );
    test("network failures retain the host's last frame through a transient error", async () => {
      await expect(
        draw(id, {
          request: async () => {
            throw new Error("timeout");
          },
        }),
      ).rejects.toThrow(TransientError);
    });
    test("corrupt saved state is replaced with validated data", async () => {
      const result = await draw(id, {
        state: { date: "2026-10-02", zone: "UTC", items: "bad", item: 7, offset: -3 },
      });
      expect(result.log).toEqual([]);
      expect(state(result).date).toBe("2026-10-02");
    });
    test("local New Year and leap day drive the calendar", async () => {
      for (const [utc, zone, date] of [
        ["2026-12-31T15:00:00Z", "Asia/Tokyo", "2027-01-01"],
        ["2027-01-01T07:59:59Z", "America/Los_Angeles", "2026-12-31"],
        ["2028-02-29T12:00:00Z", "UTC", "2028-02-29"],
      ] as const) {
        const result = await draw(id, { now: new Date(utc), timezone: zone });
        expect(state(result).date).toBe(date);
      }
    });
    test("zone changes invalidate saved content but DST within a civil date does not", async () => {
      const initial = await draw(id);
      let calls = 0;
      await draw(id, {
        state: initial.state,
        timezone: "Europe/London",
        request: async () => {
          calls++;
          return reply(fixtures[id].json);
        },
      });
      expect(calls).toBe(1);
      const first = await draw(id, {
        now: new Date("2026-03-08T05:00:00Z"),
        timezone: "America/New_York",
      });
      const later = await draw(id, {
        now: new Date("2026-03-09T03:59:59Z"),
        timezone: "America/New_York",
        state: first.state,
        request: async () => {
          throw new Error("Must use cache");
        },
      });
      expect(later).toEqual(first);
    });
    test("invalid host dates fail explicitly", () => {
      expect(() =>
        runInSandbox(fixtures[id].source, "plan", {
          now: { local: { year: 2026, month: 2, day: 30 } },
        }),
      ).toThrow("valid local date");
    });
    test("real discovery exposes tap fallback and renders a full panel PNG", async () => {
      const { faces, skipped } = await discoverPlugins(
        root,
        { request: async () => reply(fixtures[id].json), readSecrets: async () => ({}) },
        [id],
      );
      expect(skipped).toEqual([]);
      const face = faces[0];
      expect(face?.tap).toBeTruthy();
      const input = { kind: id, settings: {}, timezone: "UTC", event: { taps: 1, point: null } };
      expect(viewsRequest(input, face)).toEqual({ views: [""] });
      expect(() => tapRequest(input, face, instant)).toThrow("handles taps through render");
      const output = await renderRequest(input, face, instant);
      const png = Buffer.from(output.png, "base64");
      expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toEqual([448, 368]);
    });
  });

test("history taps advance cached events and wrap, including coalesced taps", async () => {
  const initial = await draw("this-day-in-history");
  const count = (state(initial).items as unknown[]).length;
  const next = await draw("this-day-in-history", {
    state: initial.state,
    event: { taps: count + 2, point: null },
    request: async () => {
      throw new Error("Must use today's events");
    },
  });
  expect(state(next).index).toBe(2 % count);
  expect(next.svg).not.toBe(initial.svg);
});
test("history endpoint uses local month/day and escapes upstream text", async () => {
  let requested = "";
  const result = await draw("this-day-in-history", {
    now: new Date("2028-02-29T12:00:00Z"),
    request: async (input) => {
      requested = input.url;
      return reply({
        selected: [
          { year: 1900, text: "<script> & quoted event" },
          { year: 2000, text: "x".repeat(300) },
          { year: 2001, text: "漢字" },
        ],
      });
    },
  });
  expect(requested.endsWith("/02/29")).toBe(true);
  expect(result.svg).toContain("&lt;script&gt;");
  expect(result.svg).not.toContain("<script>");
  expect(state(result).items).toHaveLength(1);
});
test("joke taps fetch one new joke even for many coalesced taps", async () => {
  const initial = await draw("dad-jokes");
  let calls = 0;
  const next = await draw("dad-jokes", {
    state: initial.state,
    event: { taps: 9, point: null },
    request: async () => {
      calls++;
      return reply({ id: "next", joke: "A different recorded test joke." });
    },
  });
  expect(calls).toBe(1);
  expect(state(next).item).toEqual({ id: "next", joke: "A different recorded test joke." });
});
test("overlong jokes fail without silently clipping the punchline", async () => {
  await expect(
    draw("dad-jokes", {
      request: async () => reply({ id: "long", joke: "A long joke. ".repeat(100) }),
    }),
  ).rejects.toThrow(TransientError);
});
test("word vocabulary, rollover and coalesced taps select the expected word", async () => {
  const initial = await draw("word-of-the-day");
  expect(state(initial).word).toBe("assiduous");
  const next = await draw("word-of-the-day", {
    state: initial.state,
    event: { taps: 2, point: null },
  });
  expect(state(next).word).toBe("sagacious");
  const nextDay = await draw("word-of-the-day", {
    state: next.state,
    now: new Date("2026-10-03T12:00:00Z"),
  });
  expect(state(nextDay).word).toBe("ruminate");
  expect(state(nextDay).offset).toBe(0);
  const everyday = await draw("word-of-the-day", {
    state: initial.state,
    settings: { vocabulary: "everyday" },
  });
  expect(state(everyday).word).toBe("quietude");
  expect((await draw("word-of-the-day", { settings: { vocabulary: "unknown" } })).svg).toBe(
    initial.svg,
  );
});
test("both word lists offer 48 unique dates before repeating", () => {
  for (const vocabulary of ["curious", "everyday"]) {
    const urls = new Set<string>();
    for (let day = 0; day < 48; day++) {
      const now = buildNow(new Date(Date.UTC(2026, 9, 2 + day, 12)), "UTC");
      const planned = runInSandbox<{ url: string }[]>(fixtures["word-of-the-day"].source, "plan", {
        now,
        settings: { vocabulary },
        answers: [],
      });
      if (!planned[0]) throw new Error("A word must declare its source request");
      urls.add(planned[0].url);
    }
    expect(urls.size).toBe(48);
  }
});
test("definitions strip HTML, decode entities and skip unusable entries", async () => {
  const result = await draw("word-of-the-day", {
    request: async () =>
      reply({
        en: [
          {
            partOfSpeech: "Noun",
            definitions: [
              { definition: "x".repeat(200) },
              {
                definition: '<a href="https://bad.example">Light</a> &amp; joy, &#x2014; together.',
              },
            ],
          },
        ],
      }),
  });
  expect(result.svg).toContain("Light &amp; joy, — together.");
  expect(result.svg).not.toContain("bad.example");
  expect(state(result).text).toBe("Light & joy, — together.");
});

test("every body line fits its real font width, including spaces", async () => {
  for (const id of ids) {
    const checks = JSON.parse(readFileSync(join(root, id, "check.json"), "utf8"));
    for (const example of checks.cases) {
      const body = example.responses[0].body;
      const result = await draw(id, {
        now: new Date(example.now),
        settings: example.settings || {},
        event: example.event,
        request: async () => reply(JSON.parse(body)),
      });
      for (const match of result.svg.matchAll(
        /<text x="26" y="[^"]+" font-size="(\d+)">([^<]*)<\/text>/g,
      )) {
        const text = (match[2] ?? "")
          .replace(/&amp;/g, "&")
          .replace(/&lt;/g, "<")
          .replace(/&gt;/g, ">")
          .replace(/&quot;/g, '"')
          .replace(/&apos;/g, "'");
        expect(textWidth(text, Number(match[1]), 400), `${id}: ${text}`).toBeLessThanOrEqual(397);
      }
    }
  }
});
