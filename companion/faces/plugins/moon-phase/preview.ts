// Runs the shipped sandbox and renderer; no alternate drawing implementation.
import { join } from "node:path";
import { Resvg } from "@resvg/resvg-js";
import { pngFromSvg } from "../../src/kit/raster";
import { renderRequest } from "../../src/main";
import { discoverPlugins } from "../../src/plugins/discovery";
import { warmSandbox } from "../../src/plugins/sandbox";
import { cases } from "./check.json";

await warmSandbox();
const { faces } = await discoverPlugins(join(import.meta.dir, ".."));
const face = faces.find((entry) => entry.kind === "moon-phase");
if (!face) throw new Error("Moon Phase was not discovered");

const panels: string[] = [];
for (const [index, { name, now, settings }] of cases.entries()) {
  const { png } = await renderRequest(
    { kind: face.kind, settings, timezone: "UTC" },
    face,
    new Date(now),
  );
  await Bun.write(join(import.meta.dir, "previews", `${name}.png`), Buffer.from(png, "base64"));
  const scaled = pngFromSvg(
    `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><image width="448" height="368" href="data:image/png;base64,${png}"/></svg>`,
    0.4,
  );
  await Bun.write(join(import.meta.dir, "previews", `${name}-desk.png`), scaled);
  if (index < 8) {
    panels.push(
      `<image x="${(index % 4) * 448}" y="${Math.floor(index / 4) * 368}" width="448" height="368" href="data:image/png;base64,${png}"/>`,
    );
  }
}
await Bun.write(
  join(import.meta.dir, "previews", "phases.png"),
  // Author-only contact sheet of the real PNGs, larger than the panel. The
  // production pngFromSvg deliberately refuses non-panel dimensions.
  new Resvg(
    `<svg xmlns="http://www.w3.org/2000/svg" width="1792" height="736">${panels.join("")}</svg>`,
  )
    .render()
    .asPng(),
);
console.log("Rendered eight phases, Southern view and year rollover at full and desk scale.");
