import { afterEach, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkPlugin } from "../../src/author/check";
import { newPlugin } from "../../src/author/new";
import { SANDBOX_LIMITS } from "../../src/plugins/sandbox";

const roots: string[] = [];
afterEach(async () => {
  for (const root of roots.splice(0)) await rm(root, { recursive: true, force: true });
});
async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "plugin-author-"));
  roots.push(root);
  const pluginsDir = join(root, "plugins");
  const directory = await newPlugin("sample", pluginsDir);
  const options = { pluginsDir, outDir: join(root, "out") };
  return { directory, options, check: () => checkPlugin("sample", options) };
}
const card = `{ svg: '<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><rect width="448" height="368" fill="black"/></svg>' }`;
const errors = (report: Awaited<ReturnType<typeof checkPlugin>>) =>
  [...report.errors, ...report.cases.flatMap((example) => example.errors)].join(" ");

test("scaffold renders both sizes and its generated test runs", async () => {
  const f = await fixture();
  const report = await f.check();
  expect(report.ok, errors(report)).toBe(true);
  const result = report.cases[0];
  if (!result?.png || !result.desk) throw new Error(errors(report));
  for (const [file, width, height] of [
    [result.png, 448, 368],
    [result.desk, 179, 147],
  ] as const) {
    const png = await readFile(join(f.options.outDir, "sample", file));
    expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toEqual([width, height]);
  }
  const code = (await readFile(join(f.directory, "index.test.js"), "utf8")).replaceAll(
    "../../src/",
    join(import.meta.dir, "../../src/"),
  );
  await writeFile(join(f.directory, "index.test.js"), code);
  const child = Bun.spawn([process.execPath, "test", join(f.directory, "index.test.js")], {
    stdout: "pipe",
    stderr: "pipe",
  });
  const output = await new Response(child.stderr).text();
  expect(await child.exited, output).toBe(0);
});

test("CLI paths, built-in collisions and existing author files are refused", async () => {
  const f = await fixture();
  for (const id of ["../escape", "Hello World", "weather"]) {
    await expect(newPlugin(id, f.options.pluginsDir)).rejects.toThrow();
    await expect(checkPlugin(id, f.options)).rejects.toThrow();
  }
  const original = await readFile(join(f.directory, "index.js"), "utf8");
  await expect(newPlugin("sample", f.options.pluginsDir)).rejects.toThrow();
  expect(await readFile(join(f.directory, "index.js"), "utf8")).toBe(original);
});

test("manifest and discovery failures use the real contract diagnostics", async () => {
  const f = await fixture();
  const path = join(f.directory, "plugin.json");
  const manifest = JSON.parse(await readFile(path, "utf8"));
  await writeFile(path, JSON.stringify({ ...manifest, api: 9 }));
  expect(errors(await f.check())).toContain("unsupported api 9");
  await writeFile(path, JSON.stringify(manifest));
  await writeFile(
    join(f.directory, "index.js"),
    `export function plan(c){ return c.settings.missing; }\nexport function render(){ return ${card}; }`,
  );
  expect(errors(await f.check())).toContain("failed to load");
  await writeFile(join(f.directory, "index.js"), " ".repeat(SANDBOX_LIMITS.sourceBytes + 1));
  expect(errors(await f.check())).toContain("byte cap");
});

test("offline previews use recorded replies and retain sandbox isolation", async () => {
  const f = await fixture();
  const path = join(f.directory, "plugin.json");
  const manifest = JSON.parse(await readFile(path, "utf8"));
  await writeFile(path, JSON.stringify({ ...manifest, hosts: ["api.example.com"] }));
  await writeFile(
    join(f.directory, "index.js"),
    `export function plan(c){ return c.answers?.length ? [] : [{url: "https://api.example.com/data", as: "json"}]; }
export function render(c){
  if (typeof fetch !== "undefined" || typeof process !== "undefined" || typeof Intl !== "undefined") throw new Error("sandbox escape");
  if (c.answers[0]?.json?.value !== c.settings.expected) throw new Error("wrong offline answer");
  return ${card};
}`,
  );
  await writeFile(
    join(f.directory, "check.json"),
    JSON.stringify({
      cases: [
        {
          name: "recorded",
          settings: { expected: 42 },
          responses: [{ url: "https://api.example.com/data", status: 200, body: '{"value":42}' }],
        },
        { name: "absent" },
      ],
    }),
  );
  const report = await f.check();
  expect(report.ok, errors(report)).toBe(true);
  expect(report.cases[1]?.notices.join(" ")).toContain("Network is disabled");
  expect(report.cases[0]?.notices).toEqual([]);
});

test("every case runs after failure and runtime soft limits are visible", async () => {
  const f = await fixture();
  await writeFile(
    join(f.directory, "index.js"),
    `export function plan(){ return []; }
export function render(c){
  if (c.settings.fail) return { svg: "x".repeat(512 * 1024 + 1) };
  return { ...${card}, state: "x".repeat(17000), log: Array(11).fill("x".repeat(201)) };
}`,
  );
  await writeFile(
    join(f.directory, "check.json"),
    JSON.stringify({ cases: [{ settings: { fail: true } }, {}] }),
  );
  const report = await f.check();
  expect(report.ok).toBe(false);
  expect(errors(report)).toContain("SVG");
  expect(report.cases[1]?.png).toBeDefined();
  const notices = report.cases[1]?.notices.join(" ");
  expect(notices).toContain("16 KB");
  expect(notices).toContain("10 lines");
  expect(notices).toContain("200 characters");
});

test("invalid fixture shape and dates fail with useful messages", async () => {
  const f = await fixture();
  await writeFile(join(f.directory, "check.json"), '{"cases":[]}');
  expect(errors(await f.check())).toContain("non-empty cases array");
  await writeFile(join(f.directory, "check.json"), '{"cases":[null,{"now":"invalid"}]}');
  const report = await f.check();
  expect(errors(report)).toContain("Each check case");
  expect(errors(report)).toContain("valid ISO date/time");
});

test("recorded HTTP bodies use the real cap and report a refused response", async () => {
  const f = await fixture();
  const path = join(f.directory, "plugin.json");
  const manifest = JSON.parse(await readFile(path, "utf8"));
  await writeFile(path, JSON.stringify({ ...manifest, hosts: ["api.example.com"] }));
  await writeFile(
    join(f.directory, "index.js"),
    `export function plan(c){ return c.answers?.length ? [] : [{url: "https://api.example.com/data", as: "text"}]; }
export function render(c){ if (c.answers[0]?.ok) throw new Error("Oversize response escaped the host cap"); return ${card}; }`,
  );
  await writeFile(
    join(f.directory, "check.json"),
    JSON.stringify({
      cases: [
        {
          responses: [
            { url: "https://api.example.com/data", status: 200, body: "x".repeat(1024 * 1024 + 1) },
          ],
        },
      ],
    }),
  );
  const report = await f.check();
  expect(report.ok, errors(report)).toBe(true);
  expect(report.cases[0]?.notices.join(" ")).toContain("too large");
});

test("an ambient secrets file is never read by plugin:check", async () => {
  const f = await fixture();
  const path = join(f.directory, "plugin.json");
  const manifest = JSON.parse(await readFile(path, "utf8"));
  await writeFile(
    path,
    JSON.stringify({
      ...manifest,
      hosts: ["api.example.com"],
      secrets: [
        {
          key: "token",
          label: "Token",
          kind: "api_key",
          host: "api.example.com",
          send_as: "query",
          param: "key",
        },
      ],
    }),
  );
  const url = "https://api.example.com/data?key={{secret:token}}";
  await writeFile(
    join(f.directory, "index.js"),
    `export function plan(c){ return c.answers?.length ? [] : [{url: ${JSON.stringify(url)}, as: "text"}]; }
export function render(c){ if (c.answers[0]?.text !== "placeholder stayed") throw new Error("Unexpected credential substitution"); return ${card}; }`,
  );
  await writeFile(
    join(f.directory, "check.json"),
    JSON.stringify({
      cases: [
        {
          responses: [
            {
              url: `${new URL(url).origin}/data?${new URLSearchParams(new URL(url).search).toString()}`,
              status: 200,
              body: "placeholder stayed",
            },
          ],
        },
      ],
    }),
  );
  await writeFile(
    join(f.directory, "plugin-secrets.json"),
    '{"sample":{"token":"synthetic-test-value"}}',
  );
  const previous = process.env.DESKMATE_CONFIG_DIR;
  process.env.DESKMATE_CONFIG_DIR = f.directory;
  try {
    const report = await f.check();
    expect(report.ok, errors(report)).toBe(true);
    expect(JSON.stringify(report)).not.toContain("synthetic-test-value");
  } finally {
    if (previous === undefined) delete process.env.DESKMATE_CONFIG_DIR;
    else process.env.DESKMATE_CONFIG_DIR = previous;
  }
});

test("planning limit notice survives a later rendering failure", async () => {
  const f = await fixture();
  await writeFile(
    join(f.directory, "index.js"),
    `export function plan(){ return [{measure: [{text: "a", size: 12, weight: 400}]}]; }
export function render(){ throw new Error("broken drawing"); }`,
  );
  const report = await f.check();
  expect(errors(report)).toContain("broken drawing");
  expect(report.cases[0]?.notices.join(" ")).toContain("3-round limit");
});

test("both CLI commands reject missing or extra arguments", async () => {
  for (const [script, args] of [
    ["check", []],
    ["check", ["sample", "--bad", "path"]],
    ["check", ["sample", "--out"]],
    ["check", ["sample", "--out", "path", "extra"]],
    ["new", []],
    ["new", ["sample", "extra"]],
  ] as const) {
    const child = Bun.spawn(
      [process.execPath, join(import.meta.dir, `../../src/author/${script}.ts`), ...args],
      { stdout: "pipe", stderr: "pipe" },
    );
    const output = await new Response(child.stderr).text();
    expect(await child.exited).toBe(1);
    expect(output).toContain("Usage:");
  }
});
