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
    for (const host of ["https://api.github.com", "api.github.com/x", "*", ""]) {
      expect(() => parseManifest({ ...valid, hosts: [host], secrets: [] }, "github-stats")).toThrow(
        ManifestError,
      );
    }
  });

  test("refuses IP literals explicitly", () => {
    const cases = ["10.0.0.1", "192.168.1.50", "169.254.169.254"];
    for (const host of cases) {
      expect(() => parseManifest({ ...valid, hosts: [host], secrets: [] }, "github-stats")).toThrow(
        /IP literal/,
      );
    }
  });

  test("refuses bracketed IPv6 addresses", () => {
    expect(() =>
      parseManifest({ ...valid, hosts: ["[::1]"], secrets: [] }, "github-stats"),
    ).toThrow(/bracketed IPv6/);
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

  test("refuses malformed secret entries (TypeError → ManifestError)", () => {
    expect(() => parseManifest({ ...valid, secrets: [null] }, "github-stats")).toThrow(
      ManifestError,
    );
    expect(() => parseManifest({ ...valid, secrets: [{}] }, "github-stats")).toThrow(ManifestError);
  });

  test("refuses invalid SecretSpec fields", () => {
    // Invalid send_as
    expect(() =>
      parseManifest(
        {
          ...valid,
          secrets: [{ ...valid.secrets[0], send_as: "weird" }],
        },
        "github-stats",
      ),
    ).toThrow(/send_as/);

    // Invalid kind
    expect(() =>
      parseManifest(
        {
          ...valid,
          secrets: [{ ...valid.secrets[0], kind: "oauth_token" }],
        },
        "github-stats",
      ),
    ).toThrow(/kind/);

    // Missing key
    expect(() =>
      parseManifest(
        {
          ...valid,
          secrets: [{ ...valid.secrets[0], key: undefined }],
        },
        "github-stats",
      ),
    ).toThrow(/key/);

    // Missing label
    expect(() =>
      parseManifest(
        {
          ...valid,
          secrets: [{ ...valid.secrets[0], label: undefined }],
        },
        "github-stats",
      ),
    ).toThrow(/label/);
  });

  test("refuses non-array hosts, secrets, or fields", () => {
    expect(() => parseManifest({ ...valid, hosts: "api.github.com" }, "github-stats")).toThrow(
      /hosts must be an array/,
    );
    expect(() => parseManifest({ ...valid, secrets: "a-secret" }, "github-stats")).toThrow(
      /secrets must be an array/,
    );
    expect(() => parseManifest({ ...valid, fields: "a-field" }, "github-stats")).toThrow(
      /fields must be an array/,
    );
  });
});
