import { afterAll } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Package tests have no operator instance. A build user may not even be able to
// traverse the live config directory. Policy tests supply their own explicit
// files; ordinary tests and their children start with a missing, readable path.
const root = mkdtempSync(join(tmpdir(), "faces-test-policy-"));
const previous = process.env.DESKMATE_PLUGIN_DENYLIST;
process.env.DESKMATE_PLUGIN_DENYLIST = join(root, "denylist.json");
afterAll(() => {
  if (previous === undefined) delete process.env.DESKMATE_PLUGIN_DENYLIST;
  else process.env.DESKMATE_PLUGIN_DENYLIST = previous;
  rmSync(root, { recursive: true, force: true });
});
