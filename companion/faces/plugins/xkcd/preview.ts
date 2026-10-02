// Generates previews through the production checker; all media fixtures are synthetic.
import { copyFile, mkdir } from "node:fs/promises";
import { join } from "node:path";
import { checkPlugin } from "../../src/author/check";
const id = "xkcd";
const report = await checkPlugin(id);
if (!report.ok) throw new Error(JSON.stringify(report.errors));
await mkdir(join(import.meta.dir, "previews"), { recursive: true });
for (const [i, entry] of report.cases.entries()) {
  if (!entry.png || !entry.desk) throw new Error(`Missing preview ${entry.name}`);
  await copyFile(
    join(import.meta.dir, "../../out/plugins", id, entry.png),
    join(import.meta.dir, "previews", `${i + 1}-full.png`),
  );
  await copyFile(
    join(import.meta.dir, "../../out/plugins", id, entry.desk),
    join(import.meta.dir, "previews", `${i + 1}-desk.png`),
  );
}
console.log(`Wrote ${report.cases.length} synthetic full-size and desk previews.`);
