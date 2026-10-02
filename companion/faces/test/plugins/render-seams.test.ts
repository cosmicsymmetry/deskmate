import { afterEach, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { newPlugin } from "../../src/author/new";

const roots: string[] = [];
const main = join(import.meta.dir, "../../src/main.ts");
const valid = { svg: '<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"/>' };
const event = { taps: 2, point: null };
afterEach(async () => {
  for (const root of roots.splice(0)) await rm(root, { recursive: true, force: true });
});

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "plugin-render-seams-"));
  roots.push(root);
  const plugins = join(root, "plugins");
  const directory = await newPlugin("sample", plugins);
  const manifestPath = join(directory, "plugin.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  await writeFile(manifestPath, JSON.stringify({ ...manifest, tap: "Draw the next page" }));
  await writeFile(
    join(directory, "index.js"),
    `export function plan(){ return []; }
export function render(c){
  return { ...c.settings.card, state: { timezone: c.now.timezone, taps: c.event?.taps ?? 0 } };
}`,
  );
  const env = {
    ...process.env,
    TZ: "UTC",
    DESKMATE_PLUGINS_DIR: plugins,
    DESKMATE_CONFIG_DIR: root,
  };
  const invoke = (verb: string, request: object) => {
    // These are complete fixture requests, not an interactive pipe test. Let
    // spawnSync own stdin/EOF and bound each child, including failure cleanup.
    const child = Bun.spawnSync([process.execPath, "run", main, verb], {
      env,
      stdin: Buffer.from(`${JSON.stringify(request)}\n`),
      stdout: "pipe",
      stderr: "pipe",
      timeout: 10_000,
    });
    const error = `${verb}: ${child.stderr.toString()}`;
    if (child.signalCode) throw new Error(`${error} (stopped by ${child.signalCode})`);
    return { output: child.stdout.toString(), error, status: child.exitCode };
  };
  const select = async (request: object) => {
    // The server keeps this process alive between requests. Observe its framed
    // reply with stdin still open; process exit/EOF is not the seam under test.
    const child = Bun.spawn([process.execPath, "run", main, "tap-worker"], {
      env,
      stdin: "pipe",
      stdout: "pipe",
      stderr: "pipe",
    });
    const errors = new Response(child.stderr).text();
    const reader = child.stdout.getReader();
    let deadline: ReturnType<typeof setTimeout> | undefined;
    let output: string;
    try {
      child.stdin.write(`${JSON.stringify(request)}\n`);
      await child.stdin.flush();
      const reply = async () => {
        let output = "";
        const decoder = new TextDecoder();
        while (true) {
          const { done, value } = await reader.read();
          if (done) throw new Error("selector closed stdout before replying");
          output += decoder.decode(value, { stream: true });
          const newline = output.indexOf("\n");
          if (newline !== -1) return output.slice(0, newline);
        }
      };
      output = await Promise.race([
        reply(),
        new Promise<never>((_, reject) => {
          deadline = setTimeout(() => reject(new Error("selector did not reply in 10s")), 10_000);
        }),
      ]);
    } finally {
      clearTimeout(deadline);
      child.kill();
      await child.exited;
      await reader.cancel();
      await errors;
    }
    const error = await errors;
    if (error) throw new Error(`tap-worker: ${error}`);
    return JSON.parse(output);
  };
  return { invoke, select };
}

test("the real warm selector defers plugin taps to a render carrying the owner's zone", async () => {
  const { invoke, select } = await fixture();
  const request = { kind: "sample", settings: { card: valid }, event };
  const reply = await select(request);
  expect(reply.code).toBe(1);
  expect(reply.error).toContain("this face handles taps through render");
  expect(reply.result).toBeUndefined();
  // Plugin v1 exposes neither views nor onTap. Discovery must not invent staged
  // plugin views, and the warm process must never run plugin render itself.
  const views = await invoke("views", request);
  expect(views.status, views.error).toBe(0);
  expect(JSON.parse(views.output)).toEqual({ views: [""] });
  for (const timezone of ["Asia/Tokyo", "America/Los_Angeles"]) {
    const rendered = await invoke("render", { ...request, timezone });
    expect(rendered.status, rendered.error).toBe(0);
    const envelope = JSON.parse(rendered.output);
    expect(envelope.state).toEqual({ timezone, taps: 2 });
    const png = Buffer.from(envelope.png, "base64");
    expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toEqual([448, 368]);
  }
}, 60_000);

for (const [name, context] of [
  ["tap fallback", { event }],
  ["staged-view envelope", { view: "page-1" }],
] as const) {
  test(`${name} shares timezone and external-resource validation in the live render process`, async () => {
    const { invoke } = await fixture();
    const request = { kind: "sample", timezone: "Asia/Tokyo", ...context };
    const accepted = await invoke("render", { ...request, settings: { card: valid } });
    expect(accepted.status, accepted.error).toBe(0);
    expect(JSON.parse(accepted.output).state).toEqual({
      timezone: "Asia/Tokyo",
      taps: "event" in context ? 2 : 0,
    });

    // A staged-view envelope uses the same render verb. Plugin v1 does not
    // advertise alternate views, but supplying view must never skip the guard.
    // Loopback keeps a regressed subprocess from contacting an external host.
    const url = "http://localhost:9/private.png";
    for (const card of [
      { layout: { style: { width: 448, height: 368, backgroundImage: `url(${url})` } } },
      { layout: { style: { width: 448, height: 368, maskImage: `url(${url})` } } },
      {
        svg: `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><image href="${url}"/></svg>`,
      },
    ]) {
      const refused = await invoke("render", { ...request, settings: { card } });
      expect(refused.status, refused.error).toBe(2);
      expect(refused.output).toBe("");
      expect(refused.error).toContain("external resources are not allowed");
    }
  }, 60_000);
}
