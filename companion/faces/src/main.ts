#!/usr/bin/env bun
// The seam between the Rust server and the faces. The server runs this file as a
// subprocess and speaks four verbs:
//
//   main.ts describe   -> stdout: the face catalog as JSON. The browser's add menu
//                         and settings form are built from it.
//   main.ts render     <- stdin:  {"kind": "...", "settings": {...}, "state": ...}
//                      -> stdout: {"png": "<base64>", "state": ...}, which the server
//                         feeds to the same ingest path an external producer's POST
//                         arrives through.
//   main.ts views      <- stdin:  {"kind": "...", "settings": {...}, "state": ...}
//                      -> stdout: {"views": ["", ...]}
//   main.ts tap        <- stdin:  {"kind": "...", "settings": {...}, "state": ...,
//                                "event": {"taps": 1, "point": null}}
//                      -> stdout: {"view": "...", "state": ...}
//
// Exit codes are the error taxonomy: 0 is a frame, 2 means the owner must change a
// setting (retrying cannot help), anything else is transient and the stored frame
// stays. The one line on stderr is what the server logs. Nothing but the JSON result
// may reach stdout in `render`, which is why diagnostics never use `console.log` here.

import {
  ConfigurationError,
  type FaceDefinition,
  type Settings,
  type TapEvent,
  type ViewId,
} from "./face";
import { pngFromSvg } from "./kit/raster";
import { warmSandbox } from "./plugins/sandbox";
import { FACES, allFaces, faceOfKind } from "./registry";

const EXIT_CONFIGURATION = 2;
const EXIT_TRANSIENT = 1;

export function describeCatalog(faces: readonly FaceDefinition[] = FACES): string {
  return JSON.stringify(
    faces.map((face) => {
      const { kind, label, fields, tap, refreshSeconds, withdrawn } = face;
      return {
        kind,
        label,
        fields,
        ...(withdrawn === undefined ? {} : { withdrawn }),
        ...(tap === undefined ? {} : { tap }),
        // snake_case on the wire, deliberately: this JSON is read by
        // `crates/server/src/data_cards/faces_package.rs`'s `CatalogFace`, which has no
        // `#[serde(rename)]` and no `deny_unknown_fields` -- a camelCase key here is
        // silently ignored, not refused, and the field quietly deserializes to `None`.
        // `kind`/`label`/`fields`/`tap` are single words so this was invisible until a
        // compound-word field arrived. `FaceDefinition.refreshSeconds` and the
        // manifest's own `refreshSeconds` (author-facing) are unaffected -- only this
        // wire-facing key changes shape.
        ...(refreshSeconds === undefined ? {} : { refresh_seconds: refreshSeconds }),
        // What the server may skip. A built-in is recognised by identity, not by
        // kind, so a plugin folder named like a built-in cannot claim to be one.
        // A withdrawn tombstone can do nothing, so it declares nothing.
        origin: faceOfKind(kind) === face ? "builtin" : "plugin",
        views: withdrawn === undefined && face.views !== undefined,
        selector: withdrawn === undefined && face.onTap !== undefined,
      };
    }),
    null,
    2,
  );
}

function tapEvent(value: unknown): TapEvent | undefined {
  if (typeof value !== "object" || value === null) {
    return undefined;
  }
  const { taps, point } = value as { taps?: unknown; point?: unknown };
  if (typeof taps !== "number" || !Number.isFinite(taps) || !Number.isInteger(taps)) {
    return undefined;
  }
  const boundedTaps = Math.min(32, Math.max(1, taps));
  // An absent point is as valid as an explicit null: C1 has no point to send, and a
  // dropped event here would silently swallow the tap.
  if (point === null || point === undefined) {
    return { taps: boundedTaps, point: null };
  }
  if (typeof point !== "object") {
    return undefined;
  }
  const { x, y } = point as { x?: unknown; y?: unknown };
  if (
    typeof x !== "number" ||
    !Number.isFinite(x) ||
    typeof y !== "number" ||
    !Number.isFinite(y)
  ) {
    return undefined;
  }
  return { taps: boundedTaps, point: { x, y } };
}

interface FaceRequest {
  kind?: unknown;
  settings?: unknown;
  state?: unknown;
  timezone?: unknown;
}

function definitionFor(request: FaceRequest, face?: FaceDefinition): FaceDefinition {
  const definition =
    face ?? (typeof request.kind === "string" ? faceOfKind(request.kind) : undefined);
  if (definition === undefined) {
    throw new ConfigurationError(
      `this faces package has no face of kind ${JSON.stringify(request.kind)}`,
    );
  }
  return definition;
}

function settingsFor(request: FaceRequest): Settings {
  return typeof request.settings === "object" && request.settings !== null
    ? (request.settings as Settings)
    : {};
}

export function viewsRequest(request: FaceRequest, face?: FaceDefinition): { views: ViewId[] } {
  const definition = definitionFor(request, face);
  return {
    views: definition.views?.(settingsFor(request), request.state) ?? [""],
  };
}

export function tapRequest(
  request: FaceRequest & { event?: unknown },
  face?: FaceDefinition,
  now: Date = new Date(),
): { view: ViewId; state?: unknown } {
  const definition = definitionFor(request, face);
  if (definition.onTap === undefined) {
    throw new Error("this face handles taps through render");
  }
  const event = tapEvent(request.event);
  if (event === undefined) {
    throw new Error("the tap request has no valid event");
  }
  return definition.onTap(settingsFor(request), request.state, event, now);
}

export async function renderRequest(
  request: FaceRequest & { event?: unknown; view?: unknown },
  face?: FaceDefinition,
  now: Date = new Date(),
): Promise<{ png: string; state?: unknown }> {
  const definition = definitionFor(request, face);
  const result = await definition.render(settingsFor(request), now, {
    state: request.state,
    event: tapEvent(request.event),
    view: typeof request.view === "string" ? request.view : undefined,
    // Additive: an older server simply omits this, and a plugin that needs a zone
    // falls back to the host process's own (see `plugins/discovery.ts`).
    ...(typeof request.timezone === "string" ? { timezone: request.timezone } : {}),
  });
  const svg = typeof result === "string" ? result : result.svg;
  const png = Buffer.from(pngFromSvg(svg)).toString("base64");
  // A bare SVG means "leave the stored state alone"; an explicit null clears it.
  return typeof result === "string" || !("state" in result)
    ? { png }
    : { png, state: result.state };
}

function requestFromJson(
  input: string,
  verb: string,
): FaceRequest & {
  event?: unknown;
  view?: unknown;
} {
  let request: unknown;
  try {
    request = JSON.parse(input);
  } catch {
    throw new Error(`the ${verb} request on stdin is not JSON`);
  }
  return (request ?? {}) as FaceRequest & { event?: unknown; view?: unknown };
}

async function requestedFace(request: FaceRequest): Promise<FaceDefinition> {
  // Built-in planning stays on the fast path; plugin discovery runs sandboxed.
  const builtin = typeof request.kind === "string" ? faceOfKind(request.kind) : undefined;
  if (builtin !== undefined) return builtin;
  await warmSandbox();
  const faces = await allFaces();
  const face = definitionFor(
    request,
    faces.find((face) => face.kind === request.kind),
  );
  if (face.withdrawn) throw new ConfigurationError(face.withdrawn);
  return face;
}

async function render(input: string, now: Date): Promise<string> {
  const request = requestFromJson(input, "render");
  return JSON.stringify(await renderRequest(request, await requestedFace(request), now));
}

async function views(input: string): Promise<string> {
  const request = requestFromJson(input, "views");
  return JSON.stringify(viewsRequest(request, await requestedFace(request)));
}

async function tap(input: string): Promise<string> {
  const request = requestFromJson(input, "tap");
  return JSON.stringify(tapRequest(request, await requestedFace(request)));
}

async function main(): Promise<number> {
  const verb = process.argv[2];
  if (verb === "tap-worker") {
    const { serveTapWorker } = await import("./tap-worker");
    await serveTapWorker(tap);
    return 0;
  }
  if (verb === "describe") {
    // Discovery verifies each plugin folder by running its `plan()` in the sandbox
    // (`plugins/discovery.ts`), and a render that resolves to a plugin runs its
    // `plan`/`render` there too -- both need the WASM runtime loaded first.
    await warmSandbox();
    process.stdout.write(`${describeCatalog(await allFaces())}\n`);
    return 0;
  }
  if (verb === "render") {
    // Bun.write is awaited to completion; process.stdout.write is not, and an
    // immediate process.exit can truncate a ~60 KB envelope mid-flight.
    await Bun.write(Bun.stdout, await render(await Bun.stdin.text(), new Date()));
    return 0;
  }
  if (verb === "views") {
    await Bun.write(Bun.stdout, await views(await Bun.stdin.text()));
    return 0;
  }
  if (verb === "tap") {
    await Bun.write(Bun.stdout, await tap(await Bun.stdin.text()));
    return 0;
  }
  process.stderr.write(
    "usage: main.ts describe | render | views | tap < request.json > result.json\n",
  );
  return 64;
}

if (import.meta.main) {
  main()
    .then((code) => process.exit(code))
    .catch((error: unknown) => {
      const message = error instanceof Error ? error.message : String(error);
      process.stderr.write(`${message.replace(/\s+/g, " ").slice(0, 300)}\n`);
      process.exit(error instanceof ConfigurationError ? EXIT_CONFIGURATION : EXIT_TRANSIENT);
    });
}
