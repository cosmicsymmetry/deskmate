import { beforeAll, describe, expect, test } from "bun:test";
import { ConfigurationError, TransientError } from "../../src/face";
import { parseManifest } from "../../src/plugins/manifest";
import { runPlugin } from "../../src/plugins/run";
import { warmSandbox } from "../../src/plugins/sandbox";
import { ONE_BOX_CARD, minimalManifest } from "./test_support";

beforeAll(async () => {
  await warmSandbox();
});

const manifest = parseManifest(
  minimalManifest("p", { label: "P", hosts: ["api.example.com"] }),
  "p",
);

const base = {
  manifest,
  settings: {},
  now: new Date("2026-09-23T10:00:00Z"),
  timezone: "Asia/Dubai",
  secrets: {},
};
const card = ONE_BOX_CARD;

describe("runPlugin", () => {
  test("plans, fetches, renders", async () => {
    const source = `
      export function plan(){ return [{ url: "https://api.example.com/a", as: "json" }]; }
      export function render(c){ return ${card.replace('"ok"', "String(c.answers[0].json.n)")}; }`;
    const request = async () => ({ status: 200, body: '{"n":7}', json: { n: 7 } });
    const result = await runPlugin({ ...base, source, request });
    expect(result.svg).toContain("7");
  });

  test("runs plan again with the answers, up to three rounds", async () => {
    const source = `
      export function plan(c){
        const round = (c.answers || []).length;
        return round < 3 ? [{ url: "https://api.example.com/" + round, as: "text" }] : [];
      }
      export function render(c){ return ${card.replace('"ok"', "String(c.answers.length)")}; }`;
    const seen: string[] = [];
    const request = async (input: { url: string }) => {
      seen.push(input.url);
      return { status: 200, body: "x" };
    };
    await runPlugin({ ...base, source, request });
    expect(seen).toEqual([
      "https://api.example.com/0",
      "https://api.example.com/1",
      "https://api.example.com/2",
    ]);
  });

  test("an undeclared host is a configuration error naming the host", async () => {
    const source = `export function plan(){ return [{ url: "https://evil.example/", as: "json" }]; }
      export function render(){ return ${card}; }`;
    await expect(runPlugin({ ...base, source })).rejects.toThrow(ConfigurationError);
  });

  test("a plugin that exceeds its deadline is transient", async () => {
    // One export per line, deliberately: with both on one line the second `export`
    // survives EXPORT_STRIP and the plugin never parses, so this test passed for
    // years without the deadline ever firing.
    const source = `export function plan(){ return []; }
      export function render(){ while(true){} }`;
    await expect(runPlugin({ ...base, source })).rejects.toThrow(TransientError);
  });

  test("a plugin that throws ConfigurationError-shaped text reaches the owner", async () => {
    const source = `export function plan(){ return []; }
      export function render(){ const e = new Error("no city called Xyz"); e.configuration = true; throw e; }`;
    await expect(runPlugin({ ...base, source })).rejects.toThrow(/no city called Xyz/);
  });

  test("state round-trips, and state over 16 KB is dropped with a log line", async () => {
    const source = `export function plan(){ return []; }
      export function render(c){ return { ...${card}, state: { seen: (c.state && c.state.seen || 0) + 1 } }; }`;
    const first = await runPlugin({ ...base, source });
    expect(first.state).toEqual({ seen: 1 });
    const second = await runPlugin({ ...base, source, state: first.state });
    expect(second.state).toEqual({ seen: 2 });

    const fat = `export function plan(){ return []; }
      export function render(){ return { ...${card}, state: { blob: "x".repeat(20000) } }; }`;
    const result = await runPlugin({ ...base, source: fat });
    expect(result.state).toBeUndefined();
    expect(result.log.join(" ")).toContain("16 KB");
  });

  test("a log line is sanitized, not merely capped, before it reaches our stderr", async () => {
    const source = `export function plan(){ return []; }
      export function render(){ return { ...${card}, log: ["\\u001b[31mred\\u001b[0m\\nsecond line\\u0007", "plain"] }; }`;
    const result = await runPlugin({ ...base, source });
    expect(result.log).toHaveLength(2);
    const first = result.log[0] ?? "";
    expect(first).not.toContain("\u001b");
    expect(first).not.toContain("\n");
    expect(first).not.toContain("\u0007");
    expect(first).toContain("red");
    expect(first).toContain("second line");
  });

  test.each([
    {
      name: "10 strings with non-string entries",
      raw: [null, 7, ...Array(10).fill("ok"), {}],
      lines: Array(10).fill("ok"),
      notices: [],
    },
    {
      name: "11 strings",
      raw: Array(11).fill("ok"),
      lines: Array(10).fill("ok"),
      notices: ["Plugin log was limited to 10 lines."],
    },
    {
      name: "200 sanitized characters",
      raw: [` \u001b[31m${"x".repeat(200)}\u001b[0m `],
      lines: ["x".repeat(200)],
      notices: [],
    },
    {
      name: "201 sanitized characters",
      raw: [` \u001b[31m${"x".repeat(201)}\u001b[0m `],
      lines: ["x".repeat(200)],
      notices: ["Plugin log lines were shortened to 200 characters."],
    },
    {
      name: "long eleventh string",
      raw: [...Array(10).fill("ok"), null, "x".repeat(201)],
      lines: Array(10).fill("ok"),
      notices: [
        "Plugin log was limited to 10 lines.",
        "Plugin log lines were shortened to 200 characters.",
      ],
    },
    { name: "non-array log", raw: "ignored", lines: [], notices: [] },
  ])("log cap notices: $name", async ({ raw, lines, notices }) => {
    const seen: string[] = [];
    const source = `export function plan(){ return []; }
      export function render(){ return { ...${card}, log: ${JSON.stringify(raw)} }; }`;
    const result = await runPlugin({ ...base, source, onNotice: (line) => seen.push(line) });
    expect(result.log).toEqual([...lines, ...notices]);
    expect(seen).toEqual([...notices]);
  });

  test("planning notices precede both log notices in the result and callback", async () => {
    const seen: string[] = [];
    const source = `export function plan(){ return [{url: "https://api.example.com/", as: "text"}]; }
      export function render(){ return { ...${card}, log: Array(11).fill("x".repeat(201)) }; }`;
    const result = await runPlugin({
      ...base,
      source,
      request: async () => ({ status: 200, body: "ok" }),
      onNotice: (line) => seen.push(line),
    });
    const notices = [
      "Planning stopped at the 3-round limit; return [] as soon as all answers are available.",
      "Plugin log was limited to 10 lines.",
      "Plugin log lines were shortened to 200 characters.",
    ];
    expect(seen).toEqual([...notices]);
    expect(result.log).toEqual([...Array(10).fill("x".repeat(200)), ...notices]);
  });

  test("a tap reaches the plugin as an event", async () => {
    const source = `export function plan(){ return []; }
      export function render(c){ return ${card.replace('"ok"', "String(c.event ? c.event.taps : 0)")}; }`;
    const result = await runPlugin({ ...base, source, event: { taps: 3, point: null } });
    expect(result.svg).toContain("3");
  });

  test("a plan that returns something other than an array is a configuration error", async () => {
    const source = `export function plan(){ return 42; }
      export function render(){ return ${card}; }`;
    await expect(runPlugin({ ...base, source })).rejects.toThrow(ConfigurationError);
  });

  test("rounds stop at 3 even when the plugin always asks for more", async () => {
    const source = `
      export function plan(){ return [{ url: "https://api.example.com/x", as: "text" }]; }
      export function render(c){ return ${card.replace('"ok"', "String(c.answers.length)")}; }`;
    let calls = 0;
    const request = async () => {
      calls += 1;
      return { status: 200, body: "x" };
    };
    const result = await runPlugin({ ...base, source, request });
    expect(calls).toBe(3);
    expect(result.log.join(" ")).toContain("3-round limit");
    expect(result.svg).toContain("3");
  });

  test("a plugin declaring no requests never touches the network", async () => {
    const source = `export function plan(){ return []; }
      export function render(){ return ${card}; }`;
    let called = false;
    const request = async () => {
      called = true;
      return { status: 200, body: "" };
    };
    await runPlugin({ ...base, source, request });
    expect(called).toBe(false);
  });

  test("a render that throws after successful fetches is transient, not configuration", async () => {
    const source = `
      export function plan(){ return [{ url: "https://api.example.com/a", as: "json" }]; }
      export function render(){ throw new Error("boom after fetch"); }`;
    const request = async () => ({ status: 200, body: '{"n":1}', json: { n: 1 } });
    await expect(runPlugin({ ...base, source, request })).rejects.toThrow(TransientError);
  });

  test("state exactly at the 16 KB cap round-trips unchanged", async () => {
    // {"blob":"<N x's>"} is 11 + N bytes; 16384 - 11 = 16373.
    const n = 16384 - 11;
    const source = `export function plan(){ return []; }
      export function render(){ return { ...${card}, state: { blob: "x".repeat(${n}) } }; }`;
    const result = await runPlugin({ ...base, source });
    expect(result.state).toBeDefined();
    expect(Buffer.byteLength(JSON.stringify(result.state), "utf8")).toBe(16384);
  });

  test("state one byte over the 16 KB cap is dropped", async () => {
    const n = 16384 - 11 + 1;
    const source = `export function plan(){ return []; }
      export function render(){ return { ...${card}, state: { blob: "x".repeat(${n}) } }; }`;
    const result = await runPlugin({ ...base, source });
    expect(result.state).toBeUndefined();
    expect(result.log.join(" ")).toContain("16 KB");
  });

  test("a plugin missing render() is a configuration error naming the missing function", async () => {
    const source = `export function plan(){ return []; }`;
    await expect(runPlugin({ ...base, source })).rejects.toThrow(ConfigurationError);
    await expect(runPlugin({ ...base, source })).rejects.toThrow(/exports no render\(\)/);
  });

  test("a plugin missing plan() is a configuration error naming the missing function", async () => {
    const source = `export function render(){ return ${card}; }`;
    await expect(runPlugin({ ...base, source })).rejects.toThrow(ConfigurationError);
    await expect(runPlugin({ ...base, source })).rejects.toThrow(/exports no plan\(\)/);
  });

  test("the request budget spans rounds: 5 then 5 more is refused", async () => {
    const five = JSON.stringify(
      Array.from({ length: 5 }, () => ({ url: "https://api.example.com/r", as: "text" })),
    );
    const source = `export function plan(){ return ${five}; }
      export function render(){ return ${card}; }`;
    const request = async () => ({ status: 200, body: "x" });
    await expect(runPlugin({ ...base, source, request })).rejects.toThrow(
      /at most 3 requests per refresh, not 5/,
    );
  });

  test("the request budget spans rounds: 4 then 4 succeeds, spending the render's full 8", async () => {
    const source = `
      export function plan(c){
        const n = (c.answers || []).length;
        const batch = Array.from({ length: 4 }, () => ({ url: "https://api.example.com/r", as: "text" }));
        return n < 8 ? batch : [];
      }
      export function render(c){ return ${card.replace('"ok"', "String(c.answers.length)")}; }`;
    let calls = 0;
    const request = async () => {
      calls += 1;
      return { status: 200, body: "x" };
    };
    const result = await runPlugin({ ...base, source, request });
    expect(calls).toBe(8);
    expect(result.svg).toContain("8");
  });

  // A host-detected refusal that can never succeed on a retry must reach the owner as a
  // ConfigurationError: the panel has no way to say "this plugin does not parse", so a
  // TransientError here means a 60-second retry loop forever and nothing in the window.
  describe("a deterministic host-side refusal is a configuration error", () => {
    test("a syntax error in the plugin source", async () => {
      const source = `export function plan(){ return []; }
        export function render(){ const x = ; }`;
      await expect(runPlugin({ ...base, source })).rejects.toThrow(ConfigurationError);
    });

    test("two exports on one line, which the export strip cannot reach", async () => {
      // EXPORT_STRIP is anchored to the start of a line, so a minifier's single-line
      // output leaves the second `export` in place and QuickJS refuses the keyword.
      const source = `export function plan(){ return []; } export function render(){ return ${card}; }`;
      await expect(runPlugin({ ...base, source })).rejects.toThrow(ConfigurationError);
    });

    test("a source over the 1 MB cap", async () => {
      const source = `export function plan(){ return []; }
        export function render(){ return ${card}; }
        // ${"x".repeat(1024 * 1024)}`;
      await expect(runPlugin({ ...base, source })).rejects.toThrow(ConfigurationError);
      await expect(runPlugin({ ...base, source })).rejects.toThrow(/too large/);
    });

    test("a card over one of our own caps", async () => {
      const source = `export function plan(){ return []; }
        export function render(){ return { svg: '<svg>' + ' '.repeat(512 * 1024) + '</svg>' }; }`;
      await expect(runPlugin({ ...base, source })).rejects.toThrow(ConfigurationError);
      await expect(runPlugin({ ...base, source })).rejects.toThrow(/512 KB/);
    });
  });

  describe("an engine limit and our own wrapped exception stay transient", () => {
    test("the memory cap stops the plugin", async () => {
      const source = `export function plan(){ return []; }
        export function render(){ const a = []; for(;;){ a.push("x".repeat(65536)); } }`;
      await expect(runPlugin({ ...base, source })).rejects.toThrow(TransientError);
    });

    test("a foreign satori failure is ours, not a setting the owner can change", async () => {
      // An undecodable image is a raw error out of satori: we wrapped somebody else's
      // exception rather than firing one of our own explicit checks.
      const source = `export function plan(){ return []; }
        export function render(){ return { layout: { type: "div", style: { display: "flex", width: 448, height: 368 }, children: { type: "img", style: { width: 10, height: 10 }, src: "data:image/png;base64,QUJDRA==" } } }; }`;
      await expect(runPlugin({ ...base, source })).rejects.toThrow(TransientError);
    });
  });

  test("the byte budget spans rounds and charges the wire, not the answer", async () => {
    // Both answers are REFUSED for echoing a stored credential, so neither carries a
    // payload -- and re-deriving the spend from the finished answers charged them 0,
    // which bought a plugin an unbounded number of megabyte responses out of a 4 MB
    // render budget. What crossed the wire was 6 MB, so round two must find the
    // budget spent and never reach the network.
    const credential = "ghp_SECRETVALUE";
    const withSecret = parseManifest(
      minimalManifest("p", {
        label: "P",
        hosts: ["api.example.com"],
        secrets: [
          {
            key: "token",
            label: "T",
            kind: "api_key",
            host: "api.example.com",
            send_as: "bearer",
          },
        ],
      }),
      "p",
    );
    const body = Buffer.concat([
      Buffer.alloc(3 * 1024 * 1024, 7),
      Buffer.from(credential, "utf-8"),
    ]);
    const source = `
      export function plan(c){
        const n = (c.answers || []).length;
        if (n === 0) return [
          { url: "https://api.example.com/a", as: "bytes" },
          { url: "https://api.example.com/b", as: "bytes" },
        ];
        if (n === 2) return [{ url: "https://api.example.com/c", as: "bytes" }];
        return [];
      }
      export function render(c){ return ${card.replace('"ok"', "String(c.answers.length)")}; }`;
    let calls = 0;
    const request = async () => {
      calls += 1;
      return { status: 200, body: new Uint8Array(body) };
    };
    const result = await runPlugin({
      ...base,
      manifest: withSecret,
      secrets: { token: credential },
      source,
      request,
    });
    expect(calls).toBe(2);
    expect(result.svg).toContain("3");
  });

  test("the measurement budget spans rounds: 40 then 40 more is refused", async () => {
    const forty = JSON.stringify(
      Array.from({ length: 40 }, () => ({ text: "a", size: 10, weight: 400 })),
    );
    const source = `export function plan(){ return [{ measure: ${forty} }]; }
      export function render(){ return ${card}; }`;
    await expect(runPlugin({ ...base, source })).rejects.toThrow(
      /at most 24 measurements per refresh, not 40/,
    );
  });
});
