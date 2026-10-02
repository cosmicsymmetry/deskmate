// One content identity for author evidence, CI and installation. See
// docs/plugins/releases.md for the byte format and approval boundary.
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { parseManifest } from "./manifest";

export interface Release {
  id: string;
  version: string;
  sha256: string;
}

export const RELEASE_INDEX = "releases.json";
const ID = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

export function pluginFolders(root: string): string[] {
  if (!lstatSync(root).isDirectory()) throw new Error("Plugins directory must be a real directory");
  return readdirSync(root)
    .filter((name) => {
      const stat = lstatSync(join(root, name));
      if (stat.isSymbolicLink()) throw new Error(`Plugin path ${name} is a symlink`);
      if (!stat.isDirectory()) return false;
      if (!ID.test(name)) throw new Error(`Invalid plugin folder ${JSON.stringify(name)}`);
      return true;
    })
    .sort();
}

export function hashPlugin(root: string, id: string): Release {
  if (!ID.test(id)) throw new Error("Invalid plugin id");
  const directory = join(root, id);
  const files: string[] = [];
  const walk = (relative: string) => {
    const path = join(directory, relative);
    const stat = lstatSync(path);
    if (stat.isSymbolicLink()) throw new Error(`${id}: symlinks are not released (${relative})`);
    if (stat.isDirectory()) {
      for (const name of readdirSync(path)) walk(relative ? `${relative}/${name}` : name);
    } else if (stat.isFile()) {
      files.push(relative);
    } else {
      throw new Error(`${id}: only regular files are released (${relative})`);
    }
  };
  walk("");
  const manifest = parseManifest(
    JSON.parse(readFileSync(join(directory, "plugin.json"), "utf8")),
    id,
  );
  if (!files.includes("index.js"))
    throw new Error(`${id}@${manifest.version}: index.js is missing`);
  // Bytewise UTF-8 path order; no locale, timestamps, permissions or exclusions.
  files.sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
  const hash = createHash("sha256").update("deskmate-plugin-release-v1\0");
  for (const file of files) {
    const path = Buffer.from(file, "utf8");
    const bytes = readFileSync(join(directory, file));
    hash.update(`${path.length}:`).update(path).update(`${bytes.length}:`).update(bytes);
  }
  return { id, version: manifest.version, sha256: hash.digest("hex") };
}

export function parseReleases(raw: unknown): Release[] {
  if (!Array.isArray(raw)) throw new Error("Release index must be an array");
  const seen = new Set<string>();
  return raw.map((entry: unknown) => {
    if (typeof entry !== "object" || entry === null) throw new Error("Invalid release entry");
    const r = entry as Record<string, unknown>;
    if (
      typeof r.id !== "string" ||
      !ID.test(r.id) ||
      typeof r.version !== "string" ||
      !r.version.trim() ||
      r.version !== r.version.trim() ||
      typeof r.sha256 !== "string" ||
      !/^[a-f0-9]{64}$/.test(r.sha256) ||
      Object.keys(r).sort().join(",") !== "id,sha256,version"
    ) {
      throw new Error("Each release needs only id, version and a lowercase SHA-256 hash");
    }
    const key = JSON.stringify([r.id, r.version]);
    if (seen.has(key)) throw new Error(`Duplicate reviewed release ${r.id}@${r.version}`);
    seen.add(key);
    return { id: r.id, version: r.version, sha256: r.sha256 };
  });
}

export function readReleases(root: string, missingAllowed = false): Release[] {
  try {
    const path = join(root, RELEASE_INDEX);
    if (!lstatSync(path).isFile()) throw new Error("Release index must be a regular file");
    return parseReleases(JSON.parse(readFileSync(path, "utf8")));
  } catch (error) {
    if (missingAllowed && (error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }
}

export function releaseStatus(release: Release, index: readonly Release[]): "reviewed" | "pending" {
  const approved = index.find((r) => r.id === release.id && r.version === release.version);
  if (!approved) return "pending";
  if (approved.sha256 !== release.sha256) {
    throw new Error(
      `${release.id}@${release.version}: hash mismatch (reviewed ${approved.sha256}, actual ${release.sha256}); bump the version and request review`,
    );
  }
  return "reviewed";
}

export function verifyReleases(root: string, allowPending = false): string[] {
  const folders = pluginFolders(root);
  const index = readReleases(root);
  return folders.map((id) => {
    const release = hashPlugin(root, id);
    const status = releaseStatus(release, index);
    if (status === "pending" && !allowPending)
      throw new Error(
        `${id}@${release.version}: missing release index entry (sha256 ${release.sha256}); refusing the whole faces install`,
      );
    return `${id}@${release.version}: ${status === "pending" ? "not yet in the release index (review required)" : "reviewed hash verified"}; sha256 ${release.sha256}`;
  });
}
