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

// Names for script-local bindings the host injects around the plugin's own source
// (see `runInSandbox`). `const` at top-of-script scope never becomes an enumerable
// property of `globalThis` (verified: a plugin's own `Object.keys(globalThis)` still
// sees only its own declared function), and the prefix makes an accidental collision
// with a plugin's own identifier vanishingly unlikely -- and if one ever happens, it
// fails that one plugin loudly (a SandboxError), not silently.
const STRINGIFY_PROBE = "__deskmate_sandbox_json_stringify__";
const RESULT_PROBE = "__deskmate_sandbox_result__";
const TYPE_PROBE = "__deskmate_sandbox_result_type__";

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
 * `context.dump()` MUST NEVER be used to read a plugin's *return value* (a plugin's
 * thrown error is a different, much smaller value and is still read with `dump` in
 * `runInSandbox` below). `dump`'s native implementation (`QTS_Dump` in
 * quickjs-emscripten's `c/interface.c`) runs the engine's own `JSON.stringify`
 * internally and SWALLOWS any exception it raises, falling back to `ToString(value)`.
 * That one swallow-and-fall-back behaviour is responsible for three distinct ways a
 * plugin's return value could reach the host silently wrong, all reproduced against
 * the real runtime:
 *   1. A circular structure serializes to the literal string `"[object Object]"`
 *      instead of throwing.
 *   2. Calling `dump` as a *second*, separate read after an earlier probe read makes a
 *      stateful accessor (a getter whose return value depends on how many times it has
 *      already been called) see two different reads of the same graph -- acyclic on
 *      the first, circular on the second -- so a probe taken before `dump` cannot
 *      certify what `dump` itself will see.
 *   3. A legal, well within the memory cap, but large value can exceed some internal
 *      limit of that native call and come back as an empty string, with no exception
 *      and no connection to the runtime's own `memoryBytes`/deadline limits.
 * None of these raise a catchable host-side exception; `dump` just returns different,
 * wrong data. The fix is to never call it on untrusted data: serialize the return
 * value exactly once, inside the sandbox, with a `JSON.stringify` reference captured
 * before the plugin's own source ever runs (see `STRINGIFY_PROBE` in `runInSandbox`),
 * and read the resulting JSON text out with `getString` -- which does not go through
 * `QTS_Dump`'s swallow-and-fall-back path at all -- then `JSON.parse` it here. A real
 * exception during that one serialization (a cycle, a getter's side effect, an
 * out-of-memory) is a real thrown error reaching this function through the normal
 * `evaluated.error` path in `runInSandbox`, not something this function has to guard
 * against itself.
 */
function readSandboxedResult<T>(
  context: QuickJSContext,
  value: QuickJSHandle,
  fn: "plan" | "render",
): T {
  const type = context.typeof(value);
  if (type === "undefined") {
    // The plugin returned undefined (explicitly or implicitly); JSON.stringify(undefined)
    // is itself undefined, which is what the sandboxed probe evaluates to in this case.
    return null as T;
  }
  if (type !== "string") {
    // Not reachable given the script `runInSandbox` builds -- every other type is
    // either refused inside the script (function/symbol/bigint, by name) or
    // serialized to a JSON string by STRINGIFY_PROBE. Guarded anyway: if this file's
    // script template ever changes, this must fail loudly, not silently.
    throw new SandboxError(
      `this plugin's ${fn}() returned a value that could not be represented as data (unexpected type ${type})`,
    );
  }
  const json = context.getString(value);
  try {
    return JSON.parse(json) as T;
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
    // the plugin's own source has run, so nothing the plugin does -- reassigning
    // `JSON.stringify`, defining a `toJSON`, anything -- can swap out what it calls.
    // It performs the ONE AND ONLY read of the plugin's return value: a function,
    // symbol or bigint is refused by name via `typeof` first (a pure type-tag check,
    // not a graph read, so it can't be fooled by a stateful accessor the way reading
    // the value twice could be); everything else is serialized exactly once by
    // STRINGIFY_PROBE, whose resulting JSON string is the only thing read back
    // host-side (via `getString` + `JSON.parse` in `readSandboxedResult`). See that
    // function's comment for why `context.dump()` must never be used here instead.
    const script = `const ${STRINGIFY_PROBE} = JSON.stringify;
${source.replace(EXPORT_STRIP, "")}
if (typeof ${fn} !== "function") { throw new Error("this plugin exports no ${fn}()"); }
const ${RESULT_PROBE} = ${fn}(${JSON.stringify(input)});
const ${TYPE_PROBE} = typeof ${RESULT_PROBE};
if (${TYPE_PROBE} === "function" || ${TYPE_PROBE} === "symbol" || ${TYPE_PROBE} === "bigint") {
  throw new Error("this plugin's ${fn}() returned a " + ${TYPE_PROBE} + ", which cannot be represented as data");
}
${STRINGIFY_PROBE}(${RESULT_PROBE});`;
    const evaluated = context.evalCode(script);
    if (evaluated.error) {
      const dumped = context.dump(evaluated.error);
      evaluated.error.dispose();
      const { message, configuration } = normalizeThrown(dumped);
      throw new SandboxError(message, configuration);
    }
    try {
      return readSandboxedResult<T>(context, evaluated.value, fn);
    } finally {
      evaluated.value.dispose();
    }
  } finally {
    context.dispose();
    runtime.dispose();
  }
}
