import { afterEach } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseManifest } from "../../src/plugins/manifest";

/** Raw input, deliberately not parsed: callers can override it with malformed values. */
export function minimalManifest(id: string, extra: Record<string, unknown> = {}) {
  return {
    api: 1,
    id,
    version: "1.0.0",
    label: id,
    description: "d",
    author: "a",
    hosts: [],
    secrets: [],
    fields: [],
    ...extra,
  };
}

export function writePluginFolder(
  root: string,
  folder: string,
  manifest: unknown,
  source: string,
): void {
  const directory = join(root, folder);
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, "plugin.json"), JSON.stringify(manifest));
  writeFileSync(join(directory, "index.js"), source);
}

/** Register once per test file, so every importing suite owns its cleanup hook. */
export function tempPlugins() {
  const roots: string[] = [];
  afterEach(() => {
    for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  });
  return (prefix = "plugins-"): string => {
    const root = mkdtempSync(join(tmpdir(), prefix));
    roots.push(root);
    return root;
  };
}

export function shippedPlugin(id: string) {
  const folder = join(import.meta.dir, "../../plugins", id);
  return {
    folder,
    source: readFileSync(join(folder, "index.js"), "utf8"),
    manifest: parseManifest(JSON.parse(readFileSync(join(folder, "plugin.json"), "utf8")), id),
  };
}

export const ONE_BOX_CARD = `{ layout: { type: "div", style: { display: "flex", width: 448, height: 368, background: "#000" }, children: "ok" } }`;
export const SQUARE_SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><rect width="448" height="368" fill="black"/></svg>';
