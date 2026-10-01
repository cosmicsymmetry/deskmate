// Author-side fixture runner; never loaded by the sandbox or the hosted plugin.
import { join } from "node:path";
import { pngFromSvg } from "../../src/kit/raster";
import { renderRequest } from "../../src/main";
import { discoverPlugins } from "../../src/plugins/discovery";
import { warmSandbox } from "../../src/plugins/sandbox";

await warmSandbox();
const { faces } = await discoverPlugins(join(import.meta.dir, ".."));
const face = faces.find((entry) => entry.kind === "days-left-this-year");
if (!face) throw new Error("Days Left This Year was not discovered");

const cases = [
  { name: "october", instant: "2026-10-01T12:00:00Z" },
  { name: "last-day", instant: "2026-12-31T23:59:59Z" },
  { name: "leap-new-year", instant: "2028-01-01T00:00:00Z" },
];
for (const faceStyle of ["bar", "squares", "dots"]) {
  for (const { name, instant } of cases) {
    const filename = faceStyle === "bar" ? name : `${name}-${faceStyle}`;
    const { png } = await renderRequest(
      { kind: face.kind, settings: { face: faceStyle }, timezone: "UTC" },
      face,
      new Date(instant),
    );
    await Bun.write(
      join(import.meta.dir, "previews", `${filename}.png`),
      Buffer.from(png, "base64"),
    );
    // Scale the real PNG, so the desk-scale evidence cannot use a different layout.
    const scaled = pngFromSvg(
      `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><image width="448" height="368" href="data:image/png;base64,${png}"/></svg>`,
      0.4,
    );
    await Bun.write(join(import.meta.dir, "previews", `${filename}-desk.png`), scaled);
  }
}
console.log("Rendered three dates in all three faces at 448x368 and 40% scale.");
