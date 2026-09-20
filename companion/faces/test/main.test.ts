import { expect, test } from "bun:test";

const MAIN = `${import.meta.dir}/../src/main.ts`;

async function run(
  verb: string,
  stdin = "",
): Promise<{ code: number; out: Uint8Array; err: string }> {
  const child = Bun.spawn(["bun", "run", MAIN, verb], {
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  child.stdin.write(stdin);
  await child.stdin.end();
  const [out, err, code] = await Promise.all([
    new Response(child.stdout).bytes(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  return { code, out, err };
}

test("describe prints the catalog the server builds the add menu from", async () => {
  const { code, out } = await run("describe");
  expect(code).toBe(0);
  const catalog = JSON.parse(new TextDecoder().decode(out)) as {
    kind: string;
    fields: { type: string; key: string }[];
  }[];
  expect(catalog.map((face) => face.kind)).toEqual(["weather", "hackernews", "rss", "token"]);
  for (const face of catalog) {
    for (const field of face.fields) {
      expect(["text", "url", "enum"]).toContain(field.type);
    }
  }
  // A secret has no place in a response the browser can read.
  expect(JSON.stringify(catalog)).not.toContain("api_key");
});

test("an unknown kind and an incomplete setting are configuration errors: exit 2, nothing on stdout", async () => {
  for (const request of [
    '{"kind":"horoscope","settings":{}}',
    '{"kind":"weather","settings":{"location":"  "}}',
  ]) {
    const { code, out, err } = await run("render", request);
    expect(code).toBe(2);
    expect(out.length).toBe(0);
    expect(err.trim().split("\n")).toHaveLength(1);
  }
});

test("a request that is not JSON is not the owner's fault: exit 1", async () => {
  const { code, out } = await run("render", "not json");
  expect(code).toBe(1);
  expect(out.length).toBe(0);
});

test("an unknown verb prints usage and draws nothing", async () => {
  const { code, out } = await run("paint");
  expect(code).toBe(64);
  expect(out.length).toBe(0);
});
