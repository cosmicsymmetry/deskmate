import { beforeAll, describe, expect, test } from "bun:test";
import { SandboxError, runInSandbox, warmSandbox } from "../../src/plugins/sandbox";

const plugin = `export function render(context) { return { seen: context.settings.user }; }`;

beforeAll(async () => {
  await warmSandbox();
});

describe("the sandbox", () => {
  test("runs a plugin function and returns its value", () => {
    expect(
      runInSandbox<{ seen: string }>(plugin, "render", { settings: { user: "octocat" } }),
    ).toEqual({
      seen: "octocat",
    });
  });

  test("stops an infinite loop at the deadline", () => {
    const start = Date.now();
    expect(() =>
      runInSandbox(`export function render(){ while(true){} }`, "render", {}, { deadlineMs: 300 }),
    ).toThrow(SandboxError);
    expect(Date.now() - start).toBeLessThan(3_000);
  });

  test("refuses an allocation above the memory cap", () => {
    // One bounded allocation isolates the memory guard from the interrupt deadline.
    // An endless allocation loop raced Bun's own 5s test timeout on Linux and
    // accepted a deadline error as evidence of a working memory cap.
    const source = `export function render(){ return new Uint8Array(16 * 1024 * 1024).byteLength; }`;
    expect(() => runInSandbox(source, "render", {}, { memoryBytes: 8 * 1024 * 1024 })).toThrow(
      /out of memory/i,
    );
    expect(runInSandbox<number>(source, "render", {}, { memoryBytes: 32 * 1024 * 1024 })).toBe(
      16 * 1024 * 1024,
    );
  });

  test("has no way to reach files or the network", () => {
    expect(() =>
      runInSandbox(
        `export function render(){ return require("fs").readFileSync("/etc/passwd","utf8"); }`,
        "render",
        {},
      ),
    ).toThrow(/require/);
    expect(() =>
      runInSandbox(
        `export function render(){ return fetch("https://example.com"); }`,
        "render",
        {},
      ),
    ).toThrow(/fetch/);
  });

  test("sees only its own function among the globals it can enumerate", () => {
    const globals = runInSandbox<string[]>(
      `export function render(){ return Object.keys(globalThis); }`,
      "render",
      {},
    );
    expect(globals).toEqual(["render"]);
  });

  test("reports a plugin's own error with its message", () => {
    expect(() =>
      runInSandbox(`export function render(){ throw new Error("boom"); }`, "render", {}),
    ).toThrow(/boom/);
  });

  test("refuses a source over the size cap without running it", () => {
    expect(() =>
      runInSandbox(`export function render(){ return 1; }//${"x".repeat(1_048_576)}`, "render", {}),
    ).toThrow(/too large/);
  });

  test("refuses a function the plugin does not export", () => {
    expect(() => runInSandbox(`export function render(){ return 1; }`, "plan", {})).toThrow(/plan/);
  });

  test("refuses a function/symbol/bigint return by name, instead of a raw parse error escaping", () => {
    expect(() =>
      runInSandbox(`export function render(){ return function(){}; }`, "render", {}),
    ).toThrow(/returned a function/);
    expect(() =>
      runInSandbox(`export function render(){ return Symbol("x"); }`, "render", {}),
    ).toThrow(/returned a symbol/);
    expect(() => runInSandbox(`export function render(){ return 10n; }`, "render", {})).toThrow(
      /returned a bigint/,
    );
  });

  test("reports a plugin's own throw null as a SandboxError, not a raw TypeError", () => {
    expect(() => runInSandbox(`export function render(){ throw null; }`, "render", {})).toThrow(
      SandboxError,
    );
  });

  test("reports a plugin's own throw undefined as a SandboxError, not a raw TypeError", () => {
    expect(() =>
      runInSandbox(`export function render(){ throw undefined; }`, "render", {}),
    ).toThrow(SandboxError);
  });

  test("does not corrupt a plugin's own string content when stripping its export", () => {
    const source = `export function render(){ return { html: \`<div>export const foo = 1;</div>\` }; }`;
    expect(runInSandbox<{ html: string }>(source, "render", {})).toEqual({
      html: "<div>export const foo = 1;</div>",
    });
  });

  test("counts the source cap in bytes, not UTF-16 code units", () => {
    const multiByte = "€".repeat(400_000); // 3 bytes each in UTF-8, 1 UTF-16 unit each
    expect(() =>
      runInSandbox(`export function render(){ return "${multiByte}"; }`, "render", {}),
    ).toThrow(/too large/);
  });

  test("refuses a top-level circular return, naming the circular reference", () => {
    expect(() =>
      runInSandbox(`export function render(){ const o={}; o.self=o; return o; }`, "render", {}),
    ).toThrow(/circular/);
  });

  test("refuses a circular return nested inside a wrapper object, naming the circular reference", () => {
    expect(() =>
      runInSandbox(
        `export function render(){
          const inner={};
          inner.self=inner;
          return { wrapper: { nested: inner } };
        }`,
        "render",
        {},
      ),
    ).toThrow(/circular/);
  });

  test("observes a plugin's own getter with a side effect exactly once", () => {
    // The getter increments a counter and returns the counter's OWN new value. If the
    // sandbox read this object's graph more than once (the round-2 defect: a probe
    // read, then a separate context.dump() read), the second read would observe a
    // different value (2, not 1) from the same getter -- so the returned JSON is
    // itself the proof of how many times the getter fired, not just an assertion
    // bolted on afterward.
    const source = `export function render(){
      globalThis.__reads__ = 0;
      const holder = {};
      Object.defineProperty(holder, "value", {
        enumerable: true,
        get(){ globalThis.__reads__++; return globalThis.__reads__; },
      });
      return holder;
    }`;
    expect(runInSandbox<{ value: number }>(source, "render", {})).toEqual({ value: 1 });
  });

  // The exact reproduction from the round-2 re-review: a getter that returns "safe" on
  // its first call and the circular object itself (`o`) on every call after that. Under
  // the two-read design (round 2: a probe, then a separate context.dump() read) the
  // probe observed "safe" and passed, then dump()'s own internal read observed the
  // cycle and silently degraded to the string "[object Object]" -- corruption with no
  // thrown error. Under the one-read design implemented here, the object's graph is
  // read exactly once, by the sandboxed JSON.stringify itself, which calls this getter
  // exactly once too -- and that one call is the FIRST call, so it genuinely,
  // correctly observes "safe": there is no second call within a single serialization
  // pass for a plain (non-recursive) property access to produce a cycle from. Forcing
  // this specific object to be reported as circular would require reading its graph a
  // second time, which is exactly the defect being removed and is incompatible with
  // "a getter runs exactly once". The correct, non-corrupted behavior for a single
  // honest read is the plugin's own single, true answer -- proven here, not merely
  // "does not throw".
  test("reads a stateful getter exactly once instead of corrupting the result on a second, hidden read", () => {
    const topLevel = `export function render(){
      let read = false; const o = {};
      Object.defineProperty(o, "self", { enumerable: true,
        get(){ if (!read) { read = true; return "safe"; } return o; } });
      return o;
    }`;
    expect(runInSandbox<{ self: string }>(topLevel, "render", {})).toEqual({ self: "safe" });

    const nested = `export function render(){
      let read = false; const inner = {};
      Object.defineProperty(inner, "self", { enumerable: true,
        get(){ if (!read) { read = true; return "safe"; } return inner; } });
      return { wrapper: { nested: inner } };
    }`;
    expect(runInSandbox<{ wrapper: { nested: { self: string } } }>(nested, "render", {})).toEqual({
      wrapper: { nested: { self: "safe" } },
    });
  });

  test("returns a large array intact or throws, but never as an empty or truncated string", () => {
    const source = `export function render(){
      const a = [];
      for (let i = 0; i < 200000; i++) a.push({ i, name: "item-" + i, active: true });
      return a;
    }`;
    try {
      const result = runInSandbox<Array<{ i: number; name: string; active: boolean }>>(
        source,
        "render",
        {},
        { deadlineMs: 10_000 },
      );
      expect(Array.isArray(result)).toBe(true);
      expect(result).toHaveLength(200_000);
      expect(result[0]).toEqual({ i: 0, name: "item-0", active: true });
      expect(result[199_999]).toEqual({ i: 199_999, name: "item-199999", active: true });
    } catch (error) {
      expect(error).toBeInstanceOf(SandboxError);
    }
  });

  test("returns ordinary primitive and structural values intact", () => {
    expect(runInSandbox<string>(`export function render(){ return "hello"; }`, "render", {})).toBe(
      "hello",
    );
    expect(runInSandbox<number>(`export function render(){ return 42; }`, "render", {})).toBe(42);
    expect(runInSandbox<null>(`export function render(){ return null; }`, "render", {})).toBeNull();
    expect(runInSandbox<null>(`export function render(){ }`, "render", {})).toBeNull();
    expect(
      runInSandbox<{ a: { b: number[] } }>(
        `export function render(){ return { a: { b: [1,2,3] } }; }`,
        "render",
        {},
      ),
    ).toEqual({ a: { b: [1, 2, 3] } });
  });

  test("carries a plugin's own configuration flag across the boundary", () => {
    const source = `export function render(){ const e = new Error("no city called Xyz"); e.configuration = true; throw e; }`;
    const failure = (() => {
      try {
        runInSandbox(source, "render", {});
      } catch (error) {
        return error as SandboxError;
      }
      throw new Error("expected runInSandbox to throw");
    })();
    expect(failure.message).toContain("no city called Xyz");
    expect(failure.configuration).toBe(true);
  });
});
