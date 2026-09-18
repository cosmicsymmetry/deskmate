import { fileURLToPath } from "node:url";

import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Dev-only browser harness. `VITE_DESKMATE_MOCK=1 bun run dev` swaps the backend
// for `src/dev/backendClient.ts`, so the whole UI — every device, provider,
// validation and ownership state — renders and can be screenshotted without a
// server. Unset (the default, and every production build) resolves the real HTTP
// client and `src/dev/` is never imported.
//
// One entry, because `src/lib/backend.ts` is the only module that imports
// `./backendClient`: every other file goes through that barrel, so a new call
// site cannot accidentally bypass the harness.
const useMockBackend = process.env.VITE_DESKMATE_MOCK === "1";
const devModule = (name: string) => fileURLToPath(new URL(`./src/dev/${name}`, import.meta.url));

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: useMockBackend ? { "./backendClient": devModule("backendClient.ts") } : {},
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // The harness talks to nothing, and a real backend is reached by building
    // and letting the server serve `dist/`. A proxy here would invite a third
    // way to run the app whose auth story differs from the deployed one.
  },
  envPrefix: ["VITE_"],
  build: {
    // The companion is served over HTTPS to modern browsers; nothing here needs
    // to run on a browser that predates ES2022.
    target: "es2022",
    minify: true,
    sourcemap: true,
  },
});
