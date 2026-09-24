import type { FieldSpec } from "../face";

export class ManifestError extends Error {
  override name = "ManifestError";
}

const HOSTNAME = /^(?!-)[a-z0-9-]{1,63}(?<!-)(\.(?!-)[a-z0-9-]{1,63}(?<!-))+$/;

export interface SecretSpec {
  key: string;
  label: string;
  kind: "api_key";
  host: string;
  send_as: "bearer" | "header" | "query";
  header?: string;
  param?: string;
}

export interface PluginManifest {
  api: 1;
  id: string;
  version: string;
  label: string;
  description: string;
  author: string;
  hosts: string[];
  secrets: SecretSpec[];
  fields: FieldSpec[];
  refreshSeconds?: number;
  tap?: string;
}

function text(source: Record<string, unknown>, key: string): string {
  const value = source[key];
  if (typeof value !== "string" || value.trim() === "") {
    throw new ManifestError(`${key} must be a non-empty string`);
  }
  return value.trim();
}

export function parseManifest(raw: unknown, folder: string): PluginManifest {
  if (typeof raw !== "object" || raw === null)
    throw new ManifestError("the manifest is not an object");
  const source = raw as Record<string, unknown>;
  if (source.api !== 1)
    throw new ManifestError(
      `unsupported api ${JSON.stringify(source.api)}; this host speaks api 1`,
    );
  const id = text(source, "id");
  if (id !== folder)
    throw new ManifestError(
      `id ${JSON.stringify(id)} does not match its folder ${JSON.stringify(folder)}`,
    );
  const hosts = Array.isArray(source.hosts) ? source.hosts : [];
  for (const host of hosts) {
    if (typeof host !== "string" || !HOSTNAME.test(host)) {
      throw new ManifestError(
        `${JSON.stringify(host)} is not a bare hostname, such as "api.github.com"`,
      );
    }
  }
  const secrets = (Array.isArray(source.secrets) ? source.secrets : []) as SecretSpec[];
  for (const secret of secrets) {
    if (!hosts.includes(secret.host)) {
      throw new ManifestError(
        `secret ${JSON.stringify(secret.key)} names ${secret.host}, which the manifest has not declared`,
      );
    }
  }
  if (secrets.length > 0) {
    const allowed = new Set(secrets.map((secret) => secret.host));
    const extra = hosts.filter((host: string) => !allowed.has(host));
    if (extra.length > 0) {
      throw new ManifestError(
        `a plugin using a secret may declare only the hosts its secrets belong to; remove ${extra.join(", ")}`,
      );
    }
  }
  const refreshSeconds = source.refreshSeconds;
  if (
    refreshSeconds !== undefined &&
    (typeof refreshSeconds !== "number" || !Number.isInteger(refreshSeconds))
  ) {
    throw new ManifestError("refreshSeconds must be a whole number of seconds");
  }
  return {
    api: 1,
    id,
    version: text(source, "version"),
    label: text(source, "label"),
    description: text(source, "description"),
    author: text(source, "author"),
    hosts: hosts as string[],
    secrets,
    fields: (Array.isArray(source.fields) ? source.fields : []) as FieldSpec[],
    ...(refreshSeconds === undefined ? {} : { refreshSeconds }),
    ...(typeof source.tap === "string" ? { tap: source.tap } : {}),
  };
}
