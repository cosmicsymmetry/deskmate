import { fileURLToPath } from "node:url";

import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const host = process.env.TAURI_DEV_HOST;

// Dev-only browser harness. `VITE_DESKMATE_MOCK=1 bun run dev` swaps the Tauri IPC
// bridge for `src/dev/mockBackend.ts`, so the whole UI — every device, provider,
// validation and ownership state — renders and can be screenshotted in a plain
// browser. Unset (the default, and every production build) resolves the real bridge
// and `src/dev/` is never imported.
const useMockIpc = process.env.VITE_DESKMATE_MOCK === "1";
const devModule = (name: string) => fileURLToPath(new URL(`./src/dev/${name}`, import.meta.url));

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: useMockIpc
      ? {
          "@tauri-apps/api/core": devModule("tauri-core.ts"),
          "@tauri-apps/api/event": devModule("tauri-event.ts"),
        }
      : {},
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: process.env.TAURI_ENV_DEBUG ? false : "oxc",
    sourcemap: Boolean(process.env.TAURI_ENV_DEBUG),
  },
});
