import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ConfigurationError } from "../../src/face";
import { readPluginSecrets } from "../../src/plugins/secrets";

let dir: string;
let previous: string | undefined;

beforeEach(() => {
  dir = join(tmpdir(), `secrets-${Math.random().toString(36).slice(2)}`);
  mkdirSync(dir, { recursive: true });
  previous = process.env.DESKMATE_CONFIG_DIR;
  process.env.DESKMATE_CONFIG_DIR = dir;
});

afterEach(() => {
  if (previous === undefined) delete process.env.DESKMATE_CONFIG_DIR;
  else process.env.DESKMATE_CONFIG_DIR = previous;
  rmSync(dir, { recursive: true, force: true });
});

describe("readPluginSecrets", () => {
  test("no DESKMATE_CONFIG_DIR is no secrets, not an error", async () => {
    delete process.env.DESKMATE_CONFIG_DIR;
    expect(await readPluginSecrets("github-stats")).toEqual({});
  });

  test("no plugin-secrets.json file is no secrets, not an error", async () => {
    expect(await readPluginSecrets("github-stats")).toEqual({});
  });

  test("a plugin with no entry in an existing file gets no secrets", async () => {
    writeFileSync(
      join(dir, "plugin-secrets.json"),
      JSON.stringify({ "other-plugin": { key: "value" } }),
    );
    expect(await readPluginSecrets("github-stats")).toEqual({});
  });

  test("a plugin gets only its own entry, never another's", async () => {
    writeFileSync(
      join(dir, "plugin-secrets.json"),
      JSON.stringify({
        "github-stats": { github_token: "ghp_abc" },
        "other-plugin": { other_key: "should-not-leak" },
      }),
    );
    const secrets = await readPluginSecrets("github-stats");
    expect(secrets).toEqual({ github_token: "ghp_abc" });
    expect(JSON.stringify(secrets)).not.toContain("should-not-leak");
  });

  test("malformed JSON is a ConfigurationError naming the file", async () => {
    writeFileSync(join(dir, "plugin-secrets.json"), "{not json");
    await expect(readPluginSecrets("github-stats")).rejects.toThrow(ConfigurationError);
    await expect(readPluginSecrets("github-stats")).rejects.toThrow(/plugin-secrets\.json/);
  });

  test("a non-string secret value is a ConfigurationError", async () => {
    writeFileSync(
      join(dir, "plugin-secrets.json"),
      JSON.stringify({ "github-stats": { github_token: 12345 } }),
    );
    await expect(readPluginSecrets("github-stats")).rejects.toThrow(ConfigurationError);
  });

  test("a top-level array is a ConfigurationError", async () => {
    writeFileSync(join(dir, "plugin-secrets.json"), JSON.stringify([1, 2, 3]));
    await expect(readPluginSecrets("github-stats")).rejects.toThrow(ConfigurationError);
  });
});
