import { describe, expect, test } from "bun:test";
import { ConfigurationError } from "../../src/face";
import { INGEST_CAP_BYTES } from "../../src/kit/limits";
import { ResponseTooLargeError } from "../../src/kit/http";
import { performRequests, validateRequests } from "../../src/plugins/requests";
import { parseManifest } from "../../src/plugins/manifest";
import { textInk, textWidth } from "../../src/kit/raster";

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
    const { answers } = await performRequests(
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
    const {
      answers: [answer],
    } = await performRequests(
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
    const { answers } = await performRequests(requests, manifest, {}, stub, budget);
    expect(answers[2]).toEqual({ ok: false, error: expect.stringContaining("budget") });
  });

  test("scrubs a secret value that leaks back through a thrown error", async () => {
    const stub = async (input: { headers?: Record<string, string> }) => {
      throw new Error(`upstream rejected Authorization: ${input.headers?.Authorization}`);
    };
    const {
      answers: [answer],
    } = await performRequests(
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
      { token: "ghp_SECRETVALUE" },
      stub,
      budget,
    );
    expect(JSON.stringify(answer)).not.toContain("ghp_SECRETVALUE");
    expect(answer).toEqual({
      ok: false,
      error: expect.stringContaining("[redacted]"),
    });
  });
});

describe("performRequests: send_as decides the one position a secret may land in", () => {
  const positioned = parseManifest(
    {
      api: 1,
      id: "p",
      version: "1.0.0",
      label: "P",
      description: "d",
      author: "a",
      hosts: ["bearer.example", "header.example", "query.example"],
      secrets: [
        { key: "b", label: "B", kind: "api_key", host: "bearer.example", send_as: "bearer" },
        { key: "h", label: "H", kind: "api_key", host: "header.example", send_as: "header" },
        { key: "q", label: "Q", kind: "api_key", host: "query.example", send_as: "query" },
      ],
      fields: [],
    },
    "p",
  );
  const secrets = { b: "SECRET_B", h: "SECRET_H", q: "SECRET_Q" };

  test("a bearer secret substitutes as Bearer <value> in its own header", async () => {
    const seen: { headers?: Record<string, string> }[] = [];
    const stub = async (input: { headers?: Record<string, string> }) => {
      seen.push(input);
      return { status: 200, body: "", json: undefined };
    };
    await performRequests(
      validateRequests(
        [
          {
            url: "https://bearer.example/x",
            as: "text",
            headers: { Authorization: "{{secret:b}}" },
          },
        ],
        positioned,
        budget,
      ),
      positioned,
      secrets,
      stub,
      budget,
    );
    expect(seen[0]?.headers?.Authorization).toBe("Bearer SECRET_B");
  });

  test("a bearer secret's placeholder in a query parameter stays literal", async () => {
    const seen: { url: string }[] = [];
    const stub = async (input: { url: string }) => {
      seen.push(input);
      return { status: 200, body: "", json: undefined };
    };
    await performRequests(
      validateRequests(
        [{ url: "https://bearer.example/x?b={{secret:b}}", as: "text" }],
        positioned,
        budget,
      ),
      positioned,
      secrets,
      stub,
      budget,
    );
    expect(new URL(seen[0]?.url ?? "").searchParams.get("b")).toBe("{{secret:b}}");
    expect(seen[0]?.url).not.toContain("SECRET_B");
  });

  test("a header secret substitutes its raw value in its own header", async () => {
    const seen: { headers?: Record<string, string> }[] = [];
    const stub = async (input: { headers?: Record<string, string> }) => {
      seen.push(input);
      return { status: 200, body: "", json: undefined };
    };
    await performRequests(
      validateRequests(
        [{ url: "https://header.example/x", as: "text", headers: { "X-Api-Key": "{{secret:h}}" } }],
        positioned,
        budget,
      ),
      positioned,
      secrets,
      stub,
      budget,
    );
    expect(seen[0]?.headers?.["X-Api-Key"]).toBe("SECRET_H");
  });

  test("a query secret substitutes its raw value in its own query parameter", async () => {
    const seen: { url: string }[] = [];
    const stub = async (input: { url: string }) => {
      seen.push(input);
      return { status: 200, body: "", json: undefined };
    };
    await performRequests(
      validateRequests(
        [{ url: "https://query.example/x?k={{secret:q}}", as: "text" }],
        positioned,
        budget,
      ),
      positioned,
      secrets,
      stub,
      budget,
    );
    expect(new URL(seen[0]?.url ?? "").searchParams.get("k")).toBe("SECRET_Q");
  });

  test("a query secret's placeholder in a header stays literal", async () => {
    const seen: { headers?: Record<string, string> }[] = [];
    const stub = async (input: { headers?: Record<string, string> }) => {
      seen.push(input);
      return { status: 200, body: "", json: undefined };
    };
    await performRequests(
      validateRequests(
        [{ url: "https://query.example/x", as: "text", headers: { "X-Api-Key": "{{secret:q}}" } }],
        positioned,
        budget,
      ),
      positioned,
      secrets,
      stub,
      budget,
    );
    expect(seen[0]?.headers?.["X-Api-Key"]).toBe("{{secret:q}}");
  });

  test("cross-host refusal still holds: a secret never substitutes for another host", async () => {
    const seen: { headers?: Record<string, string> }[] = [];
    const stub = async (input: { headers?: Record<string, string> }) => {
      seen.push(input);
      return { status: 200, body: "", json: undefined };
    };
    // "b" belongs to bearer.example; declaring it here would fail manifest
    // validation (Task 1), so this proves the same thing from the request side:
    // header.example has no secret named "b" registered to it, so the
    // placeholder cannot resolve there even though "b" exists in this manifest.
    await performRequests(
      validateRequests(
        [
          {
            url: "https://header.example/x",
            as: "text",
            headers: { Authorization: "{{secret:b}}" },
          },
        ],
        positioned,
        budget,
      ),
      positioned,
      secrets,
      stub,
      budget,
    );
    expect(seen[0]?.headers?.Authorization).toBe("{{secret:b}}");
    expect(JSON.stringify(seen)).not.toContain("SECRET_B");
  });

  test("a query with no secret placeholder reaches the wire byte-for-byte", async () => {
    const seen: { url: string }[] = [];
    const stub = async (input: { url: string }) => {
      seen.push(input);
      return { status: 200, body: "", json: undefined };
    };
    const url = "https://bearer.example/x?q=a+b&raw=%2Fpath";
    await performRequests(
      validateRequests([{ url, as: "text" }], positioned, budget),
      positioned,
      secrets,
      stub,
      budget,
    );
    // validateRequests already canonicalizes the URL once (`new URL(url).toString()`);
    // compare against that canonical form, not the original literal, to isolate
    // whether performRequests itself mangles a placeholder-free query.
    expect(seen[0]?.url).toBe(new URL(url).toString());
  });
});

describe("performRequests: a 200 response that echoes the secret back is scrubbed too", () => {
  const stolen = "ghp_SECRETVALUE";

  test("scrubs a secret that leaks back in a 200 text response body", async () => {
    const stub = async (input: { headers?: Record<string, string> }) => ({
      status: 200,
      body: `echo: ${input.headers?.Authorization}`,
      json: undefined,
    });
    const {
      answers: [answer],
    } = await performRequests(
      validateRequests(
        [
          {
            url: "https://api.github.com/u",
            as: "text",
            headers: { Authorization: "{{secret:token}}" },
          },
        ],
        manifest,
        budget,
      ),
      manifest,
      { token: stolen },
      stub,
      budget,
    );
    expect(JSON.stringify(answer)).not.toContain(stolen);
    expect(answer).toEqual({ ok: true, status: 200, text: "echo: Bearer [redacted]" });
  });

  test("scrubs a secret nested two levels deep in a 200 json response body", async () => {
    const stub = async (input: { headers?: Record<string, string> }) => {
      const body = JSON.stringify({ data: { seen: { header: input.headers?.Authorization } } });
      return { status: 200, body, json: JSON.parse(body) };
    };
    const {
      answers: [answer],
    } = await performRequests(
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
      { token: stolen },
      stub,
      budget,
    );
    expect(JSON.stringify(answer)).not.toContain(stolen);
    expect(answer).toEqual({
      ok: true,
      status: 200,
      json: { data: { seen: { header: "Bearer [redacted]" } } },
    });
  });

  test("refuses a 200 bytes response whose bytes contain the secret's UTF-8 form", async () => {
    const stub = async () => ({
      status: 200,
      body: new TextEncoder().encode(`leaked: ${stolen}`),
      json: undefined,
    });
    const {
      answers: [answer],
    } = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "bytes" }], manifest, budget),
      manifest,
      { token: stolen },
      stub,
      budget,
    );
    expect(JSON.stringify(answer)).not.toContain(stolen);
    expect(answer).toEqual({
      ok: false,
      status: 200,
      error: expect.stringContaining("echoed"),
    });
  });

  test("refuses a 200 bytes response whose bytes contain the secret's base64 form", async () => {
    const asBase64 = Buffer.from(stolen, "utf-8").toString("base64");
    const stub = async () => ({
      status: 200,
      body: new TextEncoder().encode(`leaked: ${asBase64}`),
      json: undefined,
    });
    const {
      answers: [answer],
    } = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "bytes" }], manifest, budget),
      manifest,
      { token: stolen },
      stub,
      budget,
    );
    expect(JSON.stringify(answer)).not.toContain(stolen);
    expect(JSON.stringify(answer)).not.toContain(asBase64);
    expect(answer).toEqual({
      ok: false,
      status: 200,
      error: expect.stringContaining("echoed"),
    });
  });

  test("a normal text response with no secret in it is returned byte-for-byte", async () => {
    const plain = "hello world, nothing secret here";
    const stub = async () => ({ status: 200, body: plain, json: undefined });
    const {
      answers: [answer],
    } = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "text" }], manifest, budget),
      manifest,
      { token: stolen },
      stub,
      budget,
    );
    expect(answer).toEqual({ ok: true, status: 200, text: plain });
  });

  test("a normal json response with no secret in it is returned deep-equal", async () => {
    const plain = { a: 1, b: { c: "plain string", d: [1, 2, "three"] }, e: null, f: false };
    const stub = async () => ({ status: 200, body: JSON.stringify(plain), json: plain });
    const {
      answers: [answer],
    } = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "json" }], manifest, budget),
      manifest,
      { token: stolen },
      stub,
      budget,
    );
    expect(answer).toEqual({ ok: true, status: 200, json: plain });
  });

  test("an empty stored secret value does not mangle an ordinary body", async () => {
    const plain = "some ordinary text with no secrets, and no {{secret:token}} either";
    const stub = async () => ({ status: 200, body: plain, json: undefined });
    const {
      answers: [answer],
    } = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "text" }], manifest, budget),
      manifest,
      { token: "" },
      stub,
      budget,
    );
    expect(answer).toEqual({ ok: true, status: 200, text: plain });
  });

  test("scrubs a secret used as a json OBJECT KEY, at the top level", async () => {
    const stub = async () => {
      const body = JSON.stringify({ [stolen]: "some value", other: "fine" });
      return { status: 200, body, json: JSON.parse(body) };
    };
    const {
      answers: [answer],
    } = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "json" }], manifest, budget),
      manifest,
      { token: stolen },
      stub,
      budget,
    );
    expect(JSON.stringify(answer)).not.toContain(stolen);
    expect(answer).toEqual({
      ok: true,
      status: 200,
      json: { "[redacted]": "some value", other: "fine" },
    });
  });

  test("scrubs a secret used as a json object key, nested two levels down", async () => {
    const stub = async () => {
      const body = JSON.stringify({ data: { seen: { [stolen]: true } } });
      return { status: 200, body, json: JSON.parse(body) };
    };
    const {
      answers: [answer],
    } = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "json" }], manifest, budget),
      manifest,
      { token: stolen },
      stub,
      budget,
    );
    expect(JSON.stringify(answer)).not.toContain(stolen);
    expect(answer).toEqual({
      ok: true,
      status: 200,
      json: { data: { seen: { "[redacted]": true } } },
    });
  });
});

describe("validateRequests / performRequests: measure requests", () => {
  // A `RequestFn` that fails the test if it is ever called: a measure request must
  // perform no I/O at all, not merely "usually" skip the network.
  const noNetwork = async () => {
    throw new Error("no network for a measure");
  };

  test("answers a measure request with the renderer's own widths", async () => {
    const requests = validateRequests(
      [{ measure: [{ text: "Hello", size: 31, weight: 600 }] }],
      manifest,
      budget,
    );
    const {
      answers: [answer],
    } = await performRequests(requests, manifest, {}, noNetwork, budget);
    expect(answer).toEqual({
      ok: true,
      measurements: [{ width: textWidth("Hello", 31, 600), ink: expect.any(Object) }],
    });
  });

  test("the ink matches the renderer's own textInk for the same run", async () => {
    const requests = validateRequests(
      [{ measure: [{ text: "$12.34", size: 22, weight: 400 }] }],
      manifest,
      budget,
    );
    const {
      answers: [answer],
    } = await performRequests(requests, manifest, {}, noNetwork, budget);
    expect(answer).toEqual({
      ok: true,
      measurements: [{ width: textWidth("$12.34", 22, 400), ink: textInk("$12.34", 22, 400) }],
    });
  });

  test("answers several measurements in one request, in the order asked", async () => {
    const requests = validateRequests(
      [
        {
          measure: [
            { text: "one", size: 10, weight: 400 },
            { text: "two", size: 20, weight: 600 },
            { text: "three", size: 30, weight: 400 },
          ],
        },
      ],
      manifest,
      budget,
    );
    const {
      answers: [answer],
    } = await performRequests(requests, manifest, {}, noNetwork, budget);
    expect(answer).toEqual({
      ok: true,
      measurements: [
        { width: textWidth("one", 10, 400), ink: textInk("one", 10, 400) },
        { width: textWidth("two", 20, 600), ink: textInk("two", 20, 600) },
        { width: textWidth("three", 30, 400), ink: textInk("three", 30, 400) },
      ],
    });
  });

  test("an empty measure array is accepted and answers an empty measurements array", async () => {
    const requests = validateRequests([{ measure: [] }], manifest, budget);
    const {
      answers: [answer],
    } = await performRequests(requests, manifest, {}, noNetwork, budget);
    expect(answer).toEqual({ ok: true, measurements: [] });
  });

  test("an empty string measures as zero width, not a configuration error", async () => {
    const requests = validateRequests(
      [{ measure: [{ text: "", size: 20, weight: 400 }] }],
      manifest,
      budget,
    );
    const {
      answers: [answer],
    } = await performRequests(requests, manifest, {}, noNetwork, budget);
    expect(answer).toEqual({ ok: true, measurements: [{ width: 0, ink: expect.any(Object) }] });
  });

  test("a run with no coverage in the bundled fonts still answers, not throws", async () => {
    // The bundled Inter faces cover Latin text; an emoji is outside that coverage.
    // The renderer measures whatever it can rather than refusing -- there is no
    // owner action to take over a plugin's own dynamic text content.
    const requests = validateRequests(
      [{ measure: [{ text: "🎉", size: 20, weight: 400 }] }],
      manifest,
      budget,
    );
    const {
      answers: [answer],
    } = await performRequests(requests, manifest, {}, noNetwork, budget);
    expect(answer).toEqual({
      ok: true,
      measurements: [{ width: expect.any(Number), ink: expect.any(Object) }],
    });
  });

  test("refuses a non-string text as a configuration error", () => {
    expect(() =>
      validateRequests([{ measure: [{ text: 5, size: 20, weight: 400 }] }], manifest, budget),
    ).toThrow(ConfigurationError);
  });

  test("refuses a negative size as a configuration error", () => {
    expect(() =>
      validateRequests([{ measure: [{ text: "x", size: -1, weight: 400 }] }], manifest, budget),
    ).toThrow(ConfigurationError);
  });

  test("refuses a zero size as a configuration error", () => {
    expect(() =>
      validateRequests([{ measure: [{ text: "x", size: 0, weight: 400 }] }], manifest, budget),
    ).toThrow(ConfigurationError);
  });

  test("refuses a non-finite size as a configuration error", () => {
    expect(() =>
      validateRequests(
        [{ measure: [{ text: "x", size: Number.POSITIVE_INFINITY, weight: 400 }] }],
        manifest,
        budget,
      ),
    ).toThrow(ConfigurationError);
    expect(() =>
      validateRequests(
        [{ measure: [{ text: "x", size: Number.NaN, weight: 400 }] }],
        manifest,
        budget,
      ),
    ).toThrow(ConfigurationError);
  });

  test("refuses an enormous size as a configuration error", () => {
    expect(() =>
      validateRequests(
        [{ measure: [{ text: "x", size: 1_000_000, weight: 400 }] }],
        manifest,
        budget,
      ),
    ).toThrow(ConfigurationError);
  });

  test("refuses a weight the bundled fonts do not have", () => {
    expect(() =>
      validateRequests([{ measure: [{ text: "x", size: 20, weight: 500 }] }], manifest, budget),
    ).toThrow(ConfigurationError);
    expect(() =>
      validateRequests([{ measure: [{ text: "x", size: 20, weight: 700 }] }], manifest, budget),
    ).toThrow(ConfigurationError);
  });

  test("refuses a non-numeric weight as a configuration error", () => {
    expect(() =>
      validateRequests(
        // biome-ignore lint/suspicious/noExplicitAny: exercising a hostile, wrongly-typed input
        [{ measure: [{ text: "x", size: 20, weight: "bold" as any }] }],
        manifest,
        budget,
      ),
    ).toThrow(ConfigurationError);
  });

  test("refuses more measurements than the budget allows, in a single request", () => {
    const measure = Array.from({ length: 65 }, () => ({ text: "x", size: 17, weight: 400 }));
    expect(() => validateRequests([{ measure }], manifest, budget)).toThrow(/64/);
  });

  test("refuses more measurements than the budget allows, spread across requests", () => {
    const first = Array.from({ length: 40 }, () => ({ text: "x", size: 17, weight: 400 }));
    const second = Array.from({ length: 30 }, () => ({ text: "y", size: 17, weight: 400 }));
    expect(() =>
      validateRequests([{ measure: first }, { measure: second }], manifest, budget),
    ).toThrow(/64/);
  });

  test("exactly the budget's worth of measurements is accepted", () => {
    const measure = Array.from({ length: 64 }, () => ({ text: "x", size: 17, weight: 400 }));
    expect(validateRequests([{ measure }], manifest, budget)).toHaveLength(1);
  });

  test("a measure request does not consume the request budget", () => {
    const tight = { requests: 1, bytes: budget.bytes, measurements: 64 };
    const requests = validateRequests(
      [
        { url: "https://api.github.com/u", as: "json" },
        { measure: [{ text: "a", size: 10, weight: 400 }] },
        { measure: [{ text: "b", size: 10, weight: 400 }] },
      ],
      manifest,
      tight,
    );
    expect(requests).toHaveLength(3);
  });

  test("a request array that exceeds the request budget still refuses, measure entries aside", () => {
    const tight = { requests: 1, bytes: budget.bytes, measurements: 64 };
    const many = [
      { url: "https://api.github.com/u", as: "json" as const },
      { url: "https://api.github.com/v", as: "json" as const },
      { measure: [{ text: "a", size: 10, weight: 400 }] },
    ];
    expect(() => validateRequests(many, manifest, tight)).toThrow(/at most 1/);
  });

  test("a measure item alongside a network request answers at the same positions", async () => {
    const seen: unknown[] = [];
    const stub = async (input: unknown) => {
      seen.push(input);
      return { status: 200, body: "{}", json: {} };
    };
    const requests = validateRequests(
      [
        { measure: [{ text: "a", size: 10, weight: 400 }] },
        { url: "https://api.github.com/u", as: "json" },
      ],
      manifest,
      budget,
    );
    const { answers } = await performRequests(requests, manifest, {}, stub, budget);
    expect(answers[0]).toEqual({
      ok: true,
      measurements: [{ width: textWidth("a", 10, 400), ink: expect.any(Object) }],
    });
    expect(answers[1]).toEqual({ ok: true, status: 200, json: {} });
    expect(seen).toHaveLength(1);
  });

  test("a measure request is answered even once the byte budget is spent", async () => {
    const big = "x".repeat(2 * 1024 * 1024);
    const stub = async () => ({ status: 200, body: big, json: undefined });
    const requests = validateRequests(
      [
        { url: "https://api.github.com/u", as: "text" },
        { url: "https://api.github.com/v", as: "text" },
        { url: "https://api.github.com/w", as: "text" },
        { measure: [{ text: "still measured", size: 10, weight: 400 }] },
      ],
      manifest,
      budget,
    );
    const { answers } = await performRequests(requests, manifest, {}, stub, budget);
    expect(answers[2]).toEqual({ ok: false, error: expect.stringContaining("budget") });
    expect(answers[3]).toEqual({
      ok: true,
      measurements: [{ width: textWidth("still measured", 10, 400), ink: expect.any(Object) }],
    });
  });
});

describe("a manifest that names a header or param pins the credential there", () => {
  const pinned = parseManifest(
    {
      api: 1,
      id: "p",
      version: "1.0.0",
      label: "P",
      description: "d",
      author: "a",
      hosts: ["header.example", "query.example"],
      secrets: [
        {
          key: "h",
          label: "H",
          kind: "api_key",
          host: "header.example",
          send_as: "header",
          header: "X-Api-Key",
        },
        {
          key: "q",
          label: "Q",
          kind: "api_key",
          host: "query.example",
          send_as: "query",
          param: "apikey",
        },
      ],
      fields: [],
    },
    "p",
  );
  const secrets = { h: "SECRET_H", q: "SECRET_Q" };
  let seen: { url: string; headers?: Record<string, string> }[] = [];
  const stub = async (input: { url: string; headers?: Record<string, string> }) => {
    seen.push(input);
    return { status: 200, body: "", json: undefined };
  };

  test("the declared header substitutes", async () => {
    seen = [];
    await performRequests(
      validateRequests(
        [{ url: "https://header.example/x", as: "text", headers: { "X-Api-Key": "{{secret:h}}" } }],
        pinned,
        budget,
      ),
      pinned,
      secrets,
      stub,
      budget,
    );
    expect(seen[0]?.headers?.["X-Api-Key"]).toBe("SECRET_H");
  });

  test("the same header in a different case still substitutes", async () => {
    seen = [];
    await performRequests(
      validateRequests(
        [{ url: "https://header.example/x", as: "text", headers: { "x-api-KEY": "{{secret:h}}" } }],
        pinned,
        budget,
      ),
      pinned,
      secrets,
      stub,
      budget,
    );
    expect(seen[0]?.headers?.["x-api-KEY"]).toBe("SECRET_H");
  });

  test("a placeholder in a header the manifest did not name stays literal", async () => {
    seen = [];
    await performRequests(
      validateRequests(
        [
          {
            url: "https://header.example/x",
            as: "text",
            headers: { Authorization: "{{secret:h}}" },
          },
        ],
        pinned,
        budget,
      ),
      pinned,
      secrets,
      stub,
      budget,
    );
    expect(seen[0]?.headers?.Authorization).toBe("{{secret:h}}");
    expect(JSON.stringify(seen)).not.toContain("SECRET_H");
  });

  test("a placeholder in a query parameter the manifest did not name stays literal", async () => {
    seen = [];
    await performRequests(
      validateRequests(
        [{ url: "https://query.example/x?other={{secret:q}}&apikey={{secret:q}}", as: "text" }],
        pinned,
        budget,
      ),
      pinned,
      secrets,
      stub,
      budget,
    );
    const url = new URL(seen[0]?.url ?? "");
    expect(url.searchParams.get("apikey")).toBe("SECRET_Q");
    expect(url.searchParams.get("other")).toBe("{{secret:q}}");
  });
});

describe("performRequests reports the bytes it actually spent", () => {
  test("a text answer charges what crossed the wire, not what survived redaction", async () => {
    const stolen = "ghp_SECRETVALUE";
    const body = stolen.repeat(1000);
    const stub = async () => ({ status: 200, body, json: undefined });
    const { answers, bytesSpent } = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "text" }], manifest, budget),
      manifest,
      { token: stolen },
      stub,
      budget,
    );
    expect(answers[0]).toEqual({ ok: true, status: 200, text: "[redacted]".repeat(1000) });
    expect(bytesSpent).toBe(Buffer.byteLength(body, "utf-8"));
  });

  test("a bytes answer refused for echoing a credential still charges what it consumed", async () => {
    const stolen = "ghp_SECRETVALUE";
    const body = Buffer.concat([Buffer.alloc(64_000, 7), Buffer.from(stolen, "utf-8")]);
    const stub = async () => ({ status: 200, body: new Uint8Array(body), json: undefined });
    const { answers, bytesSpent } = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "bytes" }], manifest, budget),
      manifest,
      { token: stolen },
      stub,
      budget,
    );
    expect(answers[0]).toEqual({
      ok: false,
      status: 200,
      error: "the response echoed a stored credential and was refused",
    });
    expect(bytesSpent).toBe(body.length);
  });

  test("a response refused for exceeding the ingest cap charges what it read", async () => {
    const stub = async () => {
      throw new ResponseTooLargeError(INGEST_CAP_BYTES + 1);
    };
    const { answers, bytesSpent } = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "text" }], manifest, budget),
      manifest,
      {},
      stub,
      budget,
    );
    expect(answers[0]?.ok).toBe(false);
    expect(bytesSpent).toBe(INGEST_CAP_BYTES + 1);
  });

  test("a measure request spends nothing", async () => {
    const { bytesSpent } = await performRequests(
      validateRequests([{ measure: [{ text: "a", size: 10, weight: 400 }] }], manifest, budget),
      manifest,
      {},
      async () => ({ status: 200, body: "", json: undefined }),
      budget,
    );
    expect(bytesSpent).toBe(0);
  });
});
