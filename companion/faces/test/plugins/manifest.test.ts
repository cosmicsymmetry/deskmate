import { describe, expect, test } from "bun:test";
import type { FieldSpec } from "../../src/face";
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

  test.each([
    ["null field", null],
    ["array field", []],
    ["unknown type", { type: "number", key: "n", label: "N" }],
    ["missing key", { type: "text", label: "L", placeholder: "p" }],
    ["non-string label", { type: "url", key: "k", label: 2, placeholder: "p" }],
    ["missing text placeholder", { type: "text", key: "k", label: "L" }],
    ["non-string url placeholder", { type: "url", key: "k", label: "L", placeholder: null }],
    [
      "non-string text default",
      { type: "text", key: "k", label: "L", placeholder: "p", default: 5 },
    ],
    [
      "non-string url default",
      { type: "url", key: "k", label: "L", placeholder: "p", default: null },
    ],
    ["missing enum default", { type: "enum", key: "k", label: "L", options: [] }],
    ["missing enum options", { type: "enum", key: "k", label: "L", default: "a" }],
    ["non-array enum options", { type: "enum", key: "k", label: "L", default: "a", options: {} }],
    ["null option", { type: "enum", key: "k", label: "L", default: "a", options: [null] }],
    [
      "non-string option value",
      { type: "enum", key: "k", label: "L", default: "a", options: [{ value: 1, label: "A" }] },
    ],
    [
      "missing option label",
      { type: "enum", key: "k", label: "L", default: "a", options: [{ value: "a" }] },
    ],
  ])("refuses a malformed catalog field: %s", (_name, field) => {
    expect(() => parseManifest({ ...valid, fields: [field] }, "github-stats")).toThrow(
      ManifestError,
    );
  });

  test.each([-1, 1e20, 2 ** 64, 0.5, NaN, Infinity])(
    "refuses refreshSeconds outside u64: %s",
    (refreshSeconds) => {
      expect(() => parseManifest({ ...valid, refreshSeconds }, "github-stats")).toThrow(
        /refreshSeconds/,
      );
    },
  );

  test.each([0, 1e16, 2 ** 64 - 2048])("preserves a valid refreshSeconds: %s", (refreshSeconds) => {
    expect(parseManifest({ ...valid, refreshSeconds }, "github-stats").refreshSeconds).toBe(
      refreshSeconds,
    );
  });

  test("preserves field strings, duplicate keys, empty options and optional defaults", () => {
    const fields: FieldSpec[] = [
      { type: "text", key: " spaced ", label: "", placeholder: "  " },
      { type: "url", key: " spaced ", label: " ", placeholder: "", default: " url " },
      { type: "enum", key: "", label: "", default: "", options: [] },
      {
        type: "enum",
        key: "enum",
        label: " L ",
        default: " a ",
        options: [{ value: " a ", label: "" }],
      },
    ];
    expect(parseManifest({ ...valid, fields }, "github-stats").fields).toEqual(fields);
  });

  test("secret strings keep their trimming, exact errors and validation order", () => {
    const secret = { ...valid.secrets[0], key: " token ", label: " Token " };
    expect(parseManifest({ ...valid, secrets: [secret] }, "github-stats").secrets[0]).toMatchObject(
      { key: "token", label: "Token" },
    );
    expect(() =>
      parseManifest({ ...valid, secrets: [{ ...secret, key: " ", label: " " }] }, "github-stats"),
    ).toThrow("secret key must be a non-empty string");
    expect(() =>
      parseManifest(
        { ...valid, secrets: [{ ...secret, label: " ", kind: "bad" }] },
        "github-stats",
      ),
    ).toThrow("secret label must be a non-empty string");
  });
});
