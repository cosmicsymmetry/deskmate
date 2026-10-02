import { readFileSync } from "node:fs";
import { join } from "node:path";
import { ConfigurationError } from "../face";

interface Denial {
  id: string;
  version?: string;
  reason: string;
}
interface Denylist {
  entries: Denial[];
  unreadable?: true;
}

export class PluginWithdrawnError extends ConfigurationError {}

export function readDenylist(path?: string | null): Denylist {
  // Author previews explicitly opt out of instance policy, like stored secrets.
  if (path === null) return { entries: [] };
  const file =
    path ??
    process.env.DESKMATE_PLUGIN_DENYLIST ??
    join(process.env.DESKMATE_CONFIG_DIR ?? "/var/lib/deskmate/configs", "plugin-denylist.json");
  try {
    const text = readFileSync(file, "utf8");
    if (Buffer.byteLength(text) > 256 * 1024) throw new Error("file exceeds 256 KiB");
    const raw: unknown = JSON.parse(text);
    if (!Array.isArray(raw))
      throw new Error("expected an array of id, optional version, and reason");
    const entries = raw.map((entry: unknown): Denial => {
      if (typeof entry !== "object" || entry === null) throw new Error("invalid entry");
      const e = entry as Record<string, unknown>;
      if (
        typeof e.id !== "string" ||
        !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(e.id) ||
        typeof e.reason !== "string" ||
        !e.reason.trim() ||
        (e.version !== undefined &&
          (typeof e.version !== "string" || !e.version.trim() || e.version !== e.version.trim())) ||
        Object.keys(e).some((key) => !["id", "version", "reason"].includes(key))
      )
        throw new Error("invalid id, version or reason");
      return {
        id: e.id,
        reason: e.reason.replace(/\s+/g, " ").trim().slice(0, 140),
        ...(typeof e.version === "string" ? { version: e.version } : {}),
      };
    });
    return { entries };
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return { entries: [] };
    console.error(
      `Plugin denylist is unreadable: ${String(error).replace(/\s+/g, " ").slice(0, 300)}`,
    );
    return { entries: [], unreadable: true };
  }
}

export function assertNotWithdrawn(policy: Denylist, id: string, version?: string): void {
  if (policy.unreadable)
    throw new PluginWithdrawnError("withdrawn by the operator: denylist is unreadable");
  const entry = policy.entries.find(
    (e) => e.id === id && (e.version === undefined || e.version === version),
  );
  if (entry) throw new PluginWithdrawnError(`withdrawn by the operator: ${entry.reason}`);
}
