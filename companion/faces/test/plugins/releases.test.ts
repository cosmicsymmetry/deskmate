import { afterEach, expect, test } from "bun:test";
import {
  chmodSync,
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  hashPlugin,
  parseReleases,
  readReleases,
  releaseStatus,
  verifyReleases,
} from "../../src/plugins/releases";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});
const manifest = {
  api: 1,
  id: "sample",
  version: "1.0.0",
  label: "Sample",
  description: "A card",
  author: "Test",
  hosts: [],
  secrets: [],
  fields: [],
};
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "plugin-release-"));
  roots.push(root);
  mkdirSync(join(root, "sample", "assets"), { recursive: true });
  writeFileSync(join(root, "sample", "plugin.json"), JSON.stringify(manifest));
  writeFileSync(join(root, "sample", "index.js"), "export function plan(){ return []; }");
  writeFileSync(join(root, "sample", "assets", "image.svg"), "<svg/>");
  writeFileSync(join(root, "sample", "README.md"), "Read me");
  writeFileSync(join(root, "sample", "index.test.js"), "// Test evidence");
  writeFileSync(join(root, "releases.json"), JSON.stringify([hashPlugin(root, "sample")]));
  return root;
}

test("reviewed bytes verify through the exact deployment CLI", () => {
  const root = fixture();
  const result = Bun.spawnSync([
    process.execPath,
    "run",
    join(import.meta.dir, "../../src/author/releases.ts"),
    "verify",
    root,
  ]);
  expect(result.exitCode, result.stderr.toString()).toBe(0);
  expect(result.stdout.toString()).toContain("sample@1.0.0: reviewed hash verified");
});

for (const file of ["index.js", "plugin.json", "assets/image.svg", "README.md", "index.test.js"]) {
  test(`changing ${file} under an unchanged version is refused`, () => {
    const root = fixture();
    const path = join(root, "sample", file);
    writeFileSync(path, `${readFileSync(path, "utf8")}\n`);
    expect(() => verifyReleases(root)).toThrow("sample@1.0.0: hash mismatch");
    // Ordinary CI must not silently approve a reused version either.
    expect(() => verifyReleases(root, true)).toThrow("bump the version");
  });
}

test("new version is informational in CI and refused for installation until reviewed", () => {
  const root = fixture();
  writeFileSync(
    join(root, "sample", "plugin.json"),
    JSON.stringify({ ...manifest, version: "2.0.0" }),
  );
  expect(verifyReleases(root, true)[0]).toContain("not yet in the release index");
  expect(() => verifyReleases(root)).toThrow("sample@2.0.0: missing release index entry");
  const old = readReleases(root);
  writeFileSync(join(root, "releases.json"), JSON.stringify([...old, hashPlugin(root, "sample")]));
  expect(verifyReleases(root)[0]).toContain("reviewed hash verified");
  expect(readReleases(root)).toHaveLength(2);
});

test("permission changes require a new reviewed version", () => {
  const root = fixture();
  writeFileSync(
    join(root, "sample/plugin.json"),
    JSON.stringify({ ...manifest, hosts: ["api.example.com"] }),
  );
  expect(() => verifyReleases(root)).toThrow("sample@1.0.0: hash mismatch");
});

test("the author checker prints the shared identity and ignores instance withdrawal policy", () => {
  const output = mkdtempSync(join(tmpdir(), "release-evidence-"));
  roots.push(output);
  const policy = join(output, "operator.json");
  writeFileSync(policy, "malformed policy");
  const result = Bun.spawnSync(
    [
      process.execPath,
      "run",
      join(import.meta.dir, "../../src/author/check.ts"),
      "github-stats",
      "--out",
      output,
    ],
    {
      env: { ...process.env, DESKMATE_PLUGIN_DENYLIST: policy },
      timeout: 10_000,
    },
  );
  expect(result.exitCode, result.stderr.toString()).toBe(0);
  const release = hashPlugin(join(import.meta.dir, "../../plugins"), "github-stats");
  expect(result.stdout.toString()).toContain(JSON.stringify(release));
  expect(
    JSON.parse(readFileSync(join(output, "github-stats/report.json"), "utf8")).release,
  ).toEqual(release);
}, 30_000);

test("file names and added files are covered; creation order and chmod are irrelevant", () => {
  const root = fixture();
  const approved = hashPlugin(root, "sample");
  chmodSync(join(root, "sample", "index.js"), 0o755);
  const bytes = readFileSync(join(root, "sample", "assets/image.svg"));
  rmSync(join(root, "sample", "assets/image.svg"));
  writeFileSync(join(root, "sample", "assets/image.svg"), bytes);
  expect(hashPlugin(root, "sample")).toEqual(approved);
  renameSync(join(root, "sample", "assets/image.svg"), join(root, "sample", "assets/other.svg"));
  expect(hashPlugin(root, "sample").sha256).not.toBe(approved.sha256);
  expect(() => verifyReleases(root)).toThrow("hash mismatch");
  writeFileSync(join(root, "sample", "extra.js"), "extra executable");
  expect(() => verifyReleases(root)).toThrow("hash mismatch");
});

test("symlinked assets, folders and index cannot hide unreviewed bytes", () => {
  const root = fixture();
  const container = mkdtempSync(join(tmpdir(), "linked-plugins-"));
  roots.push(container);
  symlinkSync(root, join(container, "plugins"));
  expect(() => verifyReleases(join(container, "plugins"))).toThrow("real directory");
  symlinkSync("image.svg", join(root, "sample", "assets/link.svg"));
  expect(() => hashPlugin(root, "sample")).toThrow("symlinks are not released");
  rmSync(join(root, "sample", "assets/link.svg"));
  symlinkSync("sample", join(root, "alias"));
  expect(() => verifyReleases(root)).toThrow("symlink");
  rmSync(join(root, "alias"));
  renameSync(join(root, "releases.json"), join(root, "other.json"));
  symlinkSync("other.json", join(root, "releases.json"));
  expect(() => verifyReleases(root)).toThrow("symlink");
  expect(() => readReleases(root)).toThrow("regular file");
});

test("strict index parsing rejects ambiguous approvals and a missing index fails deployment", () => {
  const root = fixture();
  const release = hashPlugin(root, "sample");
  expect(() => parseReleases([release, release])).toThrow("Duplicate reviewed release");
  for (const raw of [{}, [null], [{ ...release, sha256: "bad" }], [{ ...release, extra: true }]])
    expect(() => parseReleases(raw)).toThrow();
  expect(() => releaseStatus({ ...release, sha256: "0".repeat(64) }, [release])).toThrow(
    "hash mismatch",
  );
  rmSync(join(root, "releases.json"));
  expect(() => verifyReleases(root)).toThrow();
});

test("installation verifies every folder, not just plugins already named by the index", () => {
  const root = fixture();
  cpSync(join(root, "sample"), join(root, "new-plugin"), { recursive: true });
  writeFileSync(
    join(root, "new-plugin", "plugin.json"),
    JSON.stringify({ ...manifest, id: "new-plugin" }),
  );
  expect(() => verifyReleases(root)).toThrow("new-plugin@1.0.0: missing release index entry");
  expect(
    verifyReleases(root, true).some((line) => line.includes("not yet in the release index")),
  ).toBe(true);
});
