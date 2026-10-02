import { expect, test } from "bun:test";
import {
  chmodSync,
  cpSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import {
  hashPlugin,
  parseReleases,
  readReleases,
  releaseStatus,
  verifyReleases,
} from "../../src/plugins/releases";
import { minimalManifest, tempPlugins, writePluginFolder } from "./test_support";

const temporaryRoot = tempPlugins();
const manifest = minimalManifest("sample", {
  label: "Sample",
  description: "A card",
  author: "Test",
});
function fixture() {
  const root = temporaryRoot("plugin-release-");
  mkdirSync(join(root, "sample", "assets"), { recursive: true });
  writePluginFolder(root, "sample", manifest, "export function plan(){ return []; }");
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
  for (const mode of ["check", "verify"]) {
    const result = Bun.spawnSync([
      process.execPath,
      "run",
      join(import.meta.dir, "../../src/author/releases.ts"),
      mode,
      root,
    ]);
    expect(result.exitCode).toBe(mode === "check" ? 0 : 1);
    expect(Buffer.concat([result.stdout, result.stderr]).toString()).toContain(
      mode === "check"
        ? "not yet in the release index"
        : "sample@2.0.0: missing release index entry",
    );
  }
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
  const output = temporaryRoot("release-evidence-");
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

test("adding a dotfile changes the reviewed content", () => {
  const root = fixture();
  writeFileSync(join(root, "sample/.settings"), "new hidden asset");
  expect(() => verifyReleases(root)).toThrow("hash mismatch");
});

test("changing an existing dotfile changes the reviewed content", () => {
  const root = fixture();
  writeFileSync(join(root, "sample/.settings"), "reviewed hidden asset");
  writeFileSync(join(root, "releases.json"), JSON.stringify([hashPlugin(root, "sample")]));
  writeFileSync(join(root, "sample/.settings"), "changed hidden asset");
  expect(() => verifyReleases(root)).toThrow("hash mismatch");
});

for (const operation of ["remove", "rename"]) {
  test(`a nested asset ${operation} changes the reviewed content`, () => {
    const root = fixture();
    const directory = join(root, "sample/assets/nested/deep");
    mkdirSync(directory, { recursive: true });
    const asset = join(directory, "shape.svg");
    writeFileSync(asset, "<svg/>");
    writeFileSync(join(root, "releases.json"), JSON.stringify([hashPlugin(root, "sample")]));
    if (operation === "remove") rmSync(asset);
    else renameSync(asset, join(directory, "different.svg"));
    expect(() => verifyReleases(root)).toThrow("hash mismatch");
  });
}

test("LF and CRLF are different reviewed bytes", () => {
  const root = fixture();
  const file = join(root, "sample/index.js");
  writeFileSync(file, `${readFileSync(file, "utf8")}\n`);
  writeFileSync(join(root, "releases.json"), JSON.stringify([hashPlugin(root, "sample")]));
  writeFileSync(file, readFileSync(file, "utf8").replace(/\n/g, "\r\n"));
  expect(() => verifyReleases(root)).toThrow("hash mismatch");
});

test("canonical identity uses UTF-8 byte ordering and lengths for non-ASCII paths", () => {
  const root = fixture();
  // U+E000 sorts before the emoji in UTF-8, but after it in UTF-16. Creation
  // order is deliberately different. The fixed digest is a documented-format
  // vector independently calculated with Python hashlib, not this hasher.
  writeFileSync(join(root, "sample/😀.txt"), "emoji\n");
  writeFileSync(join(root, "sample/Ω.txt"), "π\n");
  writeFileSync(join(root, "sample/\ue000.txt"), "private-use\n");
  expect(hashPlugin(root, "sample").sha256).toBe(
    "de3c6cbcc14ffaa9b3315efa841778ed20db62c21c1160bae340302c73a655db",
  );
});

test("symlinked assets, folders and index cannot hide unreviewed bytes", () => {
  const root = fixture();
  const container = temporaryRoot("linked-plugins-");
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
