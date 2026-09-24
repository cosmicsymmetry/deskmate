// A plugin runs here and nowhere else: QuickJS compiled to WebAssembly, with no
// imports at all. It cannot open a file, reach the network or see our environment,
// because none of those functions exist inside it -- not because they are guarded.
import {
  type QuickJSContext,
  type QuickJSHandle,
  type QuickJSWASMModule,
  getQuickJS,
} from "quickjs-emscripten";

export class SandboxError extends Error {
  override name = "SandboxError";
  /**
   * True when the plugin's own thrown error carried a truthy `configuration`
   * property -- the owner must change a setting, and the caller must never
   * retry this the way it retries an ordinary failure.
   */
  configuration: boolean;

  constructor(message: string, configuration = false) {
    super(message);
    this.configuration = configuration;
  }
}

export interface SandboxLimits {
  memoryBytes: number;
  deadlineMs: number;
  sourceBytes: number;
}

export const SANDBOX_LIMITS: SandboxLimits = {
  memoryBytes: 16 * 1024 * 1024,
  deadlineMs: 2_000,
  sourceBytes: 1024 * 1024,
};

let quickjs: QuickJSWASMModule | undefined;

/** Loaded once and reused; each render still gets a fresh runtime and context. */
export async function warmSandbox(): Promise<void> {
  quickjs ??= await getQuickJS();
}

// `export function f`/`export const x`/`export let x` becomes a plain declaration:
// QuickJS evaluates a script, not a module, and the plugin's own source is never
// trusted to import anything. Anchored to the start of a line so it cannot fire
// inside a plugin's own string or template-literal content (e.g. a `<div>` containing
// the text "export const"). This is a narrowing, not the cure -- the real cure is that
// a published plugin is bundled to a format with no `export` at all (Task 9's concern).
const EXPORT_STRIP = /^[ \t]*export[ \t]+(?=function|const|let)/gm;

// Names for two script-local bindings the host injects around the plugin's own
// source (see `runInSandbox`). `const` at top-of-script scope never becomes an
// enumerable property of `globalThis` (verified: a plugin's own
// `Object.keys(globalThis)` still sees only its own declared function), and the
// prefix makes an accidental collision with a plugin's own identifier vanishingly
// unlikely -- and if one ever happens, it fails that one plugin loudly (a
// SandboxError), not silently.
const STRINGIFY_PROBE = "__deskmate_sandbox_json_stringify__";
const RESULT_PROBE = "__deskmate_sandbox_result__";

/** Normalizes whatever a plugin's own `throw` produced into a message and a flag. */
function normalizeThrown(dumped: unknown): { message: string; configuration: boolean } {
  if (typeof dumped === "string") {
    return { message: dumped, configuration: false };
  }
  if (dumped !== null && typeof dumped === "object") {
    const record = dumped as { message?: unknown; configuration?: unknown };
    const message = typeof record.message === "string" ? record.message : JSON.stringify(dumped);
    return { message, configuration: Boolean(record.configuration) };
  }
  // `throw null`, `throw undefined`, `throw 42`, `throw true`, ... -- anything that
  // is neither a string nor a plain object.
  return { message: String(dumped), configuration: false };
}

/**
 * Converts the sandboxed return value to host data via `context.dump`, never via the
 * sandbox's own (plugin-controllable) `JSON.stringify`. A function, symbol or bigint
 * cannot be represented as plugin output at all, so those are refused explicitly
 * rather than silently degraded to a stringified function body or similar -- `dump`'s
 * own best-effort fallback for those types is exactly that silent degradation.
 * A circular return is caught earlier, inside the script itself (see `runInSandbox`),
 * because `dump` degrades a cycle the same way: silently, to `"[object Object]"`,
 * rather than throwing.
 */
function normalizeReturn<T>(
  context: QuickJSContext,
  value: QuickJSHandle,
  fn: "plan" | "render",
): T {
  const type = context.typeof(value);
  if (type === "undefined") {
    return null as T;
  }
  if (type === "function" || type === "symbol" || type === "bigint") {
    throw new SandboxError(
      `this plugin's ${fn}() returned a ${type}, which cannot be represented as data`,
    );
  }
  try {
    return context.dump(value) as T;
  } catch (error) {
    throw new SandboxError(
      `this plugin's ${fn}() returned a value that could not be represented as data: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

export function runInSandbox<T>(
  source: string,
  fn: "plan" | "render",
  input: unknown,
  limits: Partial<SandboxLimits> = {},
): T {
  const { memoryBytes, deadlineMs, sourceBytes } = { ...SANDBOX_LIMITS, ...limits };
  const sourceLength = Buffer.byteLength(source, "utf8");
  if (sourceLength > sourceBytes) {
    throw new SandboxError(
      `this plugin's source is too large: ${sourceLength} bytes, the limit is ${sourceBytes}`,
    );
  }
  if (quickjs === undefined) {
    throw new SandboxError("the sandbox was not warmed; call warmSandbox() first");
  }
  const runtime = quickjs.newRuntime();
  runtime.setMemoryLimit(memoryBytes);
  runtime.setMaxStackSize(1024 * 1024);
  const deadline = Date.now() + deadlineMs;
  runtime.setInterruptHandler(() => Date.now() > deadline);
  const context = runtime.newContext();
  try {
    // The captured JSON.stringify (STRINGIFY_PROBE) is bound before a single line of
    // the plugin's own source has run, so nothing the plugin does --
    // reassigning `JSON.stringify`, defining a `toJSON`, anything -- can swap out
    // what this probe calls. It exists purely to make QuickJS's own native cycle
    // detector throw on a circular return (message: "circular reference", proven
    // both top-level and nested inside a wrapper); its result is discarded. The
    // actual return value is still read host-side via `context.dump` in
    // `normalizeReturn`, never through this or any other sandboxed `JSON.stringify`.
    const script = `const ${STRINGIFY_PROBE} = JSON.stringify;
${source.replace(EXPORT_STRIP, "")}
if (typeof ${fn} !== "function") { throw new Error("this plugin exports no ${fn}()"); }
const ${RESULT_PROBE} = ${fn}(${JSON.stringify(input)});
${STRINGIFY_PROBE}(${RESULT_PROBE});
${RESULT_PROBE};`;
    const evaluated = context.evalCode(script);
    if (evaluated.error) {
      const dumped = context.dump(evaluated.error);
      evaluated.error.dispose();
      const { message, configuration } = normalizeThrown(dumped);
      throw new SandboxError(message, configuration);
    }
    try {
      return normalizeReturn<T>(context, evaluated.value, fn);
    } finally {
      evaluated.value.dispose();
    }
  } finally {
    context.dispose();
    runtime.dispose();
  }
}
