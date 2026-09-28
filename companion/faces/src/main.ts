#!/usr/bin/env bun
// The seam between the Rust server and the faces. The server runs this file as a
// subprocess and speaks two verbs:
//
//   main.ts describe   -> stdout: the face catalog as JSON. The browser's add menu
//                         and settings form are built from it.
//   main.ts render     <- stdin:  {"kind": "...", "settings": {...}, "state": ...}
//                      -> stdout: {"png": "<base64>", "state": ...}, which the server
//                         feeds to the same ingest path an external producer's POST
//                         arrives through.
//
// Exit codes are the error taxonomy: 0 is a frame, 2 means the owner must change a
// setting (retrying cannot help), anything else is transient and the stored frame
// stays. The one line on stderr is what the server logs. Nothing but the JSON result
// may reach stdout in `render`, which is why diagnostics never use `console.log` here.

import { ConfigurationError, type FaceDefinition, type RenderContext, type Settings } from "./face";
import { pngFromSvg } from "./kit/raster";
import { warmSandbox } from "./plugins/sandbox";
import { FACES, allFaces, faceOfKind } from "./registry";

const EXIT_CONFIGURATION = 2;
const EXIT_TRANSIENT = 1;

export function describeCatalog(faces: readonly FaceDefinition[] = FACES): string {
  return JSON.stringify(
    faces.map(({ kind, label, fields, tap, refreshSeconds }) => ({
      kind,
      label,
      fields,
      ...(tap === undefined ? {} : { tap }),
      ...(refreshSeconds === undefined ? {} : { refreshSeconds }),
    })),
    null,
    2,
  );
}

function tapEvent(value: unknown): RenderContext["event"] {
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

export async function renderRequest(
  request: {
    kind?: unknown;
    settings?: unknown;
    state?: unknown;
    event?: unknown;
    timezone?: unknown;
  },
  face?: FaceDefinition,
  now: Date = new Date(),
): Promise<{ png: string; state?: unknown }> {
  const definition =
    face ?? (typeof request.kind === "string" ? faceOfKind(request.kind) : undefined);
  if (definition === undefined) {
    throw new ConfigurationError(
      `this faces package has no face of kind ${JSON.stringify(request.kind)}`,
    );
  }
  const values =
    typeof request.settings === "object" && request.settings !== null
      ? (request.settings as Settings)
      : {};
  const result = await definition.render(values, now, {
    state: request.state,
    event: tapEvent(request.event),
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

async function describe(): Promise<string> {
  return describeCatalog(await allFaces());
}

async function render(input: string, now: Date): Promise<string> {
  let request: unknown;
  try {
    request = JSON.parse(input);
  } catch {
    throw new Error("the render request on stdin is not JSON");
  }
  const req = (request ?? {}) as {
    kind?: unknown;
    settings?: unknown;
    state?: unknown;
    event?: unknown;
    timezone?: unknown;
  };
  // Discovered fresh for this render, so a plugin folder added, fixed or removed
  // since the last invocation is picked up without restarting anything (the built-in
  // faces are unaffected either way -- `allFaces()` always returns them).
  const faces = await allFaces();
  const definition =
    typeof req.kind === "string" ? faces.find((face) => face.kind === req.kind) : undefined;
  return JSON.stringify(await renderRequest(req, definition, now));
}

async function main(): Promise<number> {
  const verb = process.argv[2];
  if (verb === "describe" || verb === "render") {
    // Discovery verifies each plugin folder by running its `plan()` in the sandbox
    // (`plugins/discovery.ts`), and a render that resolves to a plugin runs its
    // `plan`/`render` there too -- both need the WASM runtime loaded first.
    await warmSandbox();
  }
  if (verb === "describe") {
    process.stdout.write(`${await describe()}\n`);
    return 0;
  }
  if (verb === "render") {
    // Bun.write is awaited to completion; process.stdout.write is not, and an
    // immediate process.exit can truncate a ~60 KB envelope mid-flight.
    await Bun.write(Bun.stdout, await render(await Bun.stdin.text(), new Date()));
    return 0;
  }
  process.stderr.write("usage: main.ts describe | render < request.json > result.json\n");
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
