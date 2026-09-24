import { describe, expect, test } from "bun:test";
import { ManifestError, parseManifest } from "../../src/plugins/manifest";

const valid = {
  api: 1,
  id: "github-stats",
  version: "1.0.0",
  label: "GitHub stats",
  description: "Commits and stars.",
  author: "Acme",
  hosts: ["api.github.com"],
  secrets: [
    {
      key: "token",
      label: "GitHub token",
      kind: "api_key",
      host: "api.github.com",
      send_as: "bearer",
    },
  ],
  fields: [{ type: "text", key: "user", label: "Username", placeholder: "octocat" }],
};

describe("parseManifest", () => {
  test("accepts a complete manifest", () => {
    expect(parseManifest(valid, "github-stats").id).toBe("github-stats");
  });

  test("refuses an id that does not match its folder", () => {
    expect(() => parseManifest(valid, "other")).toThrow(ManifestError);
  });

  test("refuses an api other than 1", () => {
    expect(() => parseManifest({ ...valid, api: 2 }, "github-stats")).toThrow(/api/);
  });

  test("refuses a host that is not a bare hostname", () => {
    for (const host of ["https://api.github.com", "api.github.com/x", "*", "10.0.0.1", ""]) {
      expect(() => parseManifest({ ...valid, hosts: [host] }, "github-stats")).toThrow(
        ManifestError,
      );
    }
  });

  test("refuses a secret whose host is not declared", () => {
    const secrets = [{ ...valid.secrets[0], host: "evil.example" }];
    expect(() => parseManifest({ ...valid, secrets }, "github-stats")).toThrow(/declared/);
  });

  test("refuses a plugin with a secret that declares more than that secret's hosts", () => {
    // The spec's rule: a plugin using a credential may reach only that credential's hosts.
    const hosts = ["api.github.com", "telemetry.example"];
    expect(() => parseManifest({ ...valid, hosts }, "github-stats")).toThrow(/only the hosts/);
  });

  test("keeps an absent refreshSeconds absent, and refuses a non-integer one", () => {
    expect(parseManifest(valid, "github-stats").refreshSeconds).toBeUndefined();
    expect(() => parseManifest({ ...valid, refreshSeconds: "900" }, "github-stats")).toThrow(
      ManifestError,
    );
  });
});
