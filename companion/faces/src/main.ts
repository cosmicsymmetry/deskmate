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
  type RenderContext,
  type Settings,
  type TapEvent,
  type ViewId,
} from "./face";
import { pngFromSvg } from "./kit/raster";
import { FACES, faceOfKind } from "./registry";

const EXIT_CONFIGURATION = 2;
const EXIT_TRANSIENT = 1;

export function describeCatalog(faces: readonly FaceDefinition[] = FACES): string {
  return JSON.stringify(
    faces.map(({ kind, label, fields, tap }) =>
      tap === undefined ? { kind, label, fields } : { kind, label, fields, tap },
    ),
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

export function viewsRequest(
  request: FaceRequest,
  face?: FaceDefinition,
): { views: ViewId[] } {
  const definition = definitionFor(request, face);
  return {
    views: definition.views?.(settingsFor(request), request.state) ?? [""],
  };
}

export function tapRequest(
  request: FaceRequest & { event?: unknown },
  face?: FaceDefinition,
): { view: ViewId; state?: unknown } {
  const definition = definitionFor(request, face);
  if (definition.onTap === undefined) {
    return { view: "" };
  }
  const event = tapEvent(request.event);
  if (event === undefined) {
    throw new Error("the tap request has no valid event");
  }
  return definition.onTap(settingsFor(request), request.state, event);
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
  });
  const svg = typeof result === "string" ? result : result.svg;
  const png = Buffer.from(pngFromSvg(svg)).toString("base64");
  // A bare SVG means "leave the stored state alone"; an explicit null clears it.
  return typeof result === "string" || !("state" in result)
    ? { png }
    : { png, state: result.state };
}

function describe(): string {
  return describeCatalog();
}

function requestFromJson(input: string, verb: string): FaceRequest & {
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

async function render(input: string, now: Date): Promise<string> {
  return JSON.stringify(
    await renderRequest(requestFromJson(input, "render"), undefined, now),
  );
}

function views(input: string): string {
  return JSON.stringify(viewsRequest(requestFromJson(input, "views")));
}

function tap(input: string): string {
  return JSON.stringify(tapRequest(requestFromJson(input, "tap")));
}

async function main(): Promise<number> {
  const verb = process.argv[2];
  if (verb === "describe") {
    process.stdout.write(`${describe()}\n`);
    return 0;
  }
  if (verb === "render") {
    // Bun.write is awaited to completion; process.stdout.write is not, and an
    // immediate process.exit can truncate a ~60 KB envelope mid-flight.
    await Bun.write(Bun.stdout, await render(await Bun.stdin.text(), new Date()));
    return 0;
  }
  if (verb === "views") {
    await Bun.write(Bun.stdout, views(await Bun.stdin.text()));
    return 0;
  }
  if (verb === "tap") {
    await Bun.write(Bun.stdout, tap(await Bun.stdin.text()));
    return 0;
  }
  process.stderr.write("usage: main.ts describe | render | views | tap < request.json > result.json\n");
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
