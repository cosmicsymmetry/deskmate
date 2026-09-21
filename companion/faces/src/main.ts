#!/usr/bin/env bun
// The seam between the Rust server and the faces. The server runs this file as a
// subprocess and speaks two verbs:
//
//   main.ts describe   -> stdout: the face catalog as JSON. The browser's add menu
//                         and settings form are built from it.
//   main.ts render     <- stdin:  {"kind": "...", "settings": {...}}
//                      -> stdout: a 448x368 PNG, which the server feeds to the same
//                         ingest path an external producer's POST arrives through.
//
// Exit codes are the error taxonomy: 0 is a frame, 2 means the owner must change a
// setting (retrying cannot help), anything else is transient and the stored frame
// stays. The one line on stderr is what the server logs. Nothing but the PNG may
// reach stdout in `render`, which is why diagnostics never use `console.log` here.

import { ConfigurationError, type Settings } from "./face";
import { pngFromSvg } from "./kit/raster";
import { FACES, faceOfKind } from "./registry";

const EXIT_CONFIGURATION = 2;
const EXIT_TRANSIENT = 1;

function describe(): string {
  return JSON.stringify(
    FACES.map(({ kind, label, fields }) => ({ kind, label, fields })),
    null,
    2,
  );
}

async function render(input: string, now: Date): Promise<Uint8Array> {
  let request: unknown;
  try {
    request = JSON.parse(input);
  } catch {
    throw new Error("the render request on stdin is not JSON");
  }
  const { kind, settings } = (request ?? {}) as { kind?: unknown; settings?: unknown };
  const face = typeof kind === "string" ? faceOfKind(kind) : undefined;
  if (face === undefined) {
    throw new ConfigurationError(`this faces package has no face of kind ${JSON.stringify(kind)}`);
  }
  const values = typeof settings === "object" && settings !== null ? (settings as Settings) : {};
  return pngFromSvg(await face.render(values, now));
}

async function main(): Promise<number> {
  const verb = process.argv[2];
  if (verb === "describe") {
    process.stdout.write(`${describe()}\n`);
    return 0;
  }
  if (verb === "render") {
    const png = await render(await Bun.stdin.text(), new Date());
    await Bun.write(Bun.stdout, png);
    return 0;
  }
  process.stderr.write("usage: main.ts describe | render < request.json > face.png\n");
  return 64;
}

main()
  .then((code) => process.exit(code))
  .catch((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`${message.replace(/\s+/g, " ").slice(0, 300)}\n`);
    process.exit(error instanceof ConfigurationError ? EXIT_CONFIGURATION : EXIT_TRANSIENT);
  });
