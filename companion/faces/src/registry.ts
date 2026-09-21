// Every face this package can draw, in the order the add menu offers them.
//
// Adding a face is: write `src/faces/<kind>.ts` exporting a `FaceDefinition`, add it
// here, add its cases to `src/cases.ts`. The server learns about it from
// `main.ts describe` -- there is no Rust change and no binary redeploy.

import type { FaceDefinition } from "./face";
import { hackernews } from "./faces/hackernews";
import { rss } from "./faces/rss";
import { token } from "./faces/token";
import { weather } from "./faces/weather";

export const FACES: readonly FaceDefinition[] = [weather, hackernews, rss, token];

export function faceOfKind(kind: string): FaceDefinition | undefined {
  return FACES.find((face) => face.kind === kind);
}
