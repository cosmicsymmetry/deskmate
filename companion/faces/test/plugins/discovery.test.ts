import { beforeAll, describe, expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { FaceDefinition } from "../../src/face";
import { discoverPlugins } from "../../src/plugins/discovery";
import { warmSandbox } from "../../src/plugins/sandbox";

beforeAll(async () => {
  await warmSandbox();
});

/** `faces[0]`, asserted present -- avoids `!` (this package's lint forbids it) while
 * still failing loudly, with a useful message, if a fixture ever discovers nothing. */
function firstFace(faces: readonly FaceDefinition[]): FaceDefinition {
  const face = faces[0];
  expect(face).toBeDefined();
  if (face === undefined) throw new Error("unreachable: asserted above");
  return face;
}

function fixture(): string {
  const root = join(tmpdir(), `plugins-${Math.random().toString(36).slice(2)}`);
  const write = (folder: string, manifest: unknown, source: string) => {
    mkdirSync(join(root, folder), { recursive: true });
    writeFileSync(join(root, folder, "plugin.json"), JSON.stringify(manifest));
    writeFileSync(join(root, folder, "index.js"), source);
  };
  const ok = (id: string) => ({
    api: 1,
    id,
    version: "1.0.0",
    label: id,
    description: "d",
    author: "a",
    hosts: [],
    secrets: [],
    fields: [],
  });
  const card = `export function plan(){ return []; }
    export function render(){ return { layout: { type: "div", style: { display: "flex", width: 448, height: 368, background: "#000" }, children: "ok" } }; }`;
  write("good", ok("good"), card);
  write("broken-json", "{not json", card);
  write("wrong-api", { ...ok("wrong-api"), api: 2 }, card);
  write("id-mismatch", ok("something-else"), card);
  write("syntax-error", ok("syntax-error"), "export function plan(){ return [ }");
  mkdirSync(join(root, "empty-folder"), { recursive: true });
  return root;
}

describe("discoverPlugins", () => {
  test("one bad plugin does not empty the catalog", async () => {
    const { faces, skipped } = await discoverPlugins(fixture());
    expect(faces.map((face) => face.kind)).toEqual(["good"]);
    expect(skipped.map((entry) => entry.folder).sort()).toEqual([
      "broken-json",
      "empty-folder",
      "id-mismatch",
      "syntax-error",
      "wrong-api",
    ]);
  });

  test("a discovered plugin renders through the FaceDefinition seam", async () => {
    const { faces } = await discoverPlugins(fixture());
    const result = await firstFace(faces).render({}, new Date("2026-09-23T10:00:00Z"), {});
    expect(typeof result === "string" ? result : result.svg).toContain("<svg");
  });

  test("a folder with a syntax error is skipped at discovery, not at render", async () => {
    const { skipped } = await discoverPlugins(fixture());
    expect(skipped.find((entry) => entry.folder === "syntax-error")?.reason).toMatch(
      /plan|syntax|parse/i,
    );
  });

  test("a missing plugins directory is no plugins, not an error", async () => {
    await expect(discoverPlugins(join(tmpdir(), "definitely-not-here"))).resolves.toEqual({
      faces: [],
      skipped: [],
    });
  });

  test("a discovered face carries the manifest's tap and refreshSeconds", async () => {
    const root = join(tmpdir(), `plugins-${Math.random().toString(36).slice(2)}`);
    mkdirSync(join(root, "clicker"), { recursive: true });
    writeFileSync(
      join(root, "clicker", "plugin.json"),
      JSON.stringify({
        api: 1,
        id: "clicker",
        version: "1.0.0",
        label: "Clicker",
        description: "d",
        author: "a",
        hosts: [],
        secrets: [],
        fields: [],
        tap: "Tap to advance.",
        refreshSeconds: 120,
      }),
    );
    writeFileSync(
      join(root, "clicker", "index.js"),
      `export function plan(){ return []; }
       export function render(){ return { layout: { type: "div", style: { display: "flex", width: 448, height: 368, background: "#000" }, children: "ok" } }; }`,
    );
    const { faces } = await discoverPlugins(root);
    expect(faces[0]?.tap).toBe("Tap to advance.");
    expect(faces[0]?.refreshSeconds).toBe(120);
  });

  test("a plugin's own render sees the timezone the render request sent", async () => {
    const root = join(tmpdir(), `plugins-${Math.random().toString(36).slice(2)}`);
    mkdirSync(join(root, "clock"), { recursive: true });
    writeFileSync(
      join(root, "clock", "plugin.json"),
      JSON.stringify({
        api: 1,
        id: "clock",
        version: "1.0.0",
        label: "Clock",
        description: "d",
        author: "a",
        hosts: [],
        secrets: [],
        fields: [],
      }),
    );
    writeFileSync(
      join(root, "clock", "index.js"),
      `export function plan(){ return []; }
       export function render(c){ return { layout: { type: "div", style: { display: "flex", width: 448, height: 368, background: "#000" }, children: "x" }, state: c.now.timezone }; }`,
    );
    const { faces } = await discoverPlugins(root);
    const result = await firstFace(faces).render({}, new Date("2026-09-23T10:00:00Z"), {
      timezone: "Asia/Tokyo",
    });
    expect(typeof result === "string" ? undefined : result.state).toBe("Asia/Tokyo");
  });

  test("a discovered plugin without a sent timezone still renders (falls back, not throws)", async () => {
    const { faces } = await discoverPlugins(fixture());
    await expect(
      firstFace(faces).render({}, new Date("2026-09-23T10:00:00Z"), {}),
    ).resolves.toBeDefined();
  });
});
