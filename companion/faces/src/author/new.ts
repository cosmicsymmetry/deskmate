import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { parseManifest } from "../plugins/manifest";
import { PLUGINS_DIR, checkId, message } from "./common";

export async function newPlugin(id: string, root = PLUGINS_DIR): Promise<string> {
  checkId(id);
  const label = id
    .split("-")
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
  const manifest = {
    api: 1,
    id,
    version: "0.1.0",
    label,
    description: "A calendar date for your desk. Replace this with your plugin's purpose.",
    author: "Plugin contributors",
    hosts: [],
    secrets: [],
    fields: [],
    refreshSeconds: 900,
  };
  parseManifest(manifest, id);
  const directory = join(root, id);
  await mkdir(root, { recursive: true });
  // Atomic refusal on an existing directory, including a symlink: never overwrite an author's work.
  await mkdir(directory);
  const source = `// No imports: the host supplies the clock and runs both functions in QuickJS.
export function plan() {
  return [];
}

export function render(context) {
  return {
    layout: {
      type: "div",
      style: {
        display: "flex", flexDirection: "column", alignItems: "center",
        justifyContent: "center", width: 448, height: 368,
        background: "#000000", color: "#f5f5f7", fontFamily: "Inter",
      },
      children: [
        { type: "div", style: { fontSize: 32, fontWeight: 600 }, children: "${label}" },
        { type: "div", style: { fontSize: 24, marginTop: 16 }, children: format.date(context.now.local, "d MMM yyyy") },
      ],
    },
  };
}
`;
  const test = `import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseManifest } from "../../src/plugins/manifest";
import { runPlugin } from "../../src/plugins/run";
import { runInSandbox, warmSandbox } from "../../src/plugins/sandbox";

test("${id} displays the owner's calendar date without networking", async () => {
  await warmSandbox();
  const source = readFileSync(join(import.meta.dir, "index.js"), "utf8");
  const manifest = parseManifest(JSON.parse(readFileSync(join(import.meta.dir, "plugin.json"), "utf8")), "${id}");
  expect(runInSandbox(source, "plan", {})).toEqual([]);
  const draw = (now) => runPlugin({ manifest, source, settings: {}, now: new Date(now), timezone: "Asia/Tokyo", secrets: {},
    request: async () => { throw new Error("No network expected"); },
  });
  const before = await draw("2027-12-31T14:59:59Z");
  const after = await draw("2027-12-31T15:00:00Z");
  expect(before.svg).not.toBe(after.svg);
  expect(after.state).toBeUndefined();
});
`;
  const readme = `# ${label}

## What this plugin does

Displays the calendar date in the owner's timezone. No settings yet.
Id: \`${id}\`; version: \`0.1.0\`. Replace the sample with your own purpose and tests.

## Reach and credentials

None. No network, secrets or taps. Requested refresh: 900 seconds.

## Preview and reproduction

From \`companion/faces\`: \`bun run plugin:check ${id}\`.
Inspect \`out/plugins/${id}/01-default.png\` and \`01-default.desk.png\`.
Optional \`check.json\` adds fixed dates, settings and recorded HTTP replies;
see [the submission guide](../../../../docs/plugins/submitting.md).

## Validation

Run \`bun test plugins/${id}\`, then the package tests, typecheck, lint and format gates.
Record results here and in the PR. CI attaches previews for changed plugins.
A local render is not a browser, server-delivery or physical-panel observation.

## Limitations and attribution

Uses the host-supplied owner-local date. Successful faces refresh within roughly
one minute of local midnight plus render/delivery time; failures retain the last
frame. Shared sources use the first consuming device's zone in device-id order.
No on-device ticking. See the contract for fallback and refresh behavior.

Original sample under GPL-3.0-only. Uses the host's bundled Inter (SIL OFL).
Replace author attribution in plugin.json and identify any new third-party assets.

## Reviewer record

Record the reviewed commit, findings and outcome in the PR. Merge and deployment
are separate actions.
`;
  for (const [name, contents] of Object.entries({
    "plugin.json": `${JSON.stringify(manifest, null, 2)}\n`,
    "index.js": source,
    "index.test.js": test,
    "README.md": readme,
  }))
    await writeFile(join(directory, name), contents, { flag: "wx" });
  return directory;
}

if (import.meta.main) {
  const [id, ...extra] = process.argv.slice(2);
  try {
    if (!id || extra.length) throw new Error("Usage: bun run plugin:new <id>");
    const directory = await newPlugin(id);
    console.log(`Created ${directory}\nNext: bun run plugin:check ${id}`);
  } catch (error) {
    console.error(message(error));
    process.exitCode = 1;
  }
}
