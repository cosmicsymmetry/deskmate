import { join } from "node:path";
import { faceOfKind } from "../registry";

export const PLUGINS_DIR = join(import.meta.dir, "../../plugins");
export const OUTPUT_DIR = join(import.meta.dir, "../../out/plugins");

// A CLI path component, not a replacement for the manifest contract validator.
export function checkId(id: string): void {
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(id)) {
    throw new Error("Use a plugin id of lowercase letters, digits and single hyphens.");
  }
  if (faceOfKind(id)) throw new Error(`${id} is already a built-in face; choose another id.`);
}

export function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
