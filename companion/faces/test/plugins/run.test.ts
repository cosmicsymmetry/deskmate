import { beforeAll, describe, expect, test } from "bun:test";
import { ConfigurationError, TransientError } from "../../src/face";
import { parseManifest } from "../../src/plugins/manifest";
import { runPlugin } from "../../src/plugins/run";
import { warmSandbox } from "../../src/plugins/sandbox";

beforeAll(async () => {
  await warmSandbox();
});

const manifest = parseManifest(
  {
    api: 1,
    id: "p",
    version: "1.0.0",
    label: "P",
    description: "d",
    author: "a",
    hosts: ["api.example.com"],
    secrets: [],
    fields: [],
  },
  "p",
);

const base = {
  manifest,
  settings: {},
  now: new Date("2026-09-23T10:00:00Z"),
  timezone: "Asia/Dubai",
  secrets: {},
};
const card = `{ layout: { type: "div", style: { display: "flex", width: 448, height: 368, background: "#000" }, children: "ok" } }`;

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
    const source = `export function plan(){ return []; } export function render(){ while(true){} }`;
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
