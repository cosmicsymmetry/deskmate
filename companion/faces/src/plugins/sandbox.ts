// A plugin runs here and nowhere else: QuickJS compiled to WebAssembly, with no
// imports at all. It cannot open a file, reach the network or see our environment,
// because none of those functions exist inside it -- not because they are guarded.
import { type QuickJSWASMModule, getQuickJS } from "quickjs-emscripten";

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

export function runInSandbox<T>(
  source: string,
  fn: "plan" | "render",
  input: unknown,
  limits: Partial<SandboxLimits> = {},
): T {
  const { memoryBytes, deadlineMs, sourceBytes } = { ...SANDBOX_LIMITS, ...limits };
  if (source.length > sourceBytes) {
    throw new SandboxError(
      `this plugin's source is too large: ${source.length} bytes, the limit is ${sourceBytes}`,
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
    // `export function f` becomes a plain declaration: QuickJS evaluates a script,
    // not a module, and the plugin's own source is never trusted to import anything.
    const script = `${source.replace(/export\s+(?=function|const|let)/g, "")}
if (typeof ${fn} !== "function") { throw new Error("this plugin exports no ${fn}()"); }
JSON.stringify(${fn}(${JSON.stringify(input)}) ?? null)`;
    const evaluated = context.evalCode(script);
    if (evaluated.error) {
      const dumped = context.dump(evaluated.error) as
        | { message?: string; configuration?: unknown }
        | string;
      evaluated.error.dispose();
      const message =
        typeof dumped === "string" ? dumped : (dumped.message ?? JSON.stringify(dumped));
      const configuration = typeof dumped === "string" ? false : Boolean(dumped.configuration);
      throw new SandboxError(message, configuration);
    }
    const json = context.getString(evaluated.value);
    evaluated.value.dispose();
    return JSON.parse(json) as T;
  } finally {
    context.dispose();
    runtime.dispose();
  }
}
