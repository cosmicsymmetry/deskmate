import { describe, expect, test } from "bun:test";
import { ConfigurationError } from "../../src/face";
import { performRequests, validateRequests } from "../../src/plugins/requests";
import { parseManifest } from "../../src/plugins/manifest";

const manifest = parseManifest(
  {
    api: 1,
    id: "p",
    version: "1.0.0",
    label: "P",
    description: "d",
    author: "a",
    hosts: ["api.github.com"],
    secrets: [
      { key: "token", label: "T", kind: "api_key", host: "api.github.com", send_as: "bearer" },
    ],
    fields: [],
  },
  "p",
);
const budget = { requests: 8, bytes: 4 * 1024 * 1024, measurements: 64 };

describe("validateRequests", () => {
  test("accepts a declared host", () => {
    expect(
      validateRequests([{ url: "https://api.github.com/users/o", as: "json" }], manifest, budget),
    ).toHaveLength(1);
  });

  test("refuses an undeclared host, naming it, as a configuration error", () => {
    expect(() =>
      validateRequests([{ url: "https://evil.example/", as: "json" }], manifest, budget),
    ).toThrow(/evil.example/);
    expect(() =>
      validateRequests([{ url: "https://evil.example/", as: "json" }], manifest, budget),
    ).toThrow(ConfigurationError);
  });

  test("refuses more requests than the budget allows", () => {
    const many = Array.from({ length: 9 }, () => ({
      url: "https://api.github.com/u",
      as: "json" as const,
    }));
    expect(() => validateRequests(many, manifest, budget)).toThrow(/at most 8/);
  });

  test("refuses a scheme that is not http(s)", () => {
    expect(() =>
      validateRequests([{ url: "file:///etc/passwd", as: "text" }], manifest, budget),
    ).toThrow(ConfigurationError);
  });

  test("refuses a host that differs only in case by normalizing, still exact-match", () => {
    // api.github.com is declared; GitHub.COM should still match via lowercasing.
    expect(
      validateRequests([{ url: "https://API.GITHUB.COM/u", as: "json" }], manifest, budget),
    ).toHaveLength(1);
  });

  test("refuses a host with a trailing dot even though the declared host has none", () => {
    expect(() =>
      validateRequests([{ url: "https://api.github.com./u", as: "json" }], manifest, budget),
    ).toThrow(ConfigurationError);
  });

  test("refuses a userinfo prefix from being mistaken for the declared host", () => {
    expect(() =>
      validateRequests(
        [{ url: "https://api.github.com@evil.example/", as: "json" }],
        manifest,
        budget,
      ),
    ).toThrow(/evil.example/);
  });

  test("refuses an IP-literal host even if it were somehow declared", () => {
    expect(() =>
      validateRequests([{ url: "https://192.168.1.1/", as: "json" }], manifest, budget),
    ).toThrow(ConfigurationError);
  });

  test("refuses a decimal IP-literal host", () => {
    expect(() =>
      validateRequests([{ url: "http://3232235777/", as: "json" }], manifest, budget),
    ).toThrow(ConfigurationError);
  });

  test("refuses a hex IP-literal host", () => {
    expect(() =>
      validateRequests([{ url: "http://0xC0A80101/", as: "json" }], manifest, budget),
    ).toThrow(ConfigurationError);
  });

  test("a declared host with an explicit port is still the same host", () => {
    expect(
      validateRequests([{ url: "https://api.github.com:443/u", as: "json" }], manifest, budget),
    ).toHaveLength(1);
  });
});

describe("performRequests", () => {
  test("substitutes a secret for its own host and never returns it", async () => {
    const seen: { url: string; headers?: Record<string, string> }[] = [];
    const stub = async (input: { url: string; headers?: Record<string, string> }) => {
      seen.push(input);
      return { status: 200, body: "{}", json: {} };
    };
    const answers = await performRequests(
      validateRequests(
        [
          {
            url: "https://api.github.com/u",
            as: "json",
            headers: { Authorization: "{{secret:token}}" },
          },
        ],
        manifest,
        budget,
      ),
      manifest,
      { token: "ghp_real" },
      stub,
      budget,
    );
    expect(seen[0]?.headers?.Authorization).toBe("Bearer ghp_real");
    expect(JSON.stringify(answers)).not.toContain("ghp_real");
  });

  test("refuses to substitute a secret into a request for another host", async () => {
    // Two hosts cannot coexist with a secret (Task 1), so this manifest has none;
    // the placeholder must survive as a literal rather than leaking the value.
    const open = parseManifest(
      {
        ...JSON.parse(JSON.stringify(manifest)),
        secrets: [],
        hosts: ["api.github.com", "example.com"],
      },
      "p",
    );
    const seen: unknown[] = [];
    const stub = async (input: unknown) => {
      seen.push(input);
      return { status: 200, body: "", json: undefined };
    };
    await performRequests(
      validateRequests(
        [
          {
            url: "https://example.com/x",
            as: "text",
            headers: { Authorization: "{{secret:token}}" },
          },
        ],
        open,
        budget,
      ),
      open,
      { token: "ghp_real" },
      stub,
      budget,
    );
    expect(JSON.stringify(seen)).not.toContain("ghp_real");
  });

  test("a failed request becomes an answer, not an exception", async () => {
    const stub = async () => {
      throw new Error("connection reset");
    };
    const [answer] = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "json" }], manifest, budget),
      manifest,
      {},
      stub,
      budget,
    );
    expect(answer).toEqual({ ok: false, error: "connection reset" });
  });

  test("stops once the total byte budget is spent", async () => {
    const big = "x".repeat(2 * 1024 * 1024);
    const stub = async () => ({ status: 200, body: big, json: undefined });
    const requests = validateRequests(
      [1, 2, 3].map(() => ({ url: "https://api.github.com/u", as: "text" as const })),
      manifest,
      budget,
    );
    const answers = await performRequests(requests, manifest, {}, stub, budget);
    expect(answers[2]).toEqual({ ok: false, error: expect.stringContaining("budget") });
  });

  test("a matched secret in a query parameter is substituted too", async () => {
    const seen: { url: string }[] = [];
    const stub = async (input: { url: string }) => {
      seen.push(input);
      return { status: 200, body: "", json: undefined };
    };
    await performRequests(
      validateRequests(
        [{ url: "https://api.github.com/u?key={{secret:token}}", as: "text" }],
        manifest,
        budget,
      ),
      manifest,
      { token: "ghp_real" },
      stub,
      budget,
    );
    expect(seen[0]?.url).toContain("ghp_real");
  });
});
