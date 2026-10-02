import { join } from "node:path";
import { verifyReleases } from "../plugins/releases";

if (import.meta.main) {
  try {
    const [mode, root = join(import.meta.dir, "../../plugins"), ...extra] = process.argv.slice(2);
    if ((mode !== "verify" && mode !== "check") || extra.length)
      throw new Error("Usage: bun run src/author/releases.ts <verify|check> [plugins-directory]");
    for (const line of verifyReleases(root, mode === "check")) console.log(line);
  } catch (error) {
    console.error(
      `Release verification failed: ${error instanceof Error ? error.message : String(error)}`,
    );
    process.exitCode = 1;
  }
}
