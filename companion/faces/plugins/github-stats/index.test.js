import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseManifest } from "../../src/plugins/manifest";
import { runPlugin } from "../../src/plugins/run";
import { warmSandbox } from "../../src/plugins/sandbox";

test("GitHub stats sends one bearer prefix for an operator-configured token", async () => {
  await warmSandbox();
  const source = readFileSync(join(import.meta.dir, "index.js"), "utf8");
  const manifest = parseManifest(
    JSON.parse(readFileSync(join(import.meta.dir, "plugin.json"), "utf8")),
    "github-stats",
  );
  const requests = [];
  await runPlugin({
    source,
    manifest,
    settings: { user: "octocat" },
    now: new Date("2026-10-01T12:00:00Z"),
    timezone: "UTC",
    secrets: { github_token: "synthetic-fixture-token" },
    request: async (input) => {
      requests.push(input);
      const json = { public_repos: 8, followers: 42, following: 3 };
      return { status: 200, body: JSON.stringify(json), json };
    },
  });
  expect(requests).toHaveLength(1);
  expect(requests[0].url).toBe("https://api.github.com/users/octocat");
  expect(requests[0].headers.Authorization).toBe("Bearer synthetic-fixture-token");
});
