# Plugin contract v1 — hosted runtime implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A folder under `companion/faces/plugins/<id>/` holding a manifest and a
JavaScript file appears in the add menu and draws a real card on a picture card, with its
code sandboxed, its network declared, and its secrets held by the host.

**Architecture:** A plugin is a pure function. The host runs `plan()` to learn which
requests it needs, performs them itself through the existing guarded fetch, runs `plan()`
again if more rounds are asked for (max 3), then runs `render()` and converts what it
returns into the 448x368 PNG the faces package already produces. The plugin's code runs in
QuickJS compiled to WebAssembly with no imports at all — no `fetch`, no `require`, no
files — under a memory cap and a deadline. Discovered plugins are adapted to the existing
`FaceDefinition` shape so `describe`/`render` and the whole Rust side need no new seam.

**Tech Stack:** TypeScript on Bun; `quickjs-emscripten` (sandbox); `satori` (flexbox
layout to SVG); `@resvg/resvg-js` (already present, rasterizes); Rust for one cadence
change in `crates/server`.

**Spec:** `docs/superpowers/specs/2026-09-23-deskmate-plugin-contract-design.md`

## Global Constraints

- **This plan is sequenced behind Track C1's PR #4** (`track-c1-tap-to-face`). It targets
  the post-C1 seam: `FaceDefinition.render(settings, now, context)` returning
  `string | {svg, state?}`, `describe` entries carrying an optional `tap`, and the
  `{png: "<base64>", state?}` render envelope. **Do not start Task 9 until PR #4 is
  merged into `main` and this branch is rebased on it.** Tasks 1–8 touch only new files
  and are safe before the merge.
- **No config-schema change, no wire change, no firmware change.** The schema/wire lock
  stays free. If a task appears to need one, stop and raise it.
- **A plugin is not a card kind.** It is an image source's producer, exactly like today's
  faces (`CLAUDE.md`).
- Scope of this plan is the **hosted** tier only. Remote plugins (`remote.json`), the
  self-host runner and the directory get their own plan.
- Limits, copied verbatim from the spec: plugin source **1 MB**; memory per call
  **16 MB**; time per call **2 s**; rounds **3**; requests per render **8**; response body
  **1 MB each, 4 MB total**; measurements **64 per render**; boxes in a card **2,000**;
  SVG out **512 KB**; PNG out **1 MB**; state out **16 KB encoded**.
- Verification for every task, from `companion/faces/`:
  `bun test && bun run check && bun run lint && bun run format:check`.
  Rust tasks additionally, from `companion/`: `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --all-targets`, `cargo test --workspace --doc`.
  `cargo` is not on the Bash tool's PATH: `export PATH="$HOME/.cargo/bin:$PATH"`, and
  never pipe cargo into `tail` — redirect to a file and check `$?`.
- Commit with conventional prefixes. Do not rewrite shared history.

## File structure

| File | Responsibility |
|---|---|
| `companion/faces/src/plugins/manifest.ts` | Parse and validate `plugin.json`; the `PluginManifest` type |
| `companion/faces/src/plugins/sandbox.ts` | Run one plugin function in QuickJS under a memory cap and deadline |
| `companion/faces/src/plugins/context.ts` | Build `now`/`format` — local time and formatting, since the sandbox has no `Intl` |
| `companion/faces/src/plugins/requests.ts` | Validate declared requests, enforce the allowlist, substitute secrets, perform them |
| `companion/faces/src/plugins/card.ts` | Convert the returned card to SVG: layout via satori, or the SVG/PNG outputs, with caps |
| `companion/faces/src/plugins/run.ts` | The rounds: plan → fetch → plan → render → card; the error taxonomy; state |
| `companion/faces/src/plugins/discovery.ts` | List plugin folders, load them in isolation, adapt each to a `FaceDefinition` |
| `companion/faces/src/kit/http.ts` | *Modify:* a request function with method, headers, body and bytes on the same guard |
| `companion/faces/src/registry.ts` | *Modify:* built-in faces plus discovered plugins |
| `companion/faces/plugins/github-stats/` | The worked example, and the fixture the tests render |
| `companion/crates/server/src/data_cards/faces_package.rs` | *Modify:* `CatalogFace` gains `refresh_seconds` |
| `companion/crates/server/src/data_cards.rs` | *Modify:* a new spec takes the catalog's cadence, clamped |
| `docs/plugins/contract-v1.md` | The contract, written for an author |

---

### Task 1: The manifest

**Files:**
- Create: `companion/faces/src/plugins/manifest.ts`
- Test: `companion/faces/test/plugins/manifest.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `parseManifest(raw: unknown, folder: string): PluginManifest` (throws
  `ManifestError`), and the types `PluginManifest`, `SecretSpec`.

```ts
export interface SecretSpec {
  key: string;
  label: string;
  kind: "api_key";
  host: string;
  send_as: "bearer" | "header" | "query";
  header?: string;
  param?: string;
}
export interface PluginManifest {
  api: 1;
  id: string;
  version: string;
  label: string;
  description: string;
  author: string;
  hosts: string[];
  secrets: SecretSpec[];
  fields: FieldSpec[];
  refreshSeconds?: number;
  tap?: string;
}
```

- [ ] **Step 1: Write the failing test**

```ts
// companion/faces/test/plugins/manifest.test.ts
import { describe, expect, test } from "bun:test";
import { ManifestError, parseManifest } from "../../src/plugins/manifest";

const valid = {
  api: 1, id: "github-stats", version: "1.0.0", label: "GitHub stats",
  description: "Commits and stars.", author: "Acme",
  hosts: ["api.github.com"],
  secrets: [{ key: "token", label: "GitHub token", kind: "api_key", host: "api.github.com", send_as: "bearer" }],
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
    for (const host of ["https://api.github.com", "api.github.com/x", "*", "10.0.0.1", ""]) {
      expect(() => parseManifest({ ...valid, hosts: [host] }, "github-stats")).toThrow(ManifestError);
    }
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
    expect(() => parseManifest({ ...valid, refreshSeconds: "900" }, "github-stats")).toThrow(ManifestError);
  });
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cd companion/faces && bun test test/plugins/manifest.test.ts`
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Implement**

```ts
// companion/faces/src/plugins/manifest.ts
import { isIP } from "node:net";
import type { FieldSpec } from "../face";

export class ManifestError extends Error {
  override name = "ManifestError";
}

const HOSTNAME = /^(?!-)[a-z0-9-]{1,63}(?<!-)(\.(?!-)[a-z0-9-]{1,63}(?<!-))+$/;

/**
 * A name, never an address. `HOSTNAME` alone accepts a dotted quad, because digits
 * are legal label characters -- and an accepted "169.254.169.254" becomes an
 * allowlisted destination in `requests.ts`, which matches on the hostname only.
 */
function isName(host: string): boolean {
  return HOSTNAME.test(host) && isIP(host) === 0 && !host.startsWith("[");
}

function text(source: Record<string, unknown>, key: string): string {
  const value = source[key];
  if (typeof value !== "string" || value.trim() === "") {
    throw new ManifestError(`${key} must be a non-empty string`);
  }
  return value.trim();
}

export function parseManifest(raw: unknown, folder: string): PluginManifest {
  if (typeof raw !== "object" || raw === null) throw new ManifestError("the manifest is not an object");
  const source = raw as Record<string, unknown>;
  if (source.api !== 1) throw new ManifestError(`unsupported api ${JSON.stringify(source.api)}; this host speaks api 1`);
  const id = text(source, "id");
  if (id !== folder) throw new ManifestError(`id ${JSON.stringify(id)} does not match its folder ${JSON.stringify(folder)}`);
  const hosts = Array.isArray(source.hosts) ? source.hosts : [];
  for (const host of hosts) {
    if (typeof host !== "string" || !isName(host)) {
      throw new ManifestError(`${JSON.stringify(host)} is not a bare hostname, such as "api.github.com"`);
    }
  }
  const secrets = (Array.isArray(source.secrets) ? source.secrets : []) as SecretSpec[];
  for (const secret of secrets) {
    if (!hosts.includes(secret.host)) {
      throw new ManifestError(`secret ${JSON.stringify(secret.key)} names ${secret.host}, which the manifest has not declared`);
    }
  }
  if (secrets.length > 0) {
    const allowed = new Set(secrets.map((secret) => secret.host));
    const extra = hosts.filter((host: string) => !allowed.has(host));
    if (extra.length > 0) {
      throw new ManifestError(`a plugin using a secret may declare only the hosts its secrets belong to; remove ${extra.join(", ")}`);
    }
  }
  const refreshSeconds = source.refreshSeconds;
  if (refreshSeconds !== undefined && (typeof refreshSeconds !== "number" || !Number.isInteger(refreshSeconds))) {
    throw new ManifestError("refreshSeconds must be a whole number of seconds");
  }
  return {
    api: 1, id, version: text(source, "version"), label: text(source, "label"),
    description: text(source, "description"), author: text(source, "author"),
    hosts: hosts as string[], secrets,
    fields: (Array.isArray(source.fields) ? source.fields : []) as FieldSpec[],
    ...(refreshSeconds === undefined ? {} : { refreshSeconds }),
    ...(typeof source.tap === "string" ? { tap: source.tap } : {}),
  };
}
```

- [ ] **Step 4: Run the tests and the gates**

Run: `cd companion/faces && bun test test/plugins/manifest.test.ts && bun run check && bun run lint`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add companion/faces/src/plugins/manifest.ts companion/faces/test/plugins/manifest.test.ts
git commit -m "feat(plugins): the manifest, and the rule that a secret pins the host list"
```

---

### Task 2: The sandbox

**Files:**
- Create: `companion/faces/src/plugins/sandbox.ts`
- Test: `companion/faces/test/plugins/sandbox.test.ts`
- Modify: `companion/faces/package.json` (add `quickjs-emscripten`)

**Interfaces:**
- Consumes: nothing.
- Produces: `runInSandbox<T>(source: string, fn: "plan" | "render", input: unknown, limits?: Partial<SandboxLimits>): T`,
  `SandboxError`, `SANDBOX_LIMITS: SandboxLimits` (`{ memoryBytes: 16_777_216, deadlineMs: 2_000, sourceBytes: 1_048_576 }`).

- [ ] **Step 1: Add the dependency**

```bash
cd companion/faces && bun add quickjs-emscripten
```

- [ ] **Step 2: Write the failing test**

```ts
// companion/faces/test/plugins/sandbox.test.ts
import { describe, expect, test } from "bun:test";
import { SandboxError, runInSandbox } from "../../src/plugins/sandbox";

const plugin = `export function render(context) { return { seen: context.settings.user }; }`;

describe("the sandbox", () => {
  test("runs a plugin function and returns its value", () => {
    expect(runInSandbox(plugin, "render", { settings: { user: "octocat" } })).toEqual({ seen: "octocat" });
  });

  test("stops an infinite loop at the deadline", () => {
    const start = Date.now();
    expect(() => runInSandbox(`export function render(){ while(true){} }`, "render", {}, { deadlineMs: 300 }))
      .toThrow(SandboxError);
    expect(Date.now() - start).toBeLessThan(3_000);
  });

  test("stops an allocation loop at the memory cap", () => {
    expect(() => runInSandbox(`export function render(){ const a=[]; for(;;) a.push(new Array(100000).fill("x")); }`,
      "render", {}, { memoryBytes: 8 * 1024 * 1024, deadlineMs: 5_000 })).toThrow(SandboxError);
  });

  test("has no way to reach files or the network", () => {
    expect(() => runInSandbox(`export function render(){ return require("fs").readFileSync("/etc/passwd","utf8"); }`, "render", {}))
      .toThrow(/require/);
    expect(() => runInSandbox(`export function render(){ return fetch("https://example.com"); }`, "render", {}))
      .toThrow(/fetch/);
  });

  test("sees only its own function among the globals it can enumerate", () => {
    const globals = runInSandbox<string[]>(`export function render(){ return Object.keys(globalThis); }`, "render", {});
    expect(globals).toEqual(["render"]);
  });

  test("reports a plugin's own error with its message", () => {
    expect(() => runInSandbox(`export function render(){ throw new Error("boom"); }`, "render", {})).toThrow(/boom/);
  });

  test("refuses a source over the size cap without running it", () => {
    expect(() => runInSandbox(`export function render(){ return 1; }//${"x".repeat(1_048_576)}`, "render", {}))
      .toThrow(/too large/);
  });

  test("refuses a function the plugin does not export", () => {
    expect(() => runInSandbox(`export function render(){ return 1; }`, "plan", {})).toThrow(/plan/);
  });
});
```

- [ ] **Step 3: Run it and watch it fail**

Run: `cd companion/faces && bun test test/plugins/sandbox.test.ts`
Expected: FAIL — the module does not exist.

- [ ] **Step 4: Implement**

```ts
// companion/faces/src/plugins/sandbox.ts
//
// A plugin runs here and nowhere else: QuickJS compiled to WebAssembly, with no
// imports at all. It cannot open a file, reach the network or see our environment,
// because none of those functions exist inside it -- not because they are guarded.
import { type QuickJSWASMModule, getQuickJS } from "quickjs-emscripten";

export class SandboxError extends Error {
  override name = "SandboxError";
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
  const size = Buffer.byteLength(source, "utf8");
  if (size > sourceBytes) {
    throw new SandboxError(`this plugin's source is too large: ${size} bytes, the limit is ${sourceBytes}`);
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
    // Anchored to statement position, or a plugin's own string containing the text
    // "export const" is silently mangled. The real cure is the publish-time bundle,
    // which need not contain `export` at all (Task 9).
    const script = `${source.replace(/^[ \t]*export[ \t]+(?=function|const|let)/gm, "")}
if (typeof ${fn} !== "function") { throw new Error("this plugin exports no ${fn}()"); }
${fn}(${JSON.stringify(input)}) ?? null`;
    const evaluated = context.evalCode(script);
    if (evaluated.error) {
      // Anything can be thrown, including null, so normalize every shape: every
      // path out of this function is a SandboxError, because that is what the
      // error taxonomy catches.
      const dumped: unknown = context.dump(evaluated.error);
      evaluated.error.dispose();
      const asObject = typeof dumped === "object" && dumped !== null ? (dumped as { message?: unknown; configuration?: unknown }) : undefined;
      const message = typeof asObject?.message === "string" ? asObject.message : String(dumped);
      const failure = new SandboxError(message);
      failure.configuration = Boolean(asObject?.configuration);
      throw failure;
    }
    // Read the value HOST-side. Round-tripping through the sandbox's own
    // JSON.stringify lets a plugin reassign it and choose what the host receives,
    // and a non-serializable return value came back as the text "undefined",
    // which the host's own JSON.parse then threw on, outside SandboxError.
    const value = context.dump(evaluated.value) as T;
    evaluated.value.dispose();
    return value;
  } finally {
    context.dispose();
    runtime.dispose();
  }
}
```

Note for the implementer: `runInSandbox` is synchronous on purpose — a pure function
needs no `await`, and keeping it synchronous is what makes the deadline a real deadline.
`warmSandbox()` is called once by `run.ts`; the tests call it in a `beforeAll`.

- [ ] **Step 5: Add the warm-up to the test file**

```ts
import { beforeAll } from "bun:test";
import { warmSandbox } from "../../src/plugins/sandbox";
beforeAll(async () => { await warmSandbox(); });
```

- [ ] **Step 6: Run the tests and the gates**

Run: `cd companion/faces && bun test test/plugins/sandbox.test.ts && bun run check && bun run lint`
Expected: PASS, including the two that must be *stopped* rather than merely slow.

- [ ] **Step 7: Commit**

```bash
git add companion/faces/src/plugins/sandbox.ts companion/faces/test/plugins/sandbox.test.ts companion/faces/package.json companion/faces/bun.lock
git commit -m "feat(plugins): a plugin runs in QuickJS with no imports, a memory cap and a deadline"
```

---

### Task 3: Local time and formatting

**Files:**
- Create: `companion/faces/src/plugins/context.ts`
- Test: `companion/faces/test/plugins/context.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `buildNow(instant: Date, timezone: string): NowContext` and
  `FORMAT_SOURCE: string` — the `format` helpers as source text, prepended to a plugin
  before evaluation.

```ts
export interface NowContext {
  utc: string;                   // RFC 3339, always UTC
  timezone: string;              // IANA name
  local: { year: number; month: number; day: number; hour: number; minute: number;
           weekday: string; offsetMinutes: number; iso: string };
}
```

Why this task exists: **the sandbox has no `Intl` and its clock is UTC** (measured). A
plugin that does this arithmetic itself gets it wrong, so the host does it once.

- [ ] **Step 1: Write the failing test**

```ts
// companion/faces/test/plugins/context.test.ts
import { describe, expect, test } from "bun:test";
import { FORMAT_SOURCE, buildNow } from "../../src/plugins/context";

describe("buildNow", () => {
  test("converts to the card's time zone, not the host's", () => {
    const now = buildNow(new Date("2026-09-23T10:00:00Z"), "Asia/Dubai");
    expect(now.local.hour).toBe(14);
    expect(now.local.offsetMinutes).toBe(240);
    expect(now.utc).toBe("2026-09-23T10:00:00.000Z");
  });

  test("handles a zone with a half-hour offset and one on the other side of midnight", () => {
    expect(buildNow(new Date("2026-09-23T10:00:00Z"), "Asia/Kolkata").local.minute).toBe(30);
    const honolulu = buildNow(new Date("2026-09-23T06:00:00Z"), "Pacific/Honolulu").local;
    expect([honolulu.day, honolulu.hour]).toEqual([22, 20]);
  });

  test("falls back to UTC for a zone it does not know, rather than throwing", () => {
    expect(buildNow(new Date("2026-09-23T10:00:00Z"), "Mars/Olympus").timezone).toBe("UTC");
  });
});

describe("FORMAT_SOURCE", () => {
  test("formats numbers with separators, since the sandbox has no Intl", async () => {
    const { runInSandbox, warmSandbox } = await import("../../src/plugins/sandbox");
    await warmSandbox();
    const plugin = `${FORMAT_SOURCE}
export function render(){ return [format.number(324800), format.number(12.5, 2), format.compact(324800)]; }`;
    expect(runInSandbox(plugin, "render", {})).toEqual(["324,800", "12.50", "325K"]);
  });

  test("formats a date and a relative age from the context it is given", async () => {
    const { runInSandbox } = await import("../../src/plugins/sandbox");
    const plugin = `${FORMAT_SOURCE}
export function render(c){ return [format.date(c.now.local, "d MMM"), format.since(c.now.utc, "2026-09-23T08:30:00Z")]; }`;
    const { buildNow } = await import("../../src/plugins/context");
    const out = runInSandbox<string[]>(plugin, "render", { now: buildNow(new Date("2026-09-23T10:00:00Z"), "Asia/Dubai") }, {});
    expect(out).toEqual(["23 Sep", "1h ago"]);
  });
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cd companion/faces && bun test test/plugins/context.test.ts`
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Implement `buildNow`**

```ts
// companion/faces/src/plugins/context.ts
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

export function buildNow(instant: Date, timezone: string): NowContext {
  let parts: Intl.DateTimeFormatPart[];
  let zone = timezone;
  try {
    parts = new Intl.DateTimeFormat("en-GB", {
      timeZone: timezone, year: "numeric", month: "2-digit", day: "2-digit",
      hour: "2-digit", minute: "2-digit", weekday: "short", hour12: false,
    }).formatToParts(instant);
  } catch {
    zone = "UTC";
    parts = new Intl.DateTimeFormat("en-GB", {
      timeZone: "UTC", year: "numeric", month: "2-digit", day: "2-digit",
      hour: "2-digit", minute: "2-digit", weekday: "short", hour12: false,
    }).formatToParts(instant);
  }
  const at = (type: string): string => parts.find((part) => part.type === type)?.value ?? "";
  const [year, month, day] = [Number(at("year")), Number(at("month")), Number(at("day"))];
  const hour = Number(at("hour")) % 24;
  const minute = Number(at("minute"));
  const asUtc = Date.UTC(year, month - 1, day, hour, minute);
  const offsetMinutes = Math.round((asUtc - instant.getTime() + instant.getSeconds() * 1000) / 60_000);
  const pad = (value: number): string => String(value).padStart(2, "0");
  return {
    utc: instant.toISOString(),
    timezone: zone,
    local: {
      year, month, day, hour, minute, weekday: at("weekday"),
      offsetMinutes,
      iso: `${year}-${pad(month)}-${pad(day)}T${pad(hour)}:${pad(minute)}`,
    },
  };
}
```

- [ ] **Step 4: Implement `FORMAT_SOURCE`**

It is a string because it is injected into the sandbox, where it becomes ordinary code
the plugin calls. Keep it dependency-free and deterministic.

```ts
export const FORMAT_SOURCE = `
const format = {
  number(value, decimals) {
    const fixed = typeof decimals === "number" ? Number(value).toFixed(decimals) : String(Math.round(Number(value)));
    const [whole, fraction] = fixed.split(".");
    const grouped = whole.replace(/\\B(?=(\\d{3})+(?!\\d))/g, ",");
    return fraction === undefined ? grouped : grouped + "." + fraction;
  },
  compact(value) {
    const n = Number(value);
    const units = [[1e9, "B"], [1e6, "M"], [1e3, "K"]];
    for (const [size, suffix] of units) {
      if (Math.abs(n) >= size) {
        const scaled = n / size;
        return (Math.abs(scaled) >= 100 ? Math.round(scaled) : Math.round(scaled * 10) / 10) + suffix;
      }
    }
    return String(Math.round(n));
  },
  date(local, pattern) {
    const MONTHS = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
    const pad = (v) => String(v).padStart(2, "0");
    return pattern
      .replace("MMM", MONTHS[local.month - 1])
      .replace("yyyy", String(local.year))
      .replace("HH", pad(local.hour))
      .replace("mm", pad(local.minute))
      .replace("dd", pad(local.day))
      .replace("d", String(local.day));
  },
  since(nowIso, thenIso) {
    const seconds = Math.max(0, Math.round((Date.parse(nowIso) - Date.parse(thenIso)) / 1000));
    if (seconds < 60) return "just now";
    if (seconds < 3600) return Math.floor(seconds / 60) + "m ago";
    if (seconds < 86400) return Math.floor(seconds / 3600) + "h ago";
    return Math.floor(seconds / 86400) + "d ago";
  },
};
`;
```

- [ ] **Step 5: Run the tests and the gates**

Run: `cd companion/faces && bun test test/plugins/context.test.ts && bun run check && bun run lint`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add companion/faces/src/plugins/context.ts companion/faces/test/plugins/context.test.ts
git commit -m "feat(plugins): local time and formatting, because the sandbox has no Intl"
```

---

### Task 4: A request with a method, headers, a body and bytes

**Files:**
- Modify: `companion/faces/src/kit/http.ts`
- Test: `companion/faces/test/http.test.ts` (extend)

**Interfaces:**
- Consumes: the existing `pinnedAddress`, `dial`, `isPrivateAddress`, `USER_AGENT`.
- Produces: `createRequest(resolve?: Resolve): RequestFn` and
  `request: RequestFn`, where
  `type RequestFn = (input: { url: string; method?: "GET" | "POST"; headers?: Record<string, string>; body?: string; as: "json" | "text" | "bytes" }) => Promise<HttpReply>`
  and `interface HttpReply { status: number; body: string | Uint8Array; json?: unknown }`.

The guard is unchanged and must stay that way: http(s) only, private/loopback/link-local/
CGNAT/metadata refused on **every** redirect hop, the resolved address pinned with the
name in `Host` and as the TLS server name, 1 MB body cap, deadline. This task adds a
method, headers, a body and a bytes result on top of it; `fetchText` keeps its behaviour
so the four existing faces are untouched.

- [ ] **Step 1: Write the failing tests**

Follow the file's existing pattern: a real `Bun.serve` on 127.0.0.1 plus injection,
never a stubbed global `fetch`.

**Note, learned the hard way:** a request cannot reach a loopback server *through*
`createRequest`, because the guard refuses loopback — that is its job. So `createRequest`
takes its transport by injection too, exactly as it takes its resolver:
`createRequest(resolve: Resolve = systemResolve, dialFn: DialFn = dial)`. A test then
gives it a resolver returning a PUBLIC address, so the real guard runs and passes, and an
injected dial that answers from the local server. Nothing in the guard is weakened and
there is no production flag to misconfigure: only a test ever passes the second argument.
One of the tests must assert the injected dial recorded ZERO calls when the resolver
returns a private address — that is what proves the guard sits in front of the transport
rather than beside it.

```ts
// append to companion/faces/test/http.test.ts
import { createRequest } from "../src/kit/http";

describe("createRequest", () => {
  let server: ReturnType<typeof Bun.serve>;
  const seen: { method: string; auth: string | null; body: string }[] = [];
  beforeAll(() => {
    server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: async (request) => {
        const url = new URL(request.url);
        seen.push({
          method: request.method,
          auth: request.headers.get("authorization"),
          body: await request.text(),
        });
        if (url.pathname === "/json") return Response.json({ n: 7 });
        if (url.pathname === "/png") return new Response(new Uint8Array([137, 80, 78, 71]));
        if (url.pathname === "/big") return new Response("x".repeat(1024 * 1024 + 1));
        if (url.pathname === "/missing") return new Response("gone", { status: 404 });
        return new Response("text");
      },
    });
  });
  afterAll(() => server.stop(true));

  const local = () => createRequest(async () => ["127.0.0.1"]);
  const at = (path: string) => `http://local.invalid:${server.port}${path}`;

  test("returns decoded JSON, text and bytes as asked", async () => {
    expect((await local()({ url: at("/json"), as: "json" })).json).toEqual({ n: 7 });
    expect((await local()({ url: at("/text"), as: "text" })).body).toBe("text");
    const bytes = (await local()({ url: at("/png"), as: "bytes" })).body;
    expect(bytes).toBeInstanceOf(Uint8Array);
    expect(Array.from(bytes as Uint8Array)).toEqual([137, 80, 78, 71]);
  });

  test("sends the method, body and headers it was given", async () => {
    seen.length = 0;
    await local()({ url: at("/text"), method: "POST", body: '{"q":1}', headers: { Authorization: "Bearer t" }, as: "text" });
    expect(seen[0]).toEqual({ method: "POST", auth: "Bearer t", body: '{"q":1}' });
  });

  test("returns a non-ok status instead of throwing, because a 404 is the plugin's business", async () => {
    expect((await local()({ url: at("/missing"), as: "text" })).status).toBe(404);
  });

  test("refuses a private address, and names the host the owner typed", async () => {
    const inward = createRequest(async () => ["169.254.169.254"]);
    const failure = await inward({ url: "https://metadata.example/", as: "text" }).catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(ConfigurationError);
    expect((failure as Error).message).toContain("metadata.example");
  });

  test("refuses a body over the 1 MB cap", async () => {
    expect(local()({ url: at("/big"), as: "text" })).rejects.toBeInstanceOf(TransientError);
  });

  test("refuses a scheme that is not http(s)", async () => {
    expect(local()({ url: "file:///etc/passwd", as: "text" })).rejects.toBeInstanceOf(ConfigurationError);
  });
});
```

The pre-existing `createFetchText` tests in this file are the regression gate for the
four built-in faces: they must stay green and unmodified.

- [ ] **Step 2: Run them and watch them fail**

Run: `cd companion/faces && bun test test/http.test.ts`
Expected: FAIL — `createRequest` is not exported.

- [ ] **Step 3: Implement**

```ts
// companion/faces/src/kit/http.ts — additions; fetchText keeps its behaviour exactly.
export interface HttpReply {
  status: number;
  body: string | Uint8Array;
  json?: unknown;
}

export interface HttpRequest {
  url: string;
  method?: "GET" | "POST";
  headers?: Record<string, string>;
  body?: string;
  as: "json" | "text" | "bytes";
}

export type RequestFn = (input: HttpRequest) => Promise<HttpReply>;

export function createRequest(resolve: Resolve = systemResolve): RequestFn {
  return async ({ url, method = "GET", headers = {}, body, as }) => {
    const target = parseUrl(url); // the existing helper: throws ConfigurationError
    const address = await pinnedAddress(target, resolve);
    const response = await dial(target, address, AbortSignal.timeout(TIMEOUT_MS), { method, headers, body });
    const bytes = await readCapped(response, MAX_BODY_BYTES); // throws TransientError over the cap
    if (as === "bytes") return { status: response.status, body: bytes };
    const text = new TextDecoder().decode(bytes);
    if (as === "text") return { status: response.status, body: text };
    try {
      return { status: response.status, body: text, json: JSON.parse(text) };
    } catch {
      return { status: response.status, body: text };
    }
  };
}

export const request: RequestFn = createRequest();
```

`dial` gains an optional fourth parameter `{ method, headers, body }`, defaulting to
today's behaviour so `fetchText` is untouched. **The guard's own headers win the merge**:
a caller must not be able to set `Host` in any casing, because the guard sets it
deliberately as part of dialling a pinned IP by name, and a plugin that can send an
arbitrary `Host` to a validated public address has a live primitive against anything that
routes internally by `Host` once past the IP and SNI checks. Reserve those names
explicitly rather than relying on spread order, so the rule is visible.

`createRequest` wraps its failures the way `createFetchText` does — `ConfigurationError`
for what the owner must fix, `TransientError` for everything else — so a deadline
surfaces as this package's vocabulary rather than a native `TimeoutError` DOMException. Redirect handling, address pinning and the
`Host`/TLS-name rules are the existing ones and must not be rewritten. Factor the
body-reading cap into `readCapped` if it is currently inline in `createFetchText`, and
have both callers use it.

- [ ] **Step 4: Run the tests and the gates**

Run: `cd companion/faces && bun test && bun run check && bun run lint`
Expected: PASS, including every pre-existing http test.

- [ ] **Step 5: Commit**

```bash
git add companion/faces/src/kit/http.ts companion/faces/test/http.test.ts
git commit -m "feat(faces): the guarded fetch gains a method, headers, a body and bytes"
```

---

### Task 5: Declared requests, the allowlist and secret substitution

**Files:**
- Create: `companion/faces/src/plugins/requests.ts`
- Test: `companion/faces/test/plugins/requests.test.ts`

**Interfaces:**
- Consumes: `PluginManifest` (Task 1), `RequestFn` (Task 4).
- Produces:
  `validateRequests(raw: unknown, manifest: PluginManifest, budget: Budget): PluginRequest[]`
  and `performRequests(requests: PluginRequest[], manifest: PluginManifest, secrets: Record<string, string>, request: RequestFn, budget: Budget): Promise<Answer[]>`,
  with `interface Budget { requests: number; bytes: number; measurements: number }` and
  `type Answer = { ok: true; status: number; json?: unknown; text?: string; base64?: string } | { ok: false; status?: number; error: string }`.

- [ ] **Step 1: Write the failing test**

```ts
// companion/faces/test/plugins/requests.test.ts
import { describe, expect, test } from "bun:test";
import { ConfigurationError } from "../../src/face";
import { performRequests, validateRequests } from "../../src/plugins/requests";
import { parseManifest } from "../../src/plugins/manifest";

const manifest = parseManifest({
  api: 1, id: "p", version: "1.0.0", label: "P", description: "d", author: "a",
  hosts: ["api.github.com"],
  secrets: [{ key: "token", label: "T", kind: "api_key", host: "api.github.com", send_as: "bearer" }],
  fields: [],
}, "p");
const budget = { requests: 8, bytes: 4 * 1024 * 1024, measurements: 64 };

describe("validateRequests", () => {
  test("accepts a declared host", () => {
    expect(validateRequests([{ url: "https://api.github.com/users/o", as: "json" }], manifest, budget)).toHaveLength(1);
  });

  test("refuses an undeclared host, naming it, as a configuration error", () => {
    expect(() => validateRequests([{ url: "https://evil.example/", as: "json" }], manifest, budget))
      .toThrow(/evil.example/);
    expect(() => validateRequests([{ url: "https://evil.example/", as: "json" }], manifest, budget))
      .toThrow(ConfigurationError);
  });

  test("refuses more requests than the budget allows", () => {
    const many = Array.from({ length: 9 }, () => ({ url: "https://api.github.com/u", as: "json" as const }));
    expect(() => validateRequests(many, manifest, budget)).toThrow(/at most 8/);
  });

  test("refuses a scheme that is not http(s)", () => {
    expect(() => validateRequests([{ url: "file:///etc/passwd", as: "text" }], manifest, budget)).toThrow(ConfigurationError);
  });
});

describe("performRequests", () => {
  test("substitutes a secret for its own host and never returns it", async () => {
    const seen: { url: string; headers?: Record<string, string> }[] = [];
    const stub = async (input) => { seen.push(input); return { status: 200, body: "{}", json: {} }; };
    const answers = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "json", headers: { Authorization: "{{secret:token}}" } }], manifest, budget),
      manifest, { token: "ghp_real" }, stub, budget,
    );
    expect(seen[0]?.headers?.Authorization).toBe("Bearer ghp_real");
    expect(JSON.stringify(answers)).not.toContain("ghp_real");
  });

  test("refuses to substitute a secret into a request for another host", async () => {
    // Two hosts cannot coexist with a secret (Task 1), so this manifest has none;
    // the placeholder must survive as a literal rather than leaking the value.
    const open = parseManifest({ ...JSON.parse(JSON.stringify(manifest)), secrets: [], hosts: ["api.github.com", "example.com"] }, "p");
    const seen: unknown[] = [];
    const stub = async (input) => { seen.push(input); return { status: 200, body: "", json: undefined }; };
    await performRequests(
      validateRequests([{ url: "https://example.com/x", as: "text", headers: { Authorization: "{{secret:token}}" } }], open, budget),
      open, { token: "ghp_real" }, stub, budget,
    );
    expect(JSON.stringify(seen)).not.toContain("ghp_real");
  });

  test("a failed request becomes an answer, not an exception", async () => {
    const stub = async () => { throw new Error("connection reset"); };
    const [answer] = await performRequests(
      validateRequests([{ url: "https://api.github.com/u", as: "json" }], manifest, budget), manifest, {}, stub, budget);
    expect(answer).toEqual({ ok: false, error: "connection reset" });
  });

  test("stops once the total byte budget is spent", async () => {
    const big = "x".repeat(2 * 1024 * 1024);
    const stub = async () => ({ status: 200, body: big, json: undefined });
    const requests = validateRequests(
      [1, 2, 3].map(() => ({ url: "https://api.github.com/u", as: "text" as const })), manifest, budget);
    const answers = await performRequests(requests, manifest, {}, stub, budget);
    expect(answers[2]).toEqual({ ok: false, error: expect.stringContaining("budget") });
  });
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cd companion/faces && bun test test/plugins/requests.test.ts`
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Implement**

```ts
// companion/faces/src/plugins/requests.ts — the part that must be exactly right.
const PLACEHOLDER = /\{\{secret:([a-z0-9_]+)\}\}/gi;

/**
 * A secret is substituted ONLY into a request whose host is that secret's own.
 * An unmatched placeholder stays as literal text: leaking the value is the one
 * outcome that must never happen, and a visibly wrong header is debuggable.
 */
function applySecrets(
  target: URL,
  headers: Record<string, string>,
  manifest: PluginManifest,
  secrets: Record<string, string>,
): Record<string, string> {
  const host = target.hostname.toLowerCase();
  const out: Record<string, string> = {};
  for (const [name, value] of Object.entries(headers)) {
    out[name] = value.replace(PLACEHOLDER, (literal, key: string) => {
      const spec = manifest.secrets.find((secret) => secret.key === key);
      if (spec === undefined || spec.host.toLowerCase() !== host) return literal;
      const stored = secrets[key];
      if (stored === undefined) return literal;
      return spec.send_as === "bearer" ? `Bearer ${stored}` : stored;
    });
  }
  return out;
}
```

Rules the rest of the implementation must hold, each already covered by a test above:

1. A URL is parsed; anything but `http:`/`https:` is a `ConfigurationError`.
2. Its hostname must appear in `manifest.hosts`, compared lowercased, exactly — no
   suffix matching, no wildcards. **An IP-literal host is refused here too**, even though
   Task 1 already refuses one in a manifest: this check is what stands between a plugin
   and the metadata address, and it must not depend on another file having been careful.
3. `{{secret:<key>}}` is replaced **only** when the request's host equals that secret's
   `host` **and the position matches its declared `send_as`**: `bearer` and `header`
   substitute in a header only (`bearer` as `Bearer <value>`), `query` in a query
   parameter only. A placeholder anywhere else stays literal. The declaration says where
   a credential belongs, and a bearer secret pasted into a query string puts it somewhere
   URLs get logged, cached and sent in `Referer`. One helper holds the rule, so the header
   and query paths cannot drift.
3b. **Every outward-facing value is scrubbed of the secret values this render was given**
   — error text AND response bodies, in one place. The success path is the one that
   matters: a plugin does not need a network error to read its user's credential, it
   declares a host that echoes request headers and reads the 200. Text and JSON bodies
   are scrubbed (nested values included); a bytes answer that CONTAINS the secret's UTF-8
   or base64 form is refused outright, because scrubbing binary is meaningless and a
   legitimate image never contains the user's API key. The real fetch does not put headers in its
   error messages today, but relying on that is the same "another file is currently
   careful" dependency the host check refuses to make.
4. A failure of one request is an `Answer` with `ok: false`; the plugin decides what to
   do. Only a violation of the contract itself throws.
5. Bytes come back base64-encoded, because the answer crosses into the sandbox as JSON.
6. The byte budget is decremented per response; once spent, later requests fail with a
   budget message rather than being performed.

- [ ] **Step 4: Run the tests and the gates**

Run: `cd companion/faces && bun test test/plugins/requests.test.ts && bun run check && bun run lint`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add companion/faces/src/plugins/requests.ts companion/faces/test/plugins/requests.test.ts
git commit -m "feat(plugins): declared requests, the allowlist, and secrets the plugin never sees"
```

---

### Task 6: The card — layout, SVG and PNG

**Files:**
- Create: `companion/faces/src/plugins/card.ts`
- Test: `companion/faces/test/plugins/card.test.ts`
- Modify: `companion/faces/package.json` (add `satori`)

**Interfaces:**
- Consumes: `CANVAS_WIDTH`, `CANVAS_HEIGHT` from `../kit/theme`.
- Produces: `cardToSvg(card: unknown): Promise<string>` and `CardError`.
  A card is `{ layout: LayoutNode } | { svg: string } | { png: string /* base64 */ }`.

- [ ] **Step 1: Add the dependency**

```bash
cd companion/faces && bun add satori
```

- [ ] **Step 2: Write the failing test**

```ts
// companion/faces/test/plugins/card.test.ts
import { describe, expect, test } from "bun:test";
import { CardError, cardToSvg } from "../../src/plugins/card";
import { pngFromSvg } from "../../src/kit/raster";

const box = (children: unknown) => ({
  layout: { type: "div", style: { display: "flex", width: 448, height: 368, background: "#000", fontFamily: "Inter" }, children },
});

describe("cardToSvg", () => {
  test("lays out a flexbox card at the panel's size", async () => {
    const svg = await cardToSvg(box({ type: "div", style: { fontSize: 72, color: "#f5f5f7" }, children: "128" }));
    expect(svg).toContain('width="448"');
    const png = pngFromSvg(svg);
    const view = new DataView(png.buffer, png.byteOffset, png.byteLength);
    expect([view.getUint32(16), view.getUint32(20)]).toEqual([448, 368]);
  });

  test("wraps long text instead of overflowing, which is the whole point of the layout path", async () => {
    const svg = await cardToSvg(box({ type: "div", style: { fontSize: 31 }, children: "a headline long enough to need two lines on a 448 pixel panel" }));
    expect(svg.match(/<text/g)?.length ?? 0).toBeGreaterThan(1);
  });

  test("refuses a tree over the box cap", async () => {
    let deep: unknown = "leaf";
    for (let i = 0; i < 2_001; i += 1) deep = { type: "div", style: {}, children: deep };
    await expect(cardToSvg(box(deep))).rejects.toThrow(/boxes/);
  });

  test("passes an SVG card through, and refuses one that references our disk", async () => {
    await expect(cardToSvg({ svg: '<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"></svg>' })).resolves.toContain("<svg");
    await expect(cardToSvg({ svg: '<svg xmlns="http://www.w3.org/2000/svg"><image href="/etc/hosts.png"/></svg>' })).rejects.toThrow(CardError);
    await expect(cardToSvg({ svg: '<svg xmlns="http://www.w3.org/2000/svg"><image href="https://example.com/a.png"/></svg>' })).rejects.toThrow(CardError);
  });

  test("accepts an embedded data: image in an SVG card", async () => {
    const tiny = "data:image/png;base64,iVBORw0KGgo=";
    await expect(cardToSvg({ svg: `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><image href="${tiny}"/></svg>` })).resolves.toContain("<image");
  });

  test("refuses an SVG over the size cap", async () => {
    await expect(cardToSvg({ svg: `<svg xmlns="http://www.w3.org/2000/svg">${"<g/>".repeat(200_000)}</svg>` })).rejects.toThrow(/512/);
  });

  test("wraps a PNG card as a full-canvas image", async () => {
    const png = Buffer.from(pngFromSvg('<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"></svg>')).toString("base64");
    expect(await cardToSvg({ png })).toContain("data:image/png;base64,");
  });
});
```

- [ ] **Step 3: Run it and watch it fail**

Run: `cd companion/faces && bun test test/plugins/card.test.ts`
Expected: FAIL — the module does not exist.

- [ ] **Step 4: Implement**

```ts
// companion/faces/src/plugins/card.ts
//
// What a plugin returns becomes an SVG here, and only here. The layout path goes
// through satori (flexbox), which is why a plugin never places text by hand and never
// meets `textWidth`'s letter-spacing trap. The SVG and PNG paths are pass-throughs
// with caps.
import satori from "satori";
import { CANVAS_HEIGHT, CANVAS_WIDTH } from "../kit/theme";

export class CardError extends Error {
  override name = "CardError";
}

const MAX_BOXES = 2_000;
/**
 * A box budget alone does not protect the host: 501 NESTED boxes -- far inside the
 * 2,000 budget -- crash yoga's WebAssembly with an out-of-bounds access that
 * permanently corrupts the shared module, so every later card in the process fails,
 * including other plugins' valid ones. Measured: 500 renders, 501 crashes.
 */
const MAX_DEPTH = 400;
const MAX_SVG_BYTES = 512 * 1024;
const MAX_PNG_BYTES = 1024 * 1024;

let fonts: { name: string; data: ArrayBuffer; weight: 400 | 600; style: "normal" }[] | undefined;

async function loadFonts() {
  fonts ??= [
    { name: "Inter", data: await Bun.file(new URL("../../assets/fonts/Inter-Regular.ttf", import.meta.url)).arrayBuffer(), weight: 400, style: "normal" },
    { name: "Inter", data: await Bun.file(new URL("../../assets/fonts/Inter-SemiBold.ttf", import.meta.url)).arrayBuffer(), weight: 600, style: "normal" },
  ];
  return fonts;
}

function toElement(node: unknown, counter: { boxes: number }): unknown {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (typeof node !== "object" || node === null) {
    throw new CardError("a card node must be a box, a string or a number");
  }
  counter.boxes += 1;
  if (counter.boxes > MAX_BOXES) {
    throw new CardError(`this card has more than ${MAX_BOXES} boxes`);
  }
  const { type, style, children, src } = node as { type?: unknown; style?: unknown; children?: unknown; src?: unknown };
  const kind = type === "img" ? "img" : "div";
  return {
    type: kind,
    props: {
      style: (typeof style === "object" && style !== null ? style : {}) as Record<string, unknown>,
      ...(kind === "img" && typeof src === "string" ? { src } : {}),
      ...(children === undefined ? {} : {
        children: Array.isArray(children) ? children.map((child) => toElement(child, counter)) : toElement(children, counter),
      }),
    },
  };
}

/** Every reference an SVG card may carry: embedded data only. */
function refuseExternalReferences(svg: string): void {
  // Both quote styles, and case-insensitive: SVG permits href='...' and HREF="...",
  // and a check that sees only double quotes is a file-read guard with a hole in it.
  for (const match of svg.matchAll(/(?:xlink:)?href\s*=\s*("([^"]*)"|'([^']*)')/gi)) {
    const value = (match[2] ?? match[3] ?? "").trim();
    if (!value.startsWith("data:") && !value.startsWith("#")) {
      // usvg resolves a path href FROM OUR DISK by default (usvg-0.45.1
      // src/parser/image.rs:85-100), so this is a file-read surface, not a nicety.
      throw new CardError(`an SVG card may only reference embedded data, not ${JSON.stringify(value.slice(0, 40))}`);
    }
  }
}

export async function cardToSvg(card: unknown): Promise<string> {
  if (typeof card !== "object" || card === null) throw new CardError("this plugin returned no card");
  const { layout, svg, png } = card as { layout?: unknown; svg?: unknown; png?: unknown };
  if (typeof svg === "string") {
    if (Buffer.byteLength(svg, "utf8") > MAX_SVG_BYTES) throw new CardError(`an SVG card may not exceed ${MAX_SVG_BYTES / 1024} KB`);
    refuseExternalReferences(svg);
    return svg;
  }
  if (typeof png === "string") {
    if (png.length * 0.75 > MAX_PNG_BYTES) throw new CardError(`a PNG card may not exceed ${MAX_PNG_BYTES / 1024} KB`);
    return `<svg xmlns="http://www.w3.org/2000/svg" width="${CANVAS_WIDTH}" height="${CANVAS_HEIGHT}">` +
      `<image x="0" y="0" width="${CANVAS_WIDTH}" height="${CANVAS_HEIGHT}" preserveAspectRatio="xMidYMid meet" href="data:image/png;base64,${png}"/></svg>`;
  }
  if (layout === undefined) throw new CardError("a card must be a layout, an svg or a png");
  const element = toElement(layout, { boxes: 0, depth: 0 });
  return satori(element as never, { width: CANVAS_WIDTH, height: CANVAS_HEIGHT, fonts: await loadFonts() });
}
```

- [ ] **Step 5: Run the tests and the gates**

Run: `cd companion/faces && bun test test/plugins/card.test.ts && bun run check && bun run lint`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add companion/faces/src/plugins/card.ts companion/faces/test/plugins/card.test.ts companion/faces/package.json companion/faces/bun.lock
git commit -m "feat(plugins): a card becomes an SVG — flexbox layout, or a capped SVG or PNG"
```

---

### Task 7: Measurement requests

**Files:**
- Modify: `companion/faces/src/plugins/requests.ts`
- Test: `companion/faces/test/plugins/requests.test.ts` (extend)

**Interfaces:**
- Consumes: `textWidth`, `textInk` from `../kit/raster`.
- Produces: `validateRequests` additionally accepts
  `{ measure: { text: string; size: number; weight: number }[] }` and `performRequests`
  answers it with `{ ok: true, measurements: { width: number; ink: Ink }[] }`.

Why: a plugin returning its own SVG places text by hand and needs the same widths the
renderer will use. A layout plugin never asks for this.

- [ ] **Step 1: Write the failing test**

```ts
test("answers a measure request with the renderer's own widths", async () => {
  const requests = validateRequests([{ measure: [{ text: "Hello", size: 31, weight: 600 }] }], manifest, budget);
  const [answer] = await performRequests(requests, manifest, {}, async () => { throw new Error("no network for a measure"); }, budget);
  const { textWidth } = await import("../../src/kit/raster");
  expect(answer).toEqual({ ok: true, measurements: [{ width: textWidth("Hello", 31, 600), ink: expect.any(Object) }] });
});

test("refuses more measurements than the budget allows", () => {
  const measure = Array.from({ length: 65 }, () => ({ text: "x", size: 17, weight: 400 }));
  expect(() => validateRequests([{ measure }], manifest, budget)).toThrow(/64/);
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cd companion/faces && bun test test/plugins/requests.test.ts`
Expected: FAIL — a measure request is rejected as having no URL.

- [ ] **Step 3: Implement**

Branch on the shape in both functions; a measure request performs no I/O and consumes
from `budget.measurements` rather than `budget.requests`.

- [ ] **Step 4: Run the tests and the gates**

Run: `cd companion/faces && bun test test/plugins/requests.test.ts && bun run check && bun run lint`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add companion/faces/src/plugins/requests.ts companion/faces/test/plugins/requests.test.ts
git commit -m "feat(plugins): a plugin can ask the host to measure text it will draw"
```

---

### Task 8: The rounds

**Files:**
- Create: `companion/faces/src/plugins/run.ts`
- Test: `companion/faces/test/plugins/run.test.ts`

**Interfaces:**
- Consumes: everything from Tasks 1–7.
- Produces:
  `runPlugin(input: { manifest: PluginManifest; source: string; settings: Settings; now: Date; timezone: string; state?: unknown; event?: RenderContext["event"]; secrets: Record<string, string>; request?: RequestFn }): Promise<{ svg: string; state?: unknown; log: string[] }>`.

Behaviour, each with a test below: at most 3 rounds of `plan`; `render` runs once;
`ConfigurationError` for anything the owner must fix; `TransientError` for anything worth
retrying; state over 16 KB encoded is dropped with a log line while the frame is still
published.

- [ ] **Step 1: Write the failing test**

```ts
// companion/faces/test/plugins/run.test.ts
import { beforeAll, describe, expect, test } from "bun:test";
import { ConfigurationError, TransientError } from "../../src/face";
import { parseManifest } from "../../src/plugins/manifest";
import { warmSandbox } from "../../src/plugins/sandbox";
import { runPlugin } from "../../src/plugins/run";

beforeAll(async () => { await warmSandbox(); });

const manifest = parseManifest({
  api: 1, id: "p", version: "1.0.0", label: "P", description: "d", author: "a",
  hosts: ["api.example.com"], secrets: [], fields: [],
}, "p");

const base = { manifest, settings: {}, now: new Date("2026-09-23T10:00:00Z"), timezone: "Asia/Dubai", secrets: {} };
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
    const request = async (input) => { seen.push(input.url); return { status: 200, body: "x" }; };
    await runPlugin({ ...base, source, request });
    expect(seen).toEqual(["https://api.example.com/0", "https://api.example.com/1", "https://api.example.com/2"]);
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
      export function render(c){ return ${card.replace('"ok"', 'String(c.event ? c.event.taps : 0)')}; }`;
    const result = await runPlugin({ ...base, source, event: { taps: 3, point: null } });
    expect(result.svg).toContain("3");
  });
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cd companion/faces && bun test test/plugins/run.test.ts`
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Implement**

The order is fixed: build the context (Task 3), then up to 3 rounds of
`runInSandbox(source, "plan", …)` → `validateRequests` → `performRequests`, accumulating
answers; then `runInSandbox(source, "render", …)`; then `cardToSvg`. Prepend
`FORMAT_SOURCE` to the source before every evaluation. Map errors: `ManifestError`,
`CardError` and an allowlist refusal are `ConfigurationError`; a `SandboxError` from the
deadline or the memory cap is `TransientError`; a plugin's own throw is a
`ConfigurationError` when it set `configuration = true` on the error, otherwise
`TransientError`. A plugin's `log` array is capped at 10 lines of 200 characters.

- [ ] **Step 4: Run the tests and the gates**

Run: `cd companion/faces && bun test && bun run check && bun run lint && bun run format:check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add companion/faces/src/plugins/run.ts companion/faces/test/plugins/run.test.ts
git commit -m "feat(plugins): the rounds — plan, fetch, render, card, state"
```

---

### Task 9: Discovery, isolation and the catalog

**Do not start until Track C1's PR #4 is merged and this branch is rebased on `main`.**

**Files:**
- Create: `companion/faces/src/plugins/discovery.ts`
- Create: `companion/faces/plugins/github-stats/plugin.json`, `companion/faces/plugins/github-stats/index.js`
- Modify: `companion/faces/src/registry.ts`
- Test: `companion/faces/test/plugins/discovery.test.ts`

**Interfaces:**
- Consumes: `runPlugin` (Task 8), `parseManifest` (Task 1).
- Produces: `discoverPlugins(directory?: string): Promise<{ faces: FaceDefinition[]; skipped: { folder: string; reason: string }[] }>`.
  Each returned `FaceDefinition` has `kind = manifest.id`, `label`, `fields`, `tap` and a
  `render(settings, now, context)` that calls `runPlugin` and returns `{ svg, state }`.

- [ ] **Step 1: Write the failing test**

```ts
// companion/faces/test/plugins/discovery.test.ts
import { beforeAll, describe, expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { discoverPlugins } from "../../src/plugins/discovery";
import { warmSandbox } from "../../src/plugins/sandbox";

beforeAll(async () => { await warmSandbox(); });

function fixture(): string {
  const root = join(tmpdir(), `plugins-${Math.random().toString(36).slice(2)}`);
  const write = (folder: string, manifest: unknown, source: string) => {
    mkdirSync(join(root, folder), { recursive: true });
    writeFileSync(join(root, folder, "plugin.json"), JSON.stringify(manifest));
    writeFileSync(join(root, folder, "index.js"), source);
  };
  const ok = (id: string) => ({ api: 1, id, version: "1.0.0", label: id, description: "d", author: "a", hosts: [], secrets: [], fields: [] });
  const card = `export function plan(){ return []; }
    export function render(){ return { layout: { type: "div", style: { display: "flex", width: 448, height: 368, background: "#000" }, children: "ok" } }; }`;
  write("good", ok("good"), card);
  write("broken-json", "{not json", card);
  write("wrong-api", { ...ok("wrong-api"), api: 2 }, card);
  write("id-mismatch", ok("something-else"), card);
  write("syntax-error", ok("syntax-error"), "export function plan(){ return [ }");
  mkdirSync(join(root, "empty-folder"), { recursive: true });
  return root;
}

describe("discoverPlugins", () => {
  test("one bad plugin does not empty the catalog", async () => {
    const { faces, skipped } = await discoverPlugins(fixture());
    expect(faces.map((face) => face.kind)).toEqual(["good"]);
    expect(skipped.map((entry) => entry.folder).sort())
      .toEqual(["broken-json", "empty-folder", "id-mismatch", "syntax-error", "wrong-api"]);
  });

  test("a discovered plugin renders through the FaceDefinition seam", async () => {
    const { faces } = await discoverPlugins(fixture());
    const result = await faces[0]!.render({}, new Date("2026-09-23T10:00:00Z"), {});
    expect(typeof result === "string" ? result : result.svg).toContain("<svg");
  });

  test("a folder with a syntax error is skipped at discovery, not at render", async () => {
    const { skipped } = await discoverPlugins(fixture());
    expect(skipped.find((entry) => entry.folder === "syntax-error")?.reason).toMatch(/plan|syntax|parse/i);
  });

  test("a missing plugins directory is no plugins, not an error", async () => {
    await expect(discoverPlugins(join(tmpdir(), "definitely-not-here"))).resolves.toEqual({ faces: [], skipped: [] });
  });
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cd companion/faces && bun test test/plugins/discovery.test.ts`
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Implement discovery**

Read `DESKMATE_PLUGINS_DIR`, defaulting to `plugins/` beside the package, per invocation
— the catalog is already re-read every 60 s. For each folder: read `plugin.json`, parse
it, read `index.js`, check its size, and **verify the source parses and exports both
functions** by running `plan` once in the sandbox with an empty context. Any failure
skips that folder with one line on stderr and never affects another. A duplicate id is
skipped.

- [ ] **Step 4: Write the worked example**

`companion/faces/plugins/github-stats/plugin.json` and `index.js` — the GitHub stats card
from the spec: `plan` declares one request to `api.github.com`, `render` returns a layout
with a headline number and two tiles. This is a real plugin, and the discovery test
fixture above is deliberately separate from it so the example can change freely.

- [ ] **Step 5: Merge into the registry**

```ts
// companion/faces/src/registry.ts
export async function allFaces(): Promise<readonly FaceDefinition[]> {
  const { faces } = await discoverPlugins();
  const builtIn = new Set(FACES.map((face) => face.kind));
  return [...FACES, ...faces.filter((face) => !builtIn.has(face.kind))];
}
```

A built-in face always wins a name collision, so a plugin folder cannot shadow `weather`.
`main.ts`'s `describe` and `render` call `allFaces()` and keep every other behaviour,
including C1's `{png, state}` envelope and the `tap` field.

- [ ] **Step 6: Run the whole suite and the gates**

Run: `cd companion/faces && bun test && bun run check && bun run lint && bun run format:check`
Expected: PASS — including the 32 golden cases, which must not move: this task changes
nothing about what the four built-in faces draw.

- [ ] **Step 7: Commit**

```bash
git add companion/faces/src/plugins/discovery.ts companion/faces/src/registry.ts companion/faces/src/main.ts companion/faces/plugins companion/faces/test/plugins/discovery.test.ts
git commit -m "feat(plugins): discovery with per-folder isolation, and a plugin is just a face"
```

---

### Task 10: Cadence from the catalog

**Files:**
- Modify: `companion/crates/server/src/data_cards/faces_package.rs:83-87`
- Modify: `companion/crates/server/src/data_cards.rs` (spec creation)
- Test: in the same files, following their existing `#[cfg(test)]` layout

**Interfaces:**
- Consumes: the `describe` catalog.
- Produces: `CatalogFace.refresh_seconds: Option<u64>`, used only when a spec is created.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_new_spec_takes_the_catalogs_cadence_clamped() {
    assert_eq!(cadence_for_new_spec(Some(300)), 300);
    assert_eq!(cadence_for_new_spec(Some(10)), 60);
    assert_eq!(cadence_for_new_spec(Some(1_000_000)), 86_400);
    assert_eq!(cadence_for_new_spec(None), 900);
}

#[test]
fn an_existing_specs_cadence_survives_a_catalog_reload() {
    // An owner's hand-set 300 is not rewritten when the plugin declares 900.
}

#[test]
fn an_unknown_catalog_key_is_ignored_rather_than_refusing_the_catalog() {
    let json = r#"[{"kind":"p","label":"P","fields":[],"refresh_seconds":300,"future":1}]"#;
    let faces: Vec<CatalogFace> = serde_json::from_str(json).expect("an older server ignores what it does not know");
    assert_eq!(faces[0].refresh_seconds, Some(300));
}
```

- [ ] **Step 2: Run them and watch them fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd companion && cargo test -p server data_cards > /tmp/t.log 2>&1; echo $?; tail -30 /tmp/t.log
```
Expected: FAIL — `refresh_seconds` and `cadence_for_new_spec` do not exist.

- [ ] **Step 3: Implement**

Add `#[serde(default)] pub(crate) refresh_seconds: Option<u64>` to `CatalogFace`; add
`fn cadence_for_new_spec(declared: Option<u64>) -> u64` clamping to 60…86_400 with a 900
fallback; call it where a spec is created from the browser. Do not touch the path that
loads an existing spec.

- [ ] **Step 4: Run the Rust gates**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd companion && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings > /tmp/c.log 2>&1; echo $?
cargo test --workspace --all-targets > /tmp/t.log 2>&1; echo $?; tail -5 /tmp/t.log
cargo test --workspace --doc > /tmp/d.log 2>&1; echo $?
```
Expected: all zero.

- [ ] **Step 5: Commit**

```bash
git add companion/crates/server/src/data_cards.rs companion/crates/server/src/data_cards/faces_package.rs
git commit -m "feat(server): a plugin's declared cadence seeds a new spec, clamped"
```

---

### Task 11: The author's documentation, and the repo's own pointers

**Files:**
- Create: `docs/plugins/contract-v1.md`
- Modify: `docs/images/server-rendered-cards.md`, `CLAUDE.md`

- [ ] **Step 1: Write `docs/plugins/contract-v1.md`**

For an author, in this order: what a plugin is; the two functions with the worked
`github-stats` example in full; the manifest, field by field; the three outputs; what the
host hands you (`settings`, `answers`, `now`, `format`, `state`, `event`); every limit
from the Global Constraints table with its number; the two error kinds and which one
reaches the owner; how to test a plugin locally; and the rule that a plugin using a
secret may declare only that secret's hosts.

- [ ] **Step 2: Point the operator document at it**

`docs/images/server-rendered-cards.md` keeps `data-cards.json`, the manual settings and
the operational material, and gains a line pointing authors at the contract.

- [ ] **Step 3: Update `CLAUDE.md`**

Its faces paragraph says adding a face is a file in `faces/src/faces/` plus
`deploy.sh --faces-only`. That stays true for our four; add the plugin path beside it,
and note that plugins are sandboxed and declare their hosts.

- [ ] **Step 4: Commit**

```bash
git add docs/plugins/contract-v1.md docs/images/server-rendered-cards.md CLAUDE.md
git commit -m "docs: the plugin contract, written for an author"
```

---

### Task 12: Deploy and see it on the panel

**Files:** none — this is the verification the repo's own rules demand.

- [ ] **Step 1: Run every gate**

```bash
cd companion/faces && bun test && bun run check && bun run lint && bun run format:check
cd .. && export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings > /tmp/c.log 2>&1; echo $?
cargo test --workspace --all-targets > /tmp/t.log 2>&1; echo $?
cargo test --workspace --doc > /tmp/d.log 2>&1; echo $?
cd apps/deskmate && bun test && bun run check && bun run format:check && bun run build
```

- [ ] **Step 2: Look at the example plugin's card**

```bash
cd companion/faces && bun run dump out/
```
Open `out/` and look at the `.desk.png` files — 0.4x is roughly the panel's physical size,
and type that reads on a laptop can be illegible from a chair.

- [ ] **Step 3: Deploy**

```bash
companion/crates/server/deploy/deploy.sh
```
The faces package ships as a directory and the Rust change from Task 10 needs the binary,
so this is a full deploy, not `--faces-only`.

- [ ] **Step 4: Drive the real window in Chrome**

Add the `github-stats` plugin as a card, fill its settings, save, and confirm the picture
appears and the settings window shows an error sentence when the username is wrong. The
mock harness cannot see any of this (`CLAUDE.md`).

- [ ] **Step 5: Put the frame on the panel**

Power on `dev-0005`, let the card reach it, and look at it at both mountings. **A frame in
the server's store is not the panel** — `CLAUDE.md` records this exact mistake. Record
what was observed, in those words, in `docs/hardware/board-notes.md`.

- [ ] **Step 6: Commit the board note and open the PR**

```bash
git add docs/hardware/board-notes.md
git commit -m "docs: what the plugin runtime proved on dev-0005, and what it did not"
gh pr create --title "Track B: the hosted plugin runtime" --body "..."
```

---

## What this plan does not build

Each needs its own plan, and none blocks the above:

- **Remote plugins** (`remote.json`, the POST adapter, its status handling) — the spec's
  second tier.
- **The self-host runner** (`src/runner.ts`).
- **Submission, review and the directory**, which wait on there being an outside author.
- **Sharing identical renders**, which is an optimisation and needs a cache key design.
- **Migrating the four built-in faces** onto the plugin format. The spec says they stay,
  and their goldens are the reason.
