import { readFile, mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { pngFromSvg } from "../kit/raster";
import { type HttpRequest, type HttpReply, replyFrom } from "../kit/http";
import { renderRequest } from "../main";
import { discoverPlugins } from "../plugins/discovery";
import { parseManifest } from "../plugins/manifest";
import { SANDBOX_LIMITS, warmSandbox } from "../plugins/sandbox";
import { OUTPUT_DIR, PLUGINS_DIR, checkId, message } from "./common";
import { hashPlugin, readReleases, releaseStatus, type Release } from "../plugins/releases";

interface CheckCase {
  name?: string;
  now?: string;
  timezone?: string;
  settings?: Record<string, unknown>;
  state?: unknown;
  event?: unknown;
  responses?: { url: string; method?: string; status: number; body?: string; base64?: string }[];
}

export interface CheckReport {
  id: string;
  ok: boolean;
  errors: string[];
  release?: Release;
  review?: "reviewed" | "pending";
  cases: { name: string; png?: string; desk?: string; errors: string[]; notices: string[] }[];
}

/** The transport is always offline, even when the surrounding shell has credentials.
 * Recorded replies replace I/O only; plan validation, budgets, secret handling,
 * sandbox evaluation and rasterization still run through production code. */
async function offlineReply(
  input: HttpRequest,
  example: CheckCase,
  notices: string[],
): Promise<HttpReply> {
  const recorded = example.responses?.find(
    (reply) => reply.url === input.url && (reply.method ?? "GET") === (input.method ?? "GET"),
  );
  if (!recorded) {
    notices.push(
      `No recorded reply for ${input.method ?? "GET"} ${input.url}; exercising request failure. Network is disabled.`,
    );
    throw new Error("Offline preview: no recorded response; network is disabled");
  }
  const body =
    recorded.base64 === undefined ? recorded.body : Buffer.from(recorded.base64, "base64");
  // The production HTTP reply decoder applies its own body cap and JSON fallback.
  return replyFrom(new Response(body, { status: recorded.status }), input.as);
}

function readableFailure(error: unknown): string {
  const reason = message(error);
  if (/out of memory/i.test(reason))
    return `${reason}. Sandbox memory limit: ${SANDBOX_LIMITS.memoryBytes / 1024 / 1024} MB per call.`;
  if (/interrupted/i.test(reason))
    return `${reason}. Sandbox time limit: ${SANDBOX_LIMITS.deadlineMs} ms per render call (discovery uses a shorter probe).`;
  return reason;
}

export async function checkPlugin(
  id: string,
  options: { pluginsDir?: string; outDir?: string } = {},
): Promise<CheckReport> {
  checkId(id);
  const root = options.pluginsDir ?? PLUGINS_DIR;
  const output = join(options.outDir ?? OUTPUT_DIR, id);
  await mkdir(output, { recursive: true });
  const report: CheckReport = { id, ok: false, errors: [], cases: [] };
  try {
    // This is the runtime parser, not a tooling-owned manifest schema.
    const manifest = parseManifest(
      JSON.parse(await readFile(join(root, id, "plugin.json"), "utf8")),
      id,
    );
    report.release = hashPlugin(root, id);
    report.review = releaseStatus(report.release, readReleases(root, true));
    const defaults = Object.fromEntries(
      manifest.fields.map((field) => [field.key, field.default ?? ""]),
    );
    let cases: CheckCase[] = [{ name: "default" }];
    try {
      const raw = JSON.parse(await readFile(join(root, id, "check.json"), "utf8"));
      if (!Array.isArray(raw.cases) || raw.cases.length === 0)
        throw new Error("check.json needs a non-empty cases array");
      cases = raw.cases;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    await warmSandbox();
    for (const [index, example] of cases.entries()) {
      const result: CheckReport["cases"][number] = {
        name: example?.name ?? `case ${index + 1}`,
        errors: [],
        notices: [],
      };
      report.cases.push(result);
      try {
        if (!example || typeof example !== "object")
          throw new Error("Each check case must be an object");
        const now = new Date(example.now ?? "2026-10-01T12:00:00Z");
        if (!Number.isFinite(now.getTime()))
          throw new Error("Case now must be a valid ISO date/time");
        const { faces, skipped } = await discoverPlugins(
          root,
          {
            request: async (input) => offlineReply(input, example, result.notices),
            readSecrets: async () => ({}),
            denylistPath: null,
            onLog: (line) => result.notices.push(line),
          },
          [id],
        );
        const face = faces.find((entry) => entry.kind === id);
        if (!face)
          throw new Error(
            skipped.find((entry) => entry.folder === id)?.reason ??
              "Plugin folder was not discovered",
          );
        const rendered = await renderRequest(
          {
            kind: id,
            settings: { ...defaults, ...example.settings },
            timezone: example.timezone ?? "UTC",
            state: example.state,
            event: example.event,
          },
          face,
          now,
        );
        const full = Buffer.from(rendered.png, "base64");
        const desk = pngFromSvg(
          `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><image width="448" height="368" href="data:image/png;base64,${rendered.png}"/></svg>`,
          0.4,
        );
        // Case labels are prose, never paths (fixtures may be untrusted PR content).
        const stem = `${String(index + 1).padStart(2, "0")}-${index === 0 && cases.length === 1 ? "default" : "preview"}`;
        result.png = `${stem}.png`;
        result.desk = `${stem}.desk.png`;
        await writeFile(join(output, result.png), full);
        await writeFile(join(output, result.desk), desk);
      } catch (error) {
        result.errors.push(readableFailure(error));
      }
    }
  } catch (error) {
    report.errors.push(readableFailure(error));
  }
  report.ok =
    report.errors.length === 0 && report.cases.every((example) => example.errors.length === 0);
  await writeFile(join(output, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
  return report;
}

if (import.meta.main) {
  try {
    const [id, flag, out, ...extra] = process.argv.slice(2);
    if (!id || (flag !== undefined && (flag !== "--out" || !out)) || extra.length) {
      throw new Error("Usage: bun run plugin:check <id> [--out <directory>]");
    }
    const report = await checkPlugin(id, { outDir: out === undefined ? OUTPUT_DIR : resolve(out) });
    console.log(
      `${report.ok ? "PASS" : "FAIL"}: ${id} — real discovery, plan({}), sandbox and render; network disabled, no secrets.`,
    );
    for (const error of report.errors) console.error(`  Error: ${error}`);
    if (report.release) {
      console.log(`Release identity: ${JSON.stringify(report.release)}`);
      console.log(
        report.review === "reviewed"
          ? "Reviewed hash verified."
          : "Not yet in the release index; the owner adds approval after review.",
      );
    }
    for (const example of report.cases) {
      console.log(
        `  ${example.name}: ${example.errors.length ? "FAIL" : "448×368 and 0.4× PNGs rendered"}`,
      );
      for (const error of example.errors) console.error(`    Error: ${error}`);
      for (const notice of example.notices) console.log(`    Notice: ${notice}`);
    }
    console.log(
      `Output: ${join(out ?? OUTPUT_DIR, id)}\nOnly executed cases are checked; inspect both PNG sizes and add behavioral tests.`,
    );
    process.exitCode = report.ok ? 0 : 1;
  } catch (error) {
    console.error(message(error));
    process.exitCode = 1;
  }
}
