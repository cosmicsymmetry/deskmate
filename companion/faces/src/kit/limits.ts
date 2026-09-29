/**
 * The one number three different caps are all expressions of, kept here so they cannot
 * drift apart silently.
 *
 * `INGEST_CAP_BYTES` is the ingest cap: the most of any ONE payload this package will
 * take in or hand on in a single step. Three places spend it, and the plugin contract
 * spec ties them together on purpose:
 *   - `kit/http.ts`'s `MAX_BODY_BYTES` -- the most one HTTP response body may be read
 *     to before the fetch is refused;
 *   - `plugins/card.ts`'s `MAX_PNG_BYTES` -- the most one raster frame (the top-level
 *     `{png}` card shape, and each `img` inside a `{layout}` one) may be;
 *   - `plugins/run.ts`'s `MAX_BYTES` -- the render's whole response-body budget, which
 *     the spec states as four times this, "tighter than 8 requests x 1 MB each".
 *
 * They were three independent `1 MB` literals in three files until the whole-branch
 * review; a change to one that silently left the other two behind is exactly the drift
 * this constant exists to make impossible.
 */
export const INGEST_CAP_BYTES = 1024 * 1024;
