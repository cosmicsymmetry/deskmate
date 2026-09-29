// Where a plugin's stored credentials live: a JSON file under `DESKMATE_CONFIG_DIR`,
// read fresh for every render, never cached at process start. `runPlugin` never sees
// anything but the one plugin's own entry -- not the other plugins', and not the
// file's shape.
//
// `DESKMATE_CONFIG_DIR` here is NOT the server's own config root -- the server
// rebinds it per `render` (never per `describe`) to the one account's directory
// whose card is being drawn (`companion/crates/server/src/data_cards/faces_package.rs`).
// So the path below has no `accounts/<id>` segment to add: from this process, the
// variable already points at that one account's own directory. See
// `docs/plugins/contract-v1.md` ("Configure a secret") for the operator-facing path,
// which does have that segment, because it is read relative to the SERVER's own
// `DESKMATE_CONFIG_DIR`.
//
// Shape on disk, `plugin-secrets.json`:
//
//   {
//     "<plugin id>": { "<secret key>": "<value>" },
//     "<plugin id>": { "<secret key>": "<value>" }
//   }
//
// A key is whatever a plugin's manifest names in `secrets[].key`
// (`src/plugins/manifest.ts`); the value is the plaintext credential
// `src/plugins/requests.ts` substitutes for `{{secret:<key>}}`. Documenting this file
// for the owners who edit it is Task 11's job -- this module only reads it.

import { ConfigurationError } from "../face";

const SECRETS_FILE = "plugin-secrets.json";

function configDir(): string | undefined {
  const dir = process.env.DESKMATE_CONFIG_DIR;
  return dir !== undefined && dir.trim() !== "" ? dir : undefined;
}

/**
 * This plugin's own secrets, as `{key: value}`. A missing `DESKMATE_CONFIG_DIR`, a
 * missing file, or a file with no entry for this plugin are all the ordinary case --
 * "no secrets configured" -- and return `{}`, never an error: most plugins declare
 * none, and most renders of a plugin that does happen before the owner has typed one
 * in. A file that exists but does not parse as the documented shape is the owner's
 * problem to fix (a `ConfigurationError` naming the file), because nothing about it
 * changes on a retry.
 */
export async function readPluginSecrets(pluginId: string): Promise<Record<string, string>> {
  const dir = configDir();
  if (dir === undefined) return {};
  const path = `${dir}/${SECRETS_FILE}`;
  const file = Bun.file(path);
  if (!(await file.exists())) return {};
  let raw: unknown;
  try {
    raw = JSON.parse(await file.text());
  } catch {
    throw new ConfigurationError(`${path} is not valid JSON`);
  }
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    throw new ConfigurationError(`${path} must be an object of plugin id -> secrets`);
  }
  const entry = (raw as Record<string, unknown>)[pluginId];
  if (entry === undefined) return {};
  if (typeof entry !== "object" || entry === null || Array.isArray(entry)) {
    throw new ConfigurationError(
      `${path}'s entry for ${JSON.stringify(pluginId)} must be an object`,
    );
  }
  const secrets: Record<string, string> = {};
  for (const [key, value] of Object.entries(entry as Record<string, unknown>)) {
    if (typeof value !== "string") {
      throw new ConfigurationError(
        `${path}'s entry for ${JSON.stringify(pluginId)}.${key} must be a string`,
      );
    }
    secrets[key] = value;
  }
  return secrets;
}
