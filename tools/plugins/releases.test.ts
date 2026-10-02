// Repository-only checks: the deployed faces bundle contains neither docs nor deploy.sh.
import { afterEach, expect, test } from "bun:test";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DIRECTORY_PATH, directoryMarkdown } from "../../companion/faces/src/author/directory";
import { hashPlugin } from "../../companion/faces/src/plugins/releases";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

test("the plugin directory is generated from every manifest and each README exists", () => {
  expect(readFileSync(DIRECTORY_PATH, "utf8")).toBe(directoryMarkdown());
});

test("the isolated deploy preflight stops a mismatch before gates or VM commands", () => {
  const root = mkdtempSync(join(tmpdir(), "plugin-release-"));
  roots.push(root);
  mkdirSync(join(root, "sample"));
  writeFileSync(
    join(root, "sample/plugin.json"),
    JSON.stringify({
      api: 1,
      id: "sample",
      version: "1.0.0",
      label: "Sample",
      description: "A card",
      author: "Test",
      hosts: [],
      secrets: [],
      fields: [],
    }),
  );
  writeFileSync(join(root, "sample/index.js"), "reviewed code");
  writeFileSync(join(root, "releases.json"), JSON.stringify([hashPlugin(root, "sample")]));
  writeFileSync(join(root, "sample/index.js"), "changed without a version bump");
  const harness = mkdtempSync(join(tmpdir(), "faces-deploy-preflight-"));
  roots.push(harness);
  mkdirSync(join(harness, "companion/faces"), { recursive: true });
  mkdirSync(join(harness, "bin"));
  // Only extract the faces function. Never run deploy.sh (even its dry-run syncs).
  const deploy = readFileSync(
    join(import.meta.dir, "../../companion/crates/server/deploy/deploy.sh"),
    "utf8",
  );
  const start = deploy.indexOf("ship_faces() {");
  const functionBody = deploy.slice(start, deploy.indexOf("\n}\n", start) + 3);
  const wrapper = join(harness, "bin/bun");
  writeFileSync(
    wrapper,
    `#!/bin/sh
if [ "$1" = run ] && [ "$2" = src/author/releases.ts ]; then
  exec "$REAL_BUN" run "$RELEASE_CLI" "$3" "$PLUGIN_TEST_ROOT"
fi
touch "$AFTER_PREFLIGHT"
exit 99
`,
  );
  chmodSync(wrapper, 0o700);
  const marker = join(harness, "past-preflight");
  const result = Bun.spawnSync(
    [
      "/bin/sh",
      "-c",
      `set -eu
say() { :; }
ssh() { touch "$AFTER_PREFLIGHT"; exit 99; }
rsync() { touch "$AFTER_PREFLIGHT"; exit 99; }
${functionBody}
ship_faces
`,
    ],
    {
      cwd: harness,
      env: {
        ...process.env,
        PATH: `${join(harness, "bin")}:${process.env.PATH}`,
        REAL_BUN: process.execPath,
        RELEASE_CLI: join(import.meta.dir, "../../companion/faces/src/author/releases.ts"),
        PLUGIN_TEST_ROOT: root,
        AFTER_PREFLIGHT: marker,
      },
      timeout: 10_000,
    },
  );
  expect(result.exitCode).toBe(1);
  expect(result.stderr.toString()).toContain("sample@1.0.0: hash mismatch");
  expect(() => readFileSync(marker)).toThrow();
});

test("staged package checks use a temporary empty policy and leave instance policy alone", () => {
  const root = mkdtempSync(join(tmpdir(), "faces-stage-policy-"));
  roots.push(root);
  const operator = join(root, "operator.json");
  writeFileSync(operator, "unreadable operator policy");
  const marker = join(root, "policy-path");
  const bun = join(root, "bun");
  writeFileSync(
    bun,
    `#!/bin/sh
set -eu
if [ "$1" = test ] || [ "$2" = src/main.ts ]; then
  [ "$(cat "$DESKMATE_PLUGIN_DENYLIST")" = '[]' ] || exit 19
  printf '%s' "$DESKMATE_PLUGIN_DENYLIST" > "$POLICY_MARKER"
fi
`,
  );
  chmodSync(bun, 0o700);
  const deploy = readFileSync(
    join(import.meta.dir, "../../companion/crates/server/deploy/deploy.sh"),
    "utf8",
  );
  const start = deploy.indexOf('\tssh "$VM" "set -e\n\t\tcd /tmp/deskmate-faces');
  const end = '\n\t\t$BUN run src/author/releases.ts verify"';
  expect(start).toBeGreaterThan(0);
  const stage = deploy
    .slice(start, deploy.indexOf(end, start) + end.length)
    .replace("cd /tmp/deskmate-faces", 'cd \\"\\$STAGED_FACES\\"');
  // Execute only the captured staging body locally. ssh is a shell stub; no VM
  // contact, installation, service restart or live policy edit is possible.
  const result = Bun.spawnSync(
    ["/bin/sh", "-c", `set -eu\nssh() { /bin/sh -c "$2"; }\n${stage}`],
    {
      env: {
        ...process.env,
        VM: "local-stub",
        BUN: bun,
        STAGED_FACES: root,
        POLICY_MARKER: marker,
        DESKMATE_PLUGIN_DENYLIST: operator,
      },
      timeout: 10_000,
    },
  );
  expect(result.exitCode, result.stderr.toString()).toBe(0);
  const policy = readFileSync(marker, "utf8");
  expect(policy).not.toBe(operator);
  expect(() => readFileSync(policy)).toThrow(); // The EXIT trap cleaned it up.
  expect(readFileSync(operator, "utf8")).toBe("unreadable operator policy");
});
