import { beforeAll, expect, test } from "bun:test";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { ConfigurationError } from "../../src/face";
import { discoverPlugins } from "../../src/plugins/discovery";
import { warmSandbox } from "../../src/plugins/sandbox";
import { minimalManifest, tempPlugins, writePluginFolder } from "./test_support";

beforeAll(warmSandbox);
const temporaryRoot = tempPlugins();
function fixture() {
  const root = temporaryRoot("plugin-denylist-");
  const plugins = join(root, "plugins");
  writePluginFolder(
    plugins,
    "sample",
    minimalManifest("sample", { label: "Sample", description: "A card", author: "Test" }),
    'export function plan(){ return []; }\nexport function render(){ return {svg:\'<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"/>\'}; }',
  );
  const path = join(root, "operator-denylist.json");
  const discover = () => discoverPlugins(plugins, { denylistPath: path });
  const deny = (entries: unknown) => writeFileSync(path, JSON.stringify(entries));
  return { root, plugins, path, discover, deny };
}

test("discovery observes id and version withdrawals and restores entries without a restart", async () => {
  const f = fixture();
  expect((await f.discover()).faces).toHaveLength(1);
  f.deny([{ id: "sample", version: "2.0.0", reason: "Later release" }]);
  expect((await f.discover()).faces).toHaveLength(1);
  f.deny([{ id: "sample", version: "1.0.0", reason: "Review in progress" }]);
  const denied = await f.discover();
  expect(denied.faces).toEqual([]);
  expect(denied.skipped[0]).toEqual({
    folder: "sample",
    withdrawn: true,
    reason: "withdrawn by the operator: Review in progress",
  });
  f.deny([{ id: "sample", reason: "All versions" }]);
  expect((await f.discover()).faces).toEqual([]);
  rmSync(f.path);
  expect((await f.discover()).faces).toHaveLength(1);
});

test("a retained face rechecks policy before secrets, planning or rendering on every refresh", async () => {
  const f = fixture();
  let secretReads = 0;
  const { faces } = await discoverPlugins(f.plugins, {
    denylistPath: f.path,
    readSecrets: async () => {
      secretReads++;
      return {};
    },
  });
  const face = faces[0];
  if (!face) throw new Error("fixture missing");
  await face.render({}, new Date());
  expect(secretReads).toBe(1);
  f.deny([{ id: "sample", reason: "Unsafe output" }]);
  await expect(face.render({}, new Date())).rejects.toThrow(ConfigurationError);
  await expect(face.render({}, new Date())).rejects.toThrow(
    "withdrawn by the operator: Unsafe output",
  );
  expect(secretReads).toBe(1);
  f.deny([]);
  await face.render({}, new Date());
  expect(secretReads).toBe(2);
});

test("malformed and unreadable files fail closed at discovery and on retained refresh", async () => {
  const f = fixture();
  const face = (await f.discover()).faces[0];
  if (!face) throw new Error("fixture missing");
  for (const text of [
    `[]${" ".repeat(256 * 1024)}`,
    '[{"id":"sample","version":"1.0.0 ","reason":"why"}]',
    "{",
    "{}",
    '[{"id":"sample"}]',
    '[{"id":"sample","reason":"why","typo":true}]',
  ]) {
    writeFileSync(f.path, text);
    expect((await f.discover()).faces).toEqual([]);
    await expect(face.render({}, new Date())).rejects.toThrow("denylist is unreadable");
  }
  rmSync(f.path);
  mkdirSync(f.path);
  expect((await f.discover()).skipped[0]?.reason).toContain("denylist is unreadable");
  await expect(face.render({}, new Date())).rejects.toThrow("denylist is unreadable");
});

test("withdrawn code is not evaluated even during discovery", async () => {
  const f = fixture();
  writeFileSync(join(f.plugins, "sample/index.js"), "this source cannot parse!");
  f.deny([{ id: "sample", version: "1.0.0", reason: "Do not execute" }]);
  expect((await f.discover()).skipped[0]?.reason).toBe("withdrawn by the operator: Do not execute");
});

test("unreadable policy withdraws even folders whose manifest can no longer parse", async () => {
  const f = fixture();
  writeFileSync(f.path, "bad policy");
  writeFileSync(join(f.plugins, "sample/plugin.json"), "bad manifest");
  expect((await f.discover()).skipped[0]).toEqual({
    folder: "sample",
    withdrawn: true,
    reason: "withdrawn by the operator: denylist is unreadable",
  });
});

test("live CLI hides withdrawn plugins through tombstones and refuses every render envelope; builtins remain available", () => {
  const f = fixture();
  const main = join(import.meta.dir, "../../src/main.ts");
  const invoke = (verb: string, request: object = {}) =>
    Bun.spawnSync([process.execPath, "run", main, verb], {
      env: {
        ...process.env,
        DESKMATE_PLUGINS_DIR: f.plugins,
        DESKMATE_PLUGIN_DENYLIST: f.path,
        DESKMATE_CONFIG_DIR: f.root,
      },
      stdin: Buffer.from(`${JSON.stringify(request)}\n`),
      timeout: 10_000,
    });
  expect(invoke("render", { kind: "sample" }).exitCode).toBe(0);
  for (const malformed of [false, true]) {
    if (malformed) writeFileSync(f.path, "unreadable JSON");
    else
      f.deny([
        { id: "sample", reason: "Safety review" },
        { id: "weather", reason: "Must not affect builtins" },
      ]);
    const described = invoke("describe");
    expect(described.exitCode, described.stderr.toString()).toBe(0);
    const catalog = JSON.parse(described.stdout.toString());
    expect(
      catalog
        .filter((entry: { withdrawn?: string }) => !entry.withdrawn)
        .map((entry: { kind: string }) => entry.kind),
    ).toEqual(["weather", "hackernews", "rss", "token"]);
    expect(catalog.find((entry: { kind: string }) => entry.kind === "sample").withdrawn).toContain(
      malformed ? "denylist is unreadable" : "Safety review",
    );
    for (const context of [{}, { event: { taps: 1 } }, { view: "alternate" }]) {
      const result = invoke("render", { kind: "sample", ...context });
      expect(result.exitCode, result.stderr.toString()).toBe(2);
      expect(result.stdout.toString()).toBe("");
      expect(result.stderr.toString()).toContain("withdrawn by the operator");
    }
    for (const verb of ["views", "tap"]) {
      const result = invoke(verb, { kind: "sample", event: { taps: 1 } });
      expect(result.exitCode, result.stderr.toString()).toBe(2);
      expect(result.stderr.toString()).toContain("withdrawn by the operator");
    }
    const warm = invoke("tap-worker", { kind: "sample", event: { taps: 1 } });
    expect(warm.exitCode, warm.stderr.toString()).toBe(0);
    expect(JSON.parse(warm.stdout.toString()).code).toBe(2);
    const builtin = invoke("views", { kind: "hackernews", settings: {} });
    expect(builtin.exitCode, builtin.stderr.toString()).toBe(0);
  }
  expect(readFileSync(f.path, "utf8")).toBe("unreadable JSON");
}, 120_000);
