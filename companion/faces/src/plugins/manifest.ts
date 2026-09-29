import { isIP } from "node:net";

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

function validateSecret(secret: unknown): SecretSpec {
  if (typeof secret !== "object" || secret === null) {
    throw new ManifestError("each secret must be an object");
  }
  const s = secret as Record<string, unknown>;
  const key = s.key;
  if (typeof key !== "string" || key.trim() === "") {
    throw new ManifestError("secret key must be a non-empty string");
  }
  const label = s.label;
  if (typeof label !== "string" || label.trim() === "") {
    throw new ManifestError("secret label must be a non-empty string");
  }
  const kind = s.kind;
  if (kind !== "api_key") {
    throw new ManifestError(`secret kind must be "api_key", not ${JSON.stringify(kind)}`);
  }
  const host = s.host;
  if (typeof host !== "string") {
    throw new ManifestError("secret host must be a string");
  }
  const send_as = s.send_as;
  if (send_as !== "bearer" && send_as !== "header" && send_as !== "query") {
    throw new ManifestError(
      `secret send_as must be "bearer", "header", or "query", not ${JSON.stringify(send_as)}`,
    );
  }
  const header = s.header;
  if (header !== undefined && typeof header !== "string") {
    throw new ManifestError("secret header must be a string");
  }
  const param = s.param;
  if (param !== undefined && typeof param !== "string") {
    throw new ManifestError("secret param must be a string");
  }
  // `header` and `param` PIN the credential: `requests.ts` substitutes only into the
  // one they name. A manifest that names the position its `send_as` does not use is
  // refused rather than silently ignored -- a reviewer reads these fields and has to
  // be able to believe them.
  if (header !== undefined && send_as === "query") {
    throw new ManifestError('a secret sent as "query" names a param, not a header');
  }
  if (param !== undefined && send_as !== "query") {
    throw new ManifestError(`a secret sent as "${send_as}" names a header, not a param`);
  }
  return {
    key: key.trim(),
    label: label.trim(),
    kind,
    host,
    send_as,
    ...(header !== undefined ? { header } : {}),
    ...(param !== undefined ? { param } : {}),
  };
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
  if (!Array.isArray(source.hosts)) throw new ManifestError("hosts must be an array");
  const hosts = source.hosts as unknown[];
  for (const host of hosts) {
    if (typeof host !== "string") {
      throw new ManifestError(`each host must be a string, not ${typeof host}`);
    }
    if (isIP(host) !== 0) {
      throw new ManifestError(`${JSON.stringify(host)} is an IP literal, not a bare hostname`);
    }
    if (host.startsWith("[") && host.endsWith("]")) {
      throw new ManifestError(
        `${JSON.stringify(host)} is a bracketed IPv6 address, not a bare hostname`,
      );
    }
    if (!HOSTNAME.test(host)) {
      throw new ManifestError(
        `${JSON.stringify(host)} is not a bare hostname, such as "api.github.com"`,
      );
    }
  }
  if (!Array.isArray(source.secrets)) throw new ManifestError("secrets must be an array");
  const secrets: SecretSpec[] = [];
  for (const secret of source.secrets as unknown[]) {
    secrets.push(validateSecret(secret));
  }
  for (const secret of secrets) {
    if (!(hosts as string[]).includes(secret.host)) {
      throw new ManifestError(
        `secret ${JSON.stringify(secret.key)} names ${secret.host}, which the manifest has not declared`,
      );
    }
  }
  if (secrets.length > 0) {
    const allowed = new Set(secrets.map((secret) => secret.host));
    const extra = (hosts as string[]).filter((host: string) => !allowed.has(host));
    if (extra.length > 0) {
      throw new ManifestError(
        `a plugin using a secret may declare only the hosts its secrets belong to; remove ${extra.join(", ")}`,
      );
    }
  }
  if (!Array.isArray(source.fields)) throw new ManifestError("fields must be an array");
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
    fields: source.fields as unknown[] as FieldSpec[],
    ...(refreshSeconds === undefined ? {} : { refreshSeconds }),
    ...(typeof source.tap === "string" ? { tap: source.tap } : {}),
  };
}
