// `bun run dump [dir]` -- writes every case as a PNG for a human to look at, plus a
// 0.4x "desk" render: roughly the panel's true physical size on a desktop display,
// which is the honest test of whether type is legible from a chair.
// `bun run dump --update` rewrites test/golden/ after a DELIBERATE design change.

import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { allCases } from "./cases";
import { pngFromSvg } from "./kit/raster";

const argument = process.argv[2];
const golden = fileURLToPath(new URL("../test/golden", import.meta.url));

if (argument === "--update") {
  mkdirSync(golden, { recursive: true });
  for (const { name, svg } of allCases()) {
    writeFileSync(`${golden}/${name}.svg`, svg);
  }
  console.log(`rewrote ${allCases().length} goldens in ${golden}`);
} else {
  const directory = resolve(argument ?? "out");
  mkdirSync(directory, { recursive: true });
  for (const { name, svg } of allCases()) {
    writeFileSync(`${directory}/${name}.png`, pngFromSvg(svg));
    writeFileSync(`${directory}/${name}.desk.png`, pngFromSvg(svg, 0.4));
  }
  console.log(`wrote ${allCases().length} faces to ${directory}`);
}
