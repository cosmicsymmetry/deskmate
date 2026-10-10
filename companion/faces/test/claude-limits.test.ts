import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { XMLParser } from "fast-xml-parser";
import { allCases } from "../src/cases";
import { ConfigurationError, TransientError } from "../src/face";
import {
  claudeLimits,
  fetchClaudeUsage,
  parseClaudeUsage,
  renderClaudeLimits,
  renderClaudeLimitsRequest,
  usageFreshness,
} from "../src/faces/claude-limits";
import { trackedWidth } from "../src/kit/svg";
import { describeCatalog, renderRequest, tapRequest, viewsRequest } from "../src/main";

const captured = await Bun.file(new URL("./claude-usage.captured.json", import.meta.url)).json();
const body = JSON.stringify(captured);
const now = new Date("2026-10-10T06:40:00Z");
const settings = { url: "https://example.com/claude-usage.json" };
const fromFeed = async () => body;
const noFetch = async (): Promise<string> => {
  throw new Error("A tap must not fetch");
};
const variant = (patch: Record<string, unknown>) => JSON.stringify({ ...captured, ...patch });

describe("Claude usage feed", () => {
  test("parses the captured response without losing plan, windows or reset labels", () => {
    expect(parseClaudeUsage(body)).toEqual(captured);
  });

  test.each([
    "",
    "<html>bad gateway</html>",
    "null",
    "[]",
    "42",
    '{"windows":{}}',
    '{"windows":[]}',
    "{}",
    '{"windows":[null]}',
  ])("bad feed %s has a retryable face_status sentence", (input) => {
    expect(() => parseClaudeUsage(input)).toThrow(TransientError);
    expect(() => parseClaudeUsage(input)).toThrow(/The Claude usage feed/);
  });

  test("blank and missing fields stay unknown instead of becoming zero", () => {
    expect(parseClaudeUsage(variant({ updated: " ", plan: null, windows: [{}] }))).toEqual({
      updated: null,
      plan: "",
      windows: [{ name: "Window 1", used_pct: null, resets_at_label: "", resets_in_label: "" }],
    });
  });

  test.each([null, "", "10", "NaN", true, {}, []].map((value) => ({ value })))(
    "does not coerce a nonnumeric percentage (%j)",
    ({ value: used_pct }) => {
      const usage = parseClaudeUsage(variant({ windows: [{ ...captured.windows[0], used_pct }] }));
      expect(usage.windows[0]?.used_pct).toBeNull();
      expect(renderClaudeLimits(usage, now)).toContain(">—</text>");
    },
  );

  test("clamps percentages and preserves real zero", () => {
    const usage = parseClaudeUsage(
      variant({
        windows: [-20, 0, 12.5, 140].map((used_pct) => ({ ...captured.windows[0], used_pct })),
      }),
    );
    expect(usage.windows.map((window) => window.used_pct)).toEqual([0, 0, 12.5, 100]);
  });

  test("a JSON number overflowing to infinity is unknown", () => {
    expect(parseClaudeUsage('{"windows":[{"used_pct":1e999}]}').windows[0]?.used_pct).toBeNull();
  });

  test("normalizes, bounds and escapes upstream text", () => {
    const usage = parseClaudeUsage(
      variant({
        plan: "  Max\n5× ",
        windows: [{ name: '<script> & "bad"', resets_at_label: "X".repeat(50_000) }],
      }),
    );
    expect(usage.plan).toBe("Max 5×");
    expect(usage.windows[0]?.resets_at_label).toHaveLength(160);
    const svg = renderClaudeLimits(usage, now);
    expect(svg).not.toContain("<script>");
    expect(svg).toContain("&lt;script&gt;");
  });

  test("more than twelve windows is an explicit transient failure, never silent loss", () => {
    expect(() =>
      parseClaudeUsage(variant({ windows: Array(13).fill(captured.windows[0]) })),
    ).toThrow("more than 12 windows");
  });

  test.each([
    {},
    { url: "" },
    { url: " " },
    { url: 5 },
    { url: "not a URL" },
    { url: "http://example.com/feed" },
    { url: "ftp://example.com/feed" },
    { url: "https://" },
    { url: "https:example.com" },
    { url: "https://user:secret@example.com/" },
    { url: "https://exa mple.com/" },
    { url: "https://example.com/\nfeed" },
    { url: "https:\\example.com" },
  ])("invalid URL %j fails before fetching with a configuration sentence", async (input) => {
    let calls = 0;
    const get = async () => {
      calls++;
      return body;
    };
    await expect(fetchClaudeUsage(input, get)).rejects.toBeInstanceOf(ConfigurationError);
    await expect(fetchClaudeUsage(input, get)).rejects.toThrow(
      "Set Usage feed to a valid https:// URL",
    );
    expect(calls).toBe(0);
  });

  test("fetches the trimmed URL through the supplied kit fetcher", async () => {
    const asked: string[] = [];
    expect(
      await fetchClaudeUsage({ url: `  ${settings.url}  ` }, async (url) => {
        asked.push(url);
        return body;
      }),
    ).toEqual(captured);
    expect(asked).toEqual([settings.url]);
  });

  test.each([
    new TransientError("feed returned HTTP 503"),
    new TransientError("feed timed out"),
    new TransientError("the response is too large"),
    new ConfigurationError("feed is a private or local address and is not fetched"),
  ])("preserves the HTTP guard's typed sentence: %s", async (error) => {
    await expect(
      fetchClaudeUsage(settings, async () => {
        throw error;
      }),
    ).rejects.toBe(error);
  });

  test("unexpected fetch errors become a safe transient sentence", async () => {
    const get = async () => {
      throw new Error("https://example.com/?secret=do-not-print");
    };
    await expect(fetchClaudeUsage(settings, get)).rejects.toBeInstanceOf(TransientError);
    await expect(fetchClaudeUsage(settings, get)).rejects.toThrow(
      "The Claude usage feed could not be fetched. It will be retried.",
    );
  });
});

describe("Claude feed freshness", () => {
  const usage = parseClaudeUsage(body);
  test("age is based on the capture, not the successful fetch", () => {
    expect(usageFreshness(usage, now)).toEqual({ label: "Updated 4m ago", warning: false });
    expect(usageFreshness(usage, new Date("2026-10-10T08:40:00Z"))).toEqual({
      label: "Stale · updated 2h ago",
      warning: true,
    });
    expect(renderClaudeLimits(usage, new Date("2026-10-10T08:40:00Z"))).toContain(
      "Stale · updated 2h ago",
    );
  });
  test("crosses the stale threshold only after one hour", () => {
    const at = Date.parse(captured.updated);
    expect(usageFreshness(usage, new Date(at + 3_600_000)).warning).toBe(false);
    expect(usageFreshness(usage, new Date(at + 3_600_001)).warning).toBe(true);
  });
  test.each([undefined, null, " ", "yesterday", "2026-10-10", "2026-10-10T06:35:15"])(
    "missing/invalid timestamp %j never claims to be fresh",
    (updated) => {
      expect(usageFreshness(parseClaudeUsage(variant({ updated })), now)).toEqual({
        label: "Update time unknown",
        warning: true,
      });
    },
  );
  test("future clock errors are visible, small clock skew reads as now", () => {
    expect(usageFreshness(usage, new Date("2026-10-10T05:40:00Z"))).toEqual({
      label: "Update time is ahead",
      warning: true,
    });
    expect(usageFreshness(usage, new Date("2026-10-10T06:35:00Z"))).toEqual({
      label: "Updated now",
      warning: false,
    });
  });
});

describe("Claude face and selection", () => {
  const three = variant({
    windows: [...captured.windows, { ...captured.windows[1], name: "Weekly · Sonnet" }],
  });
  const fromThree = async () => three;

  test("catalog exposes one text setting, builtin origin and a three-minute cadence", () => {
    const face = JSON.parse(describeCatalog()).find(
      (entry: { kind: string }) => entry.kind === "claude-limits",
    );
    expect(face).toMatchObject({
      label: "Claude limits",
      origin: "builtin",
      refresh_seconds: 180,
      views: true,
      selector: true,
    });
    expect(face.fields).toEqual([
      {
        type: "text",
        key: "url",
        label: "Usage feed",
        placeholder: "e.g. https://example.com/claude-usage.json",
      },
    ]);
  });

  test("the public render seam produces a PNG from captured data", async () => {
    const result = await renderRequest(
      { kind: "claude-limits", settings },
      { ...claudeLimits, render: (s, n, c) => renderClaudeLimitsRequest(s, n, c, fromFeed) },
      now,
    );
    const png = Buffer.from(result.png, "base64");
    expect(png.subarray(0, 4)).toEqual(Buffer.from([0x89, 0x50, 0x4e, 0x47]));
    expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toEqual([448, 368]);
  });

  test("a single window draws its information with no empty second row", async () => {
    const result = await renderClaudeLimitsRequest(settings, now, {}, async () =>
      variant({ windows: [captured.windows[0]] }),
    );
    expect(result.svg).toContain("Session");
    expect(result.svg).not.toContain("Weekly");
    expect(viewsRequest({ kind: "claude-limits", settings, state: result.state }).views).toEqual([
      "",
    ]);
  });

  test("extra windows are reachable by a pure selector and render-only taps, with wrapping", async () => {
    const first = await renderClaudeLimitsRequest(settings, now, {}, fromThree);
    expect(first.svg).not.toContain("Sonnet");
    expect(viewsRequest({ kind: "claude-limits", settings, state: first.state }).views).toEqual([
      "",
      "page-2",
    ]);
    const selected = tapRequest(
      { kind: "claude-limits", settings, state: first.state, event: { taps: 1 } },
      undefined,
      now,
    );
    expect(selected.view).toBe("page-2");
    const second = await renderClaudeLimitsRequest(
      settings,
      now,
      { state: selected.state, view: selected.view },
      noFetch,
    );
    const fallback = await renderClaudeLimitsRequest(
      settings,
      now,
      { state: first.state, event: { taps: 1, point: null } },
      noFetch,
    );
    expect(fallback.svg).toBe(second.svg);
    expect(second.svg).toContain("Weekly · Sonnet");
    expect(second.svg).toContain(">2/2</text>");
    const wrapped = tapRequest(
      { kind: "claude-limits", settings, state: second.state, event: { taps: 3 } },
      undefined,
      now,
    );
    expect(wrapped.view).toBe("");
  });

  test("staging a view leaves selection alone and unknown views draw the first page", async () => {
    const first = await renderClaudeLimitsRequest(settings, now, {}, fromThree);
    const staged = await renderClaudeLimitsRequest(
      settings,
      now,
      { state: first.state, view: "page-2" },
      noFetch,
    );
    expect(staged.state).toEqual(first.state);
    const unknown = await renderClaudeLimitsRequest(
      settings,
      now,
      { state: first.state, view: "page-999" },
      noFetch,
    );
    expect(unknown.svg).toBe(first.svg);
  });

  test("scheduled refresh fetches, keeps a recent page, resets after ten minutes and handles shrinking feeds", async () => {
    const first = await renderClaudeLimitsRequest(settings, now, {}, fromThree);
    const second = await renderClaudeLimitsRequest(
      settings,
      now,
      { state: first.state, event: { taps: 1, point: null } },
      noFetch,
    );
    let calls = 0;
    const get = async () => {
      calls++;
      return three;
    };
    const recent = await renderClaudeLimitsRequest(
      settings,
      new Date(now.getTime() + 180_000),
      { state: second.state },
      get,
    );
    expect(recent.state.page).toBe(1);
    expect(calls).toBe(1);
    const expired = await renderClaudeLimitsRequest(
      settings,
      new Date(now.getTime() + 601_000),
      { state: second.state },
      get,
    );
    expect(expired.state.page).toBe(0);
    const shrunk = await renderClaudeLimitsRequest(
      settings,
      now,
      { state: second.state },
      fromFeed,
    );
    expect(shrunk.state.page).toBe(0);
  });

  test("a URL change or corrupt cache cannot draw the old account's usage", async () => {
    const first = await renderClaudeLimitsRequest(settings, now, {}, fromFeed);
    for (const state of [
      { ...first.state, url: "https://other.example/feed" },
      { ...first.state, usage: {} },
      { ...first.state, usage: { windows: [null] } },
    ]) {
      await expect(
        renderClaudeLimitsRequest(settings, now, { state, view: "" }, noFetch),
      ).rejects.toThrow(TransientError);
      expect(claudeLimits.views?.(settings, state)).toEqual([""]);
    }
  });

  test("twelve windows stay within six views and the server's state budget", async () => {
    const result = await renderClaudeLimitsRequest(settings, now, {}, async () =>
      variant({
        windows: Array.from({ length: 12 }, () => ({
          name: "X".repeat(1000),
          used_pct: 90,
          resets_at_label: "Y".repeat(1000),
          resets_in_label: "Z".repeat(1000),
        })),
      }),
    );
    expect(claudeLimits.views?.(settings, result.state)).toHaveLength(6);
    expect(Buffer.byteLength(JSON.stringify(result.state))).toBeLessThan(16 * 1024);
  });

  test("reset labels fall back truthfully and blank plan does not draw a badge", () => {
    const svg = renderClaudeLimits(
      parseClaudeUsage(
        variant({
          plan: "",
          windows: [{ ...captured.windows[0], resets_at_label: "", resets_in_label: "3h" }, {}],
        }),
      ),
      now,
    );
    expect(svg).toContain("Reset in 3h at update");
    expect(svg).toContain("Reset time unavailable");
    expect(svg).not.toContain("Max");
  });

  test("all case text fits inside the content margins, including long upstream labels", () => {
    const parser = new XMLParser({ ignoreAttributes: false, attributeNamePrefix: "" });
    for (const { svg } of allCases().filter((entry) => entry.name.startsWith("claude-limits--"))) {
      const nodes = parser.parse(svg).svg.text;
      for (const node of nodes) {
        const width = trackedWidth(
          String(node["#text"]),
          Number(node["font-size"]),
          Number(node["font-weight"]),
          Number(node["letter-spacing"] ?? 0),
        );
        const x = Number(node.x);
        const left =
          node["text-anchor"] === "end"
            ? x - width
            : node["text-anchor"] === "middle"
              ? x - width / 2
              : x;
        expect(left).toBeGreaterThanOrEqual(23.9);
        expect(left + width).toBeLessThanOrEqual(424.1);
      }
    }
  });
});

test("Claude configuration failures cross the actual CLI as one face_status sentence", async () => {
  for (const url of ["", "http://example.com/feed", "https://127.0.0.1/feed"]) {
    const child = Bun.spawn(["bun", "run", `${import.meta.dir}/../src/main.ts`, "render"], {
      stdin: "pipe",
      stdout: "pipe",
      stderr: "pipe",
    });
    child.stdin.write(JSON.stringify({ kind: "claude-limits", settings: { url } }));
    await child.stdin.end();
    const [out, err, code] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ]);
    expect(code).toBe(2);
    expect(out).toBe("");
    expect(err.trim().split("\n")).toHaveLength(1);
    expect(err).toMatch(/Set Usage feed|private or local address/);
  }
});

test("a bad feed response crosses the actual CLI as a transient face_status sentence", async () => {
  // Stub only the network in a separate process; use the real parser, face registry
  // and CLI exception boundary, without relying on a public service failing today.
  const directory = mkdtempSync(join(tmpdir(), "claude-limits-cli-"));
  const preload = join(directory, "feed.ts");
  writeFileSync(
    preload,
    `import { mock } from "bun:test";
     const path = ${JSON.stringify(join(import.meta.dir, "../src/kit/http.ts"))};
     const http = await import(path);
     mock.module(path, () => ({ ...http, fetchText: async () => "<html>bad gateway</html>" }));`,
  );
  try {
    const child = Bun.spawn(
      ["bun", "--preload", preload, `${import.meta.dir}/../src/main.ts`, "render"],
      { stdin: "pipe", stdout: "pipe", stderr: "pipe" },
    );
    child.stdin.write(JSON.stringify({ kind: "claude-limits", settings }));
    await child.stdin.end();
    const [out, err, code] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ]);
    expect(code).toBe(1);
    expect(out).toBe("");
    expect(err).toBe("The Claude usage feed returned invalid JSON.\n");
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
