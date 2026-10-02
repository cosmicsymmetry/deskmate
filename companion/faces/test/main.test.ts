import { expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { storyFromItem } from "../src/faces/hackernews";
import { describeCatalog, renderRequest, tapRequest } from "../src/main";
import { pluginFolders } from "../src/plugins/releases";

const MAIN = `${import.meta.dir}/../src/main.ts`;
const SQUARE_SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><rect width="448" height="368" fill="black"/></svg>';

async function run(
  verb: string,
  stdin = "",
  env: Record<string, string> = {},
): Promise<{ code: number; out: Uint8Array; err: string }> {
  const child = Bun.spawn(["bun", "run", MAIN, verb], {
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, ...env },
  });
  child.stdin.write(stdin);
  await child.stdin.end();
  const [out, err, code] = await Promise.all([
    new Response(child.stdout).bytes(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  return { code, out, err };
}

test("describe prints the catalog the server builds the add menu from", async () => {
  const { code, out } = await run("describe");
  expect(code).toBe(0);
  const catalog = JSON.parse(new TextDecoder().decode(out)) as {
    kind: string;
    withdrawn?: string;
    fields: { type: string; key: string }[];
  }[];
  // Built-ins first, then the shipped plugins sorted by folder name
  // (`src/plugins/discovery.ts`) -- this is the default `DESKMATE_PLUGINS_DIR`, a
  // real folder beside the package, discovered exactly as the server would.
  expect(catalog.map((face) => face.kind)).toEqual([
    "weather",
    "hackernews",
    "rss",
    "token",
    ...pluginFolders(join(import.meta.dir, "../plugins")),
  ]);
  expect(catalog.map((face) => face.kind)).not.toContain("calvin-and-hobbes");
  for (const face of catalog) {
    expect(face.withdrawn).toBeUndefined();
    for (const field of face.fields) {
      expect(["text", "url", "enum"]).toContain(field.type);
    }
  }
  // A secret has no place in a response the browser can read.
  expect(JSON.stringify(catalog)).not.toContain("api_key");
});

test("render passes state and the event to the face and returns an envelope", async () => {
  const seen: unknown[] = [];
  const result = await renderRequest(
    { kind: "probe", settings: {}, state: { page: 2 }, event: { taps: 3, point: null } },
    {
      kind: "probe",
      label: "Probe",
      fields: [],
      tap: "Tap me.",
      render: (_settings, _now, context) => {
        seen.push(context);
        return { svg: SQUARE_SVG, state: { page: 5 } };
      },
    },
  );
  expect(seen).toEqual([{ state: { page: 2 }, event: { taps: 3, point: null } }]);
  expect(result.state).toEqual({ page: 5 });
  expect(Buffer.from(result.png, "base64").subarray(0, 4)).toEqual(
    Buffer.from([0x89, 0x50, 0x4e, 0x47]),
  );
});

test("a face that returns a bare SVG leaves the stored state alone", async () => {
  const result = await renderRequest(
    { kind: "probe", settings: {}, state: { page: 2 } },
    { kind: "probe", label: "Probe", fields: [], render: async () => SQUARE_SVG },
  );
  expect("state" in result).toBe(false);
});

test("a render with no state and no event is what every existing face already gets", async () => {
  const seen: unknown[] = [];
  await renderRequest(
    { kind: "probe", settings: {} },
    {
      kind: "probe",
      label: "Probe",
      fields: [],
      render: (_settings, _now, context) => {
        seen.push(context);
        return SQUARE_SVG;
      },
    },
  );
  expect(seen).toEqual([{ state: undefined, event: undefined }]);
});

test("a request carrying a timezone passes it into the face's context", async () => {
  const seen: unknown[] = [];
  await renderRequest(
    { kind: "probe", settings: {}, timezone: "Asia/Dubai" },
    {
      kind: "probe",
      label: "Probe",
      fields: [],
      render: (_settings, _now, context) => {
        seen.push(context?.timezone);
        return SQUARE_SVG;
      },
    },
  );
  expect(seen).toEqual(["Asia/Dubai"]);
});

test("a request with no timezone leaves the face's context without one", async () => {
  const seen: unknown[] = [];
  await renderRequest(
    { kind: "probe", settings: {} },
    {
      kind: "probe",
      label: "Probe",
      fields: [],
      render: (_settings, _now, context) => {
        seen.push("timezone" in (context ?? {}));
        return SQUARE_SVG;
      },
    },
  );
  expect(seen).toEqual([false]);
});

test("describe carries a face's tap sentence and omits it otherwise", () => {
  const catalog = JSON.parse(
    describeCatalog([
      {
        kind: "a",
        label: "A",
        fields: [],
        tap: "Tap the panel for more.",
        render: async () => SQUARE_SVG,
      },
      { kind: "b", label: "B", fields: [], render: async () => SQUARE_SVG },
    ]),
  );
  expect(catalog[0].tap).toBe("Tap the panel for more.");
  expect("tap" in catalog[1]).toBe(false);
});

test("describe carries a face's refreshSeconds as refresh_seconds, snake_case for the Rust catalog, and omits the key entirely otherwise", () => {
  // `crates/server/src/data_cards/faces_package.rs`'s `CatalogFace` has no
  // `#[serde(rename)]` and no `deny_unknown_fields`: a camelCase `refreshSeconds`
  // here deserializes to `None` there, silently. See catalog-sample.json below,
  // which pins this exact shape across both languages.
  const catalog = JSON.parse(
    describeCatalog([
      { kind: "a", label: "A", fields: [], refreshSeconds: 120, render: async () => SQUARE_SVG },
      { kind: "b", label: "B", fields: [], render: async () => SQUARE_SVG },
    ]),
  );
  expect(catalog[0].refresh_seconds).toBe(120);
  expect("refreshSeconds" in catalog[0]).toBe(false);
  // Absent, not `null`: the Rust side distinguishes "no cadence declared" from an
  // explicit null, and `Option<u64>`'s `#[serde(default)]` only supplies `None` for
  // a MISSING key -- an explicit `null` still deserializes fine here (serde treats
  // `null` as `None` for an `Option`), but this pins that this package never sends
  // one, which is the stricter and correct claim: this plugin declared nothing.
  expect("refresh_seconds" in catalog[1]).toBe(false);
  expect(JSON.stringify(catalog[1])).not.toContain("refresh_seconds");
});

test("an unknown kind and an incomplete setting are configuration errors: exit 2, nothing on stdout", async () => {
  for (const request of [
    '{"kind":"horoscope","settings":{}}',
    '{"kind":"weather","settings":{"location":"  "}}',
  ]) {
    const { code, out, err } = await run("render", request);
    expect(code).toBe(2);
    expect(out.length).toBe(0);
    expect(err.trim().split("\n")).toHaveLength(1);
  }
});

test("a request that is not JSON is not the owner's fault: exit 1", async () => {
  const { code, out } = await run("render", "not json");
  expect(code).toBe(1);
  expect(out.length).toBe(0);
});

test("an unknown verb prints usage and draws nothing", async () => {
  const { code, out } = await run("paint");
  expect(code).toBe(64);
  expect(out.length).toBe(0);
});

test("an event whose point is absent is still a tap", () => {
  // C1 sends `point: null`; a future sender may omit the key. Dropping the event
  // there would swallow the tap with nothing to show for it.
  const seen: unknown[] = [];
  return renderRequest(
    { kind: "probe", settings: {}, event: { taps: 2 } },
    {
      kind: "probe",
      label: "Probe",
      fields: [],
      render: (_settings, _now, context) => {
        seen.push(context?.event);
        return SQUARE_SVG;
      },
    },
  ).then(() => {
    expect(seen).toEqual([{ taps: 2, point: null }]);
  });
});

test("a whole envelope reaches stdout without truncation", async () => {
  // process.stdout.write plus an immediate process.exit can cut a ~60 KB envelope
  // short; the server then sees neither a PNG nor JSON and calls it transient.
  const child = Bun.spawn(["bun", MAIN, "render"], {
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  child.stdin.write(
    JSON.stringify({
      kind: "rss",
      settings: { url: "https://example.com/feed.xml", title: "News" },
    }),
  );
  await child.stdin.end();
  const [out, code] = [await new Response(child.stdout).text(), await child.exited];
  if (code === 0) {
    const envelope = JSON.parse(out) as { png: string };
    expect(Buffer.from(envelope.png, "base64").subarray(0, 4)).toEqual(
      Buffer.from([0x89, 0x50, 0x4e, 0x47]),
    );
  } else {
    // No network in this environment: the face failed, which is a clean exit path.
    expect(out).toBe("");
  }
});

// Two full `bun run main.ts` subprocess spawns below, each doing its own bun boot
// and QuickJS WASM instantiation, so the test below is not fast even when discovery
// itself is (see `DISCOVERY_PROBE_DEADLINE_MS`, `src/plugins/discovery.ts`). Bun's
// default per-test timeout is 5000 ms; a generous, explicit one here is the fallback
// a slow machine gets, not the fix -- the fix is that deadline being short.
const BROKEN_PLUGIN_FOLDER_TEST_TIMEOUT_MS = 20_000;

test(
  "a broken plugin folder does not empty the catalog: the built-ins and a good plugin survive",
  async () => {
    // `load_catalog` on the Rust side treats a failed `describe` as an EMPTY catalog --
    // an empty add menu -- so this is the property Task 9 exists to guarantee, driven
    // through the real CLI subprocess boundary the server actually uses.
    const root = join(tmpdir(), `main-plugins-${Math.random().toString(36).slice(2)}`);
    const write = (folder: string, manifest: unknown, source: string) => {
      mkdirSync(join(root, folder), { recursive: true });
      writeFileSync(join(root, folder, "plugin.json"), JSON.stringify(manifest));
      writeFileSync(join(root, folder, "index.js"), source);
    };
    write(
      "sound-plugin",
      {
        api: 1,
        id: "sound-plugin",
        version: "1.0.0",
        label: "Sound plugin",
        description: "d",
        author: "a",
        hosts: [],
        secrets: [],
        fields: [],
      },
      `export function plan(){ return []; }
     export function render(){ return { layout: { type: "div", style: { display: "flex", width: 448, height: 368, background: "#000" }, children: "ok" } }; }`,
    );
    write(
      "broken-plugin",
      {
        api: 1,
        id: "broken-plugin",
        version: "1.0.0",
        label: "Broken plugin",
        description: "d",
        author: "a",
        hosts: [],
        secrets: [],
        fields: [],
      },
      "export function plan(){ this does not parse }",
    );
    write(
      "hostile-plugin",
      {
        api: 1,
        id: "hostile-plugin",
        version: "1.0.0",
        label: "Hostile plugin",
        description: "d",
        author: "a",
        hosts: [],
        secrets: [],
        fields: [],
      },
      // A `plan` that never returns is exactly what discovery's own short probe
      // deadline (`DISCOVERY_PROBE_DEADLINE_MS`, `src/plugins/discovery.ts`) exists
      // to catch quickly -- at the render path's full 2000 ms deadline this one
      // folder alone would cost two seconds of every 60 s catalog re-read, and this
      // test used to time out on a slower machine because of exactly that.
      `export function plan(){ while(true){} }
     export function render(){ return { layout: { type: "div", style: { display: "flex", width: 448, height: 368, background: "#000" }, children: "ok" } }; }`,
    );

    const malformed = [
      { fields: [{ type: "text", key: "user", label: "User" }] },
      { fields: [{ type: "number", key: "n", label: "N" }] },
      { fields: [{ type: "text", key: "k", label: "L", placeholder: "p", default: 5 }] },
      { fields: [{ type: "enum", key: "k", label: "L", default: "a" }] },
      { refreshSeconds: -1 },
      { refreshSeconds: 1e20 },
    ];
    for (const [i, extra] of malformed.entries()) {
      write(
        `malformed-${i}`,
        {
          api: 1,
          id: `malformed-${i}`,
          version: "1.0.0",
          label: "Malformed",
          description: "d",
          author: "a",
          hosts: [],
          secrets: [],
          fields: [],
          ...extra,
        },
        `export function plan(){ return []; }
          export function render(){ return { svg: ${JSON.stringify(SQUARE_SVG)} }; }`,
      );
    }

    const { code, out, err } = await run("describe", "", { DESKMATE_PLUGINS_DIR: root });
    expect(code).toBe(0);
    const catalog = JSON.parse(new TextDecoder().decode(out)) as { kind: string }[];
    expect(catalog.map((face) => face.kind)).toEqual([
      "weather",
      "hackernews",
      "rss",
      "token",
      "sound-plugin",
    ]);
    expect(err).toContain("broken-plugin");
    expect(err).toContain("hostile-plugin");
    for (const i of malformed.keys()) {
      expect(catalog.map((face) => face.kind)).not.toContain(`malformed-${i}`);
      expect(err).toContain(`malformed-${i}`);
    }

    // A good plugin still renders, unaffected by the broken folders sitting beside
    // it -- deliberately the discovered plugin itself, not a built-in like `weather`,
    // so this assertion is a fast, deterministic exercise of the exact code path this
    // task is about (`discoverPlugins` -> the adapter's `render`) rather than a real
    // network fetch, which was this test's actual dominant cost before this rewrite.
    const rendered = await run("render", JSON.stringify({ kind: "sound-plugin", settings: {} }), {
      DESKMATE_PLUGINS_DIR: root,
    });
    expect(rendered.code).toBe(0);
    const planned = await run("views", JSON.stringify({ kind: "sound-plugin" }), {
      DESKMATE_PLUGINS_DIR: root,
    });
    expect(planned.code).toBe(0);
    expect(JSON.parse(new TextDecoder().decode(planned.out))).toEqual({ views: [""] });
    const tapped = await run(
      "tap",
      JSON.stringify({ kind: "sound-plugin", event: { taps: 1, point: null } }),
      {
        DESKMATE_PLUGINS_DIR: root,
      },
    );
    expect(tapped.code).toBe(1);
    expect(tapped.err).toContain("handles taps through render");
  },
  BROKEN_PLUGIN_FOLDER_TEST_TIMEOUT_MS,
);

test("a face without onTap falls back to rendering instead of selecting its resting frame", () => {
  expect(() =>
    tapRequest(
      { kind: "probe", event: { taps: 1, point: null } },
      {
        kind: "probe",
        label: "Probe",
        fields: [],
        tap: "Refresh",
        render: () => SQUARE_SVG,
      },
    ),
  ).toThrow("this face handles taps through render");
});

// The server's unstaged fallback sends the OLD state and an event, with no view.
// Exercise its real subprocess envelope, not just the staged onTap -> view path.
// Each case launches 11 (RSS) or 13 (HN) real Bun children, including cold font
// measurement and PNG rendering. The default 5s budget timed out on Linux CI
// near the end of the HN sequence; give this integration sequence 15s in total.
// Built-in renders bypass plugin discovery. This is not a per-render latency gate.
for (const kind of ["rss", "hackernews"] as const) {
  test(`${kind}: an unstaged tap renders page four and subsequent taps keep moving`, async () => {
    const now = new Date("2026-09-23T05:00:00Z");
    const items: unknown[] = await Bun.file(
      new URL("./hn-front-page.captured.json", import.meta.url),
    ).json();
    const stories = items.flatMap((item, index) => {
      const story = storyFromItem(item, index + 1, now);
      return story === undefined ? [] : [story];
    });
    const initial = {
      page: 2,
      tappedAt: null as string | null,
      ...(kind === "rss"
        ? { feedTitle: "Captured headlines", entries: stories.slice(0, 16) }
        : { stories: stories.slice(0, 20) }),
    };
    const request = async (state: unknown, extra: object) => {
      const result = await run("render", JSON.stringify({ kind, settings: {}, state, ...extra }));
      expect(result.err).toBe("");
      expect(result.code).toBe(0);
      return JSON.parse(new TextDecoder().decode(result.out)) as {
        png: string;
        state: typeof initial;
      };
    };
    const third = await request(initial, { view: "page-3" });
    const first = await request(initial, { view: "" });
    let state = initial;
    let lastPng = third.png;
    for (const page of kind === "rss" ? [3, 0, 1] : [3, 4, 0, 1]) {
      const before = Date.now();
      const tapped = await request(state, { event: { taps: 1, point: null } });
      const expected = await request(state, { view: page === 0 ? "" : `page-${page + 1}` });
      expect(tapped.state.page).toBe(page);
      expect(Date.parse(tapped.state.tappedAt ?? "")).toBeGreaterThanOrEqual(before);
      expect(tapped.png).not.toBe(lastPng);
      expect(tapped.png).toBe(expected.png);
      if (page === 3) expect(tapped.png).not.toBe(first.png);
      state = tapped.state;
      lastPng = tapped.png;
    }
    const coalesced = await request(initial, { event: { taps: 3, point: null } });
    expect(coalesced.state.page).toBe(kind === "rss" ? 1 : 0);
    // An explicit view is a draw instruction, even if an event accompanies it.
    const explicit = await request(initial, { view: "page-4", event: { taps: 3, point: null } });
    expect(explicit.state).toEqual(initial);
    expect(explicit.png).toBe((await request(initial, { view: "page-4" })).png);
  }, 15_000);
}
