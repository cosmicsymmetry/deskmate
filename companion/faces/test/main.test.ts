import { expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describeCatalog, renderRequest } from "../src/main";

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
    fields: { type: string; key: string }[];
  }[];
  // Built-ins first, then the worked-example plugin under `plugins/github-stats/`
  // (`src/plugins/discovery.ts`) -- this is the default `DESKMATE_PLUGINS_DIR`, a
  // real folder beside the package, discovered exactly as the server would.
  expect(catalog.map((face) => face.kind)).toEqual([
    "weather",
    "hackernews",
    "rss",
    "token",
    "github-stats",
  ]);
  for (const face of catalog) {
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

test("describe carries a face's refreshSeconds and omits it otherwise", () => {
  const catalog = JSON.parse(
    describeCatalog([
      { kind: "a", label: "A", fields: [], refreshSeconds: 120, render: async () => SQUARE_SVG },
      { kind: "b", label: "B", fields: [], render: async () => SQUARE_SVG },
    ]),
  );
  expect(catalog[0].refreshSeconds).toBe(120);
  expect("refreshSeconds" in catalog[1]).toBe(false);
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

test("a broken plugin folder does not empty the catalog: the built-ins and a good plugin survive", async () => {
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

  // The built-ins still render, unaffected by the broken folder sitting beside them.
  const rendered = await run(
    "render",
    JSON.stringify({ kind: "weather", settings: { location: "Berlin" } }),
    { DESKMATE_PLUGINS_DIR: root },
  );
  expect(rendered.code === 0 || rendered.code === 1).toBe(true); // 1: no network here
});
