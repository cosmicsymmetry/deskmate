// Every face this package can draw, in the order the add menu offers them.
//
// Adding a BUILT-IN face is: write `src/faces/<kind>.ts` exporting a `FaceDefinition`,
// add it here, add its cases to `src/cases.ts`. Adding a PLUGIN is a folder under
// `plugins/` -- no file here changes, and `discoverPlugins` (`src/plugins/discovery.ts`)
// is what turns it into a `FaceDefinition`. Either way the server learns about it from
// `main.ts describe` -- there is no Rust change and no binary redeploy.

import { ConfigurationError, type FaceDefinition } from "./face";
import { claudeLimits } from "./faces/claude-limits";
import { hackernews } from "./faces/hackernews";
import { rss } from "./faces/rss";
import { token } from "./faces/token";
import { weather } from "./faces/weather";
import { discoverPlugins } from "./plugins/discovery";

export const FACES: readonly FaceDefinition[] = [weather, hackernews, rss, token, claudeLimits];

export function faceOfKind(kind: string): FaceDefinition | undefined {
  return FACES.find((face) => face.kind === kind);
}

/**
 * The built-in faces plus every plugin folder that discovers cleanly, in that order.
 * A built-in face always wins a name collision: a plugin folder cannot shadow
 * `weather`, and a folder that tried to would silently vanish from the catalog rather
 * than override something the owner already relies on. Re-runs discovery on every
 * call -- see `discoverPlugins` for why that is cheap enough to be the point.
 */
export async function allFaces(): Promise<readonly FaceDefinition[]> {
  const { faces, skipped } = await discoverPlugins();
  const builtIn = new Set(FACES.map((face) => face.kind));
  const withdrawn: FaceDefinition[] = skipped
    .filter((s) => s.withdrawn)
    .map((s) => ({
      kind: s.folder,
      label: s.folder,
      fields: [],
      withdrawn: s.reason,
      render() {
        throw new ConfigurationError(s.reason);
      },
    }));
  return [...FACES, ...[...faces, ...withdrawn].filter((face) => !builtIn.has(face.kind))];
}
