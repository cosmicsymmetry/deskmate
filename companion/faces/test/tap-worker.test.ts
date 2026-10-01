import { expect, test } from "bun:test";
import { MAX_REQUEST_BYTES, MAX_REQUESTS, MAX_RESPONSE_BYTES } from "../src/tap-worker";

const MAIN = `${import.meta.dir}/../src/main.ts`;
const WORKER = `${import.meta.dir}/../src/tap-worker.ts`;
const tap = (state?: unknown) =>
  JSON.stringify({ kind: "weather", settings: {}, state, event: { taps: 1, point: null } });

async function run(input: string, code?: string) {
  const child = Bun.spawn(
    code === undefined ? ["bun", "run", MAIN, "tap-worker"] : ["bun", "-e", code],
    { stdin: "pipe", stdout: "pipe", stderr: "pipe" },
  );
  child.stdin.write(input);
  await child.stdin.end();
  const [output, error, status] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  return { output, error, status };
}

test("one selector handles independent states and preserves the error taxonomy", async () => {
  const result = await run(
    [tap(), tap({ view: "days" }), "{", '{"kind":"absent"}', tap()].join("\n") + "\n",
  );
  expect(result.status).toBe(0);
  const rows = result.output
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  expect(rows.map((row) => row.code)).toEqual([0, 0, 1, 2, 0]);
  expect(rows.filter((row) => row.code === 0).map((row) => row.result.view)).toEqual([
    "days",
    "",
    "days",
  ]);
});

test("the production request count retires a worker", async () => {
  const result = await run(`${tap()}\n`.repeat(MAX_REQUESTS + 1));
  expect(result.status).toBe(0);
  expect(result.output.trim().split("\n")).toHaveLength(MAX_REQUESTS);
});

test("memory pressure retires after the reply, before another request", async () => {
  const result = await run(
    "{}\n{}\n",
    `
    import { serveTapWorker } from ${JSON.stringify(WORKER)};
    await serveTapWorker(async () => '{"view":"ok"}', {requests: 256, rssBytes: 1, ageMs: 60000});
  `,
  );
  expect(result.status).toBe(0);
  expect(result.output.trim().split("\n")).toHaveLength(1);
});

test("an idle worker exits at its lifetime without needing stdin EOF", async () => {
  const child = Bun.spawn(
    [
      "bun",
      "-e",
      `
    import { serveTapWorker } from ${JSON.stringify(WORKER)};
    await serveTapWorker(async () => '{}', {requests: 256, rssBytes: 134217728, ageMs: 30});
  `,
    ],
    { stdin: "pipe", stdout: "pipe", stderr: "pipe" },
  );
  const timeout = setTimeout(() => child.kill(), 1000);
  try {
    expect(await child.exited).toBe(0);
  } finally {
    clearTimeout(timeout);
    child.kill();
  }
});

test("both framed and unterminated oversized requests are refused", async () => {
  for (const ending of ["", "\n"]) {
    const result = await run(" ".repeat(MAX_REQUEST_BYTES + 1) + ending);
    expect(result.status).not.toBe(0);
    expect(result.output).toBe("");
    expect(result.error).toContain("selector request too large");
  }
});

test("a reply is bounded before writing any bytes", async () => {
  const result = await run(
    "{}\n",
    `
    import { serveTapWorker } from ${JSON.stringify(WORKER)};
    await serveTapWorker(async () => JSON.stringify({view: 'x'.repeat(${MAX_RESPONSE_BYTES})}));
  `,
  );
  expect(result.status).not.toBe(0);
  expect(result.output).toBe("");
  expect(result.error).toContain("selector response too large");
});
