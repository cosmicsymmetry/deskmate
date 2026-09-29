import { beforeAll, describe, expect, test } from "bun:test";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { FaceDefinition } from "../../src/face";
import { discoverPlugins } from "../../src/plugins/discovery";
import { parseManifest } from "../../src/plugins/manifest";
import { runPlugin } from "../../src/plugins/run";
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

  describe("the shipped github-stats example", () => {
    // The real folder, not a throwaway fixture -- `docs/plugins/contract-v1.md`
    // points authors here as the copy-this-pattern example of recording an answer
    // and comparing the rendered card with no network involved.
    const root = join(import.meta.dir, "..", "..", "plugins", "github-stats");
    const manifest = parseManifest(
      JSON.parse(readFileSync(join(root, "plugin.json"), "utf8")),
      "github-stats",
    );
    const source = readFileSync(join(root, "index.js"), "utf8");

    test("fetches exactly once per render, not once per round", async () => {
      const seen: string[] = [];
      const request = async (input: { url: string }) => {
        seen.push(input.url);
        return {
          status: 200,
          body: '{"public_repos":42,"followers":1234,"following":56}',
          json: { public_repos: 42, followers: 1234, following: 56 },
        };
      };
      const result = await runPlugin({
        manifest,
        source,
        settings: { user: "octocat" },
        now: new Date("2026-09-23T10:00:00Z"),
        timezone: "UTC",
        secrets: {},
        request,
      });
      // A plan() that ignores context.answers keeps declaring the same request
      // until the 3-round cap stops it -- three GitHub requests for one render.
      // This plugin's plan() returns [] once it has an answer, so there is one.
      expect(seen).toEqual(["https://api.github.com/users/octocat"]);
      // Not "@octocat": satori splits a string into one <text> run per contiguous
      // segment (Task 9's report hit the same thing with "Asia/Tokyo"), and "@"
      // starts a new one -- "octocat" alone still pins the username was drawn.
      expect(result.svg).toContain("octocat");
      expect(result.svg).toContain("42");
      expect(result.svg).toContain("1.2K");
      expect(result.svg).toContain("56");
    });

    test("draws its placeholder dashes when the request fails, still in one round", async () => {
      const seen: string[] = [];
      const request = async (input: { url: string }) => {
        seen.push(input.url);
        return { status: 404, body: "not found" };
      };
      const result = await runPlugin({
        manifest,
        source,
        settings: {},
        now: new Date("2026-09-23T10:00:00Z"),
        timezone: "UTC",
        secrets: {},
        request,
      });
      expect(seen).toEqual(["https://api.github.com/users/octocat"]);
      // "--" draws as two separate single-"-" <text> runs, the same per-run
      // splitting as above -- six dash glyphs (repos, followers, following,
      // each "--"), and none of the real numbers.
      expect(result.svg.match(/>-</g)?.length).toBe(6);
      expect(result.svg).not.toContain("42");
      expect(result.svg).not.toContain("1.2K");
    });
  });
});
