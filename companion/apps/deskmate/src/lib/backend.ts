/**
 * The one seam between the app and its backend.
 *
 * Everything in `src/` imports the backend from here and only from here, so the
 * implementation behind it can be swapped in exactly one place. `vite.config.ts`
 * does that swap for the browser harness (`VITE_DESKMATE_MOCK=1`), pointing
 * `./backendClient` at `src/dev/backendClient.ts`; a production build resolves
 * the real HTTP client and never reaches `src/dev/`.
 *
 * A barrel rather than an alias on each relative import: there is one specifier
 * to redirect instead of one per importer, so a new call site cannot silently
 * miss the harness.
 */
export * from "./backendClient";
