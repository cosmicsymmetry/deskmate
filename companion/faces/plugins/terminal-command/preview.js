// Compose only PNGs produced by the real offline author checker.
import { mkdir, readFile, copyFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { Resvg } from "@resvg/resvg-js";
import { checkPlugin } from "../../src/author/check";

const output = join(import.meta.dir, "../../out/plugins/terminal-command");
const previews = join(import.meta.dir, "previews");
const report = await checkPlugin("terminal-command");
if (!report.ok) throw new Error(JSON.stringify(report));
await mkdir(previews, { recursive: true });
const samples = new Map([
  ["ls -lah", "files"],
  ["find . -type f -name '*.log'", "find"],
  ["uname -srm", "system"],
  ["date -d tomorrow '+%F'", "linux-date"],
  ["date -v+1d '+%F'", "macos-date"],
  ["pmset -g batt", "battery"],
]);
const desk = [];
const full = [];
for (const [i, entry] of report.cases.entries()) {
  const name = samples.get(entry.name);
  if (name) {
    await copyFile(join(output, entry.png), join(previews, `${name}.png`));
    await copyFile(join(output, entry.desk), join(previews, `${name}.desk.png`));
  }
  if (i >= 30) continue;
  const png = (await readFile(join(output, entry.png))).toString("base64");
  const small = (await readFile(join(output, entry.desk))).toString("base64");
  desk.push(
    `<image x="${(i % 5) * 195}" y="${Math.floor(i / 5) * 163}" width="179" height="147" href="data:image/png;base64,${small}"/>`,
  );
  full.push(
    `<image x="${(i % 3) * 464}" y="${Math.floor((i % 6) / 3) * 384}" width="448" height="368" href="data:image/png;base64,${png}"/>`,
  );
  if (i % 6 === 5) {
    await writeFile(
      join(output, `sheet-${Math.floor(i / 6) + 1}.png`),
      new Resvg(
        `<svg xmlns="http://www.w3.org/2000/svg" width="1392" height="768">${full.join("")}</svg>`,
      )
        .render()
        .asPng(),
    );
    full.length = 0;
  }
}
await writeFile(
  join(previews, "all-commands.desk.png"),
  new Resvg(
    `<svg xmlns="http://www.w3.org/2000/svg" width="975" height="978">${desk.join("")}</svg>`,
  )
    .render()
    .asPng(),
);
console.log(`Rendered ${report.cases.length} full/desk pairs in ${output}; samples in ${previews}`);
