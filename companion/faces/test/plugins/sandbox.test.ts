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

  test("stops an allocation loop at the memory cap", () => {
    expect(() =>
      runInSandbox(
        `export function render(){ const a=[]; for(;;) a.push(new Array(100000).fill("x")); }`,
        "render",
        {},
        { memoryBytes: 8 * 1024 * 1024, deadlineMs: 5_000 },
      ),
    ).toThrow(SandboxError);
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
