// Turns a folder under the plugins directory into a `FaceDefinition`, in total
// isolation: one plugin's malformed manifest, oversized source, or syntax error
// skips only that folder and must never affect another, the built-in faces, or the
// catalog as a whole. That isolation is the point of this module -- `load_catalog`
// on the Rust side treats a failed `describe` as an EMPTY catalog, so one bad plugin
// silently emptying the add menu would be strictly worse than skipping it.
//
// The directory is read fresh on every call, the way the catalog itself is re-read
// every 60 s (`docs/images/server-rendered-cards.md`): a plugin folder can be added,
// fixed or removed without restarting anything.

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import type {
  FaceDefinition,
  RenderContext as FaceRenderContext,
  Settings,
  TapEvent,
} from "../face";
import { type RequestFn, request as httpRequest } from "../kit/http";
import { FORMAT_SOURCE } from "./context";
import { type PluginManifest, parseManifest } from "./manifest";
import { runPlugin } from "./run";
import { readPluginSecrets } from "./secrets";
import { SANDBOX_LIMITS, SandboxError, runInSandbox } from "./sandbox";
import { assertNotWithdrawn, PluginWithdrawnError, readDenylist } from "./denylist";

const MANIFEST_FILE = "plugin.json";
const SOURCE_FILE = "index.js";

export interface DiscoveredSkip {
  folder: string;
  reason: string;
  withdrawn?: true;
}

export interface DiscoverPluginsResult {
  faces: FaceDefinition[];
  skipped: DiscoveredSkip[];
}

function defaultPluginsDir(): string {
  const configured = process.env.DESKMATE_PLUGINS_DIR;
  if (configured !== undefined && configured.trim() !== "") return configured;
  // Beside the package: this file is `src/plugins/discovery.ts`, so two levels up is
  // the package root (`companion/faces/`), and `plugins/` sits next to `src/`.
  return join(import.meta.dir, "..", "..", "plugins");
}

/** The host process's own zone, falling back to UTC -- never thrown on a bad or
 * absent Intl result, because a plugin missing its clock entirely is worse than one
 * that thinks it is in UTC. */
function hostTimezone(): string {
  try {
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    return zone !== undefined && zone !== "" ? zone : "UTC";
  } catch {
    return "UTC";
  }
}

function reasonFor(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

/**
 * Discovery's own deadline for the one-time verify `plan()` call below -- deliberately
 * far short of `SANDBOX_LIMITS.deadlineMs` (the render path's 2000 ms), and NOT to be
 * unified with it. An empty context carries no data, so a well-behaved plugin's `plan`
 * returns immediately; a hostile or broken one that loops or blocks is exactly what
 * this call exists to catch, and catching it slowly is still catching it slowly. The
 * server re-reads the catalog every 60 s (`docs/images/server-rendered-cards.md`), so
 * this deadline is paid once PER BROKEN FOLDER, every minute, for as long as that
 * folder sits there -- ten broken folders at the render deadline would be 20 s of that
 * minute gone to folders nothing will ever use. A real render, by contrast, has actual
 * work to do (a fetch can legitimately take most of a second), which is why that path
 * keeps the full 2000 ms unchanged.
 */
const DISCOVERY_PROBE_DEADLINE_MS = 200;

/** Runs `plan()` once against an empty context, the way discovery does, to confirm
 * the source at least parses and exports both functions. Reused for the one-time
 * verify at discovery and nowhere else -- a real render always goes through
 * `runPlugin`, never this, and never with this short a deadline. */
function verifyLoads(source: string): void {
  runInSandbox(
    `${FORMAT_SOURCE}\n${source}`,
    "plan",
    {},
    { deadlineMs: DISCOVERY_PROBE_DEADLINE_MS },
  );
}

interface LoadedPlugin {
  manifest: PluginManifest;
  source: string;
}

function loadFolder(
  directory: string,
  folder: string,
  policy: ReturnType<typeof readDenylist>,
): LoadedPlugin {
  const manifestPath = join(directory, folder, MANIFEST_FILE);
  const sourcePath = join(directory, folder, SOURCE_FILE);

  let rawManifest: string;
  try {
    rawManifest = readFileSync(manifestPath, "utf8");
  } catch {
    throw new Error(`no ${MANIFEST_FILE} in ${folder}`);
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(rawManifest);
  } catch (error) {
    throw new Error(`${MANIFEST_FILE} is not valid JSON: ${reasonFor(error)}`);
  }
  const manifest = parseManifest(parsed, folder);
  assertNotWithdrawn(policy, manifest.id, manifest.version);

  let source: string;
  try {
    source = readFileSync(sourcePath, "utf8");
  } catch {
    throw new Error(`no ${SOURCE_FILE} in ${folder}`);
  }
  const sourceBytes = Buffer.byteLength(source, "utf8");
  if (sourceBytes > SANDBOX_LIMITS.sourceBytes) {
    throw new Error(
      `${SOURCE_FILE} is ${sourceBytes} bytes, over the ${SANDBOX_LIMITS.sourceBytes} byte cap`,
    );
  }

  verifyLoads(source);

  return { manifest, source };
}

/** Host-only dependencies for offline author checks. Never exposed to plugin code. */
export interface PluginHost {
  request?: RequestFn;
  readSecrets?: typeof readPluginSecrets;
  onLog?: (line: string) => void;
  /** Explicit instance policy path; null isolates author previews from the operator. */
  denylistPath?: string | null;
}

function toFaceDefinition(
  manifest: PluginManifest,
  source: string,
  host: PluginHost,
): FaceDefinition {
  return {
    kind: manifest.id,
    label: manifest.label,
    fields: manifest.fields,
    ...(manifest.tap === undefined ? {} : { tap: manifest.tap }),
    ...(manifest.refreshSeconds === undefined ? {} : { refreshSeconds: manifest.refreshSeconds }),
    async render(settings: Settings, now: Date, context: FaceRenderContext = {}) {
      // A retained FaceDefinition must observe removal after discovery too.
      assertNotWithdrawn(readDenylist(host.denylistPath), manifest.id, manifest.version);
      const secrets = await (host.readSecrets ?? readPluginSecrets)(manifest.id);
      const timezone =
        context.timezone !== undefined && context.timezone.trim() !== ""
          ? context.timezone
          : hostTimezone();
      const event: TapEvent | undefined = context.event;
      const emitted = new Set<string>();
      const result = await runPlugin({
        manifest,
        source,
        settings,
        now,
        timezone,
        ...(context.state === undefined ? {} : { state: context.state }),
        ...(event === undefined ? {} : { event }),
        secrets,
        request: host.request ?? httpRequest,
        onNotice: (line) => {
          emitted.add(line);
          host.onLog?.(line);
        },
      });
      for (const line of result.log) {
        if (!emitted.has(line)) host.onLog?.(line);
      }
      return { svg: result.svg, ...(result.state === undefined ? {} : { state: result.state }) };
    },
  };
}

/**
 * Lists `directory` (`DESKMATE_PLUGINS_DIR`, defaulting to `plugins/` beside this
 * package), and turns every folder that survives four checks -- a parseable
 * `plugin.json` whose id matches the folder, a readable `index.js` under the source
 * cap, and a source that loads (parses and exports `plan`/`render`, verified by
 * running `plan` once against an empty context) -- into a `FaceDefinition`. Anything
 * else about a folder is a one-line skip on stderr, and never touches another
 * folder's result. A directory that does not exist at all is zero plugins, not an
 * error: the plugins feature is opt-in.
 */
export async function discoverPlugins(
  directory?: string,
  host: PluginHost = {},
  folders?: readonly string[],
): Promise<DiscoverPluginsResult> {
  const root = directory ?? defaultPluginsDir();
  const policy = readDenylist(host.denylistPath);

  let entries: string[];
  try {
    entries = readdirSync(root, { withFileTypes: true })
      .filter(
        (entry) => entry.isDirectory() && (folders === undefined || folders.includes(entry.name)),
      )
      .map((entry) => entry.name)
      .sort();
  } catch {
    return { faces: [], skipped: [] };
  }

  const faces: FaceDefinition[] = [];
  const skipped: DiscoveredSkip[] = [];
  const seenIds = new Set<string>();

  for (const folder of entries) {
    try {
      // An unreadable list or id-wide withdrawal prevents even the discovery probe.
      assertNotWithdrawn(policy, folder);
      const { manifest, source } = loadFolder(root, folder, policy);
      if (seenIds.has(manifest.id)) {
        throw new Error(`duplicate id ${JSON.stringify(manifest.id)}`);
      }
      seenIds.add(manifest.id);
      faces.push(toFaceDefinition(manifest, source, host));
    } catch (error) {
      const reason =
        error instanceof SandboxError ? `failed to load: ${error.message}` : reasonFor(error);
      skipped.push({
        folder,
        reason,
        ...(error instanceof PluginWithdrawnError ? { withdrawn: true as const } : {}),
      });
      process.stderr.write(
        `plugin ${folder} skipped: ${reason.replace(/\s+/g, " ").slice(0, 300)}\n`,
      );
    }
  }

  return { faces, skipped };
}
