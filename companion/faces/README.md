# Deskmate faces

The server-rendered card faces: weather, Hacker News, RSS, token, Claude limits. Each one fetches its
data, draws a 448x368 SVG, and the server pushes the rastered frame to an ordinary
picture card. A face is **never a card kind** -- see `CLAUDE.md`, "Product rules".

The whole story, including why this is TypeScript and how the server talks to it, is
`docs/images/server-rendered-cards.md`. This file is the short version for someone
about to change a face.

```sh
bun install
bun test                 # goldens (byte-exact SVG), layout invariants, parsers, the CLI
bun run check && bun run lint && bun run format:check
bun run dump out/        # every case as a PNG -- LOOK at them, the .desk.png ones too
bun run dump --update    # after a DELIBERATE design change: rewrite test/golden/
echo '{"kind":"hackernews","settings":{}}' | bun run src/main.ts render > /tmp/face.png
```

## Layout

| | |
|---|---|
| `src/faces/<kind>.ts` | One face: its fields, its fetch, its view model, its drawing. |
| `src/registry.ts` | The list the add menu is built from. |
| `src/cases.ts` | The review set. Build cases from captured responses, not invented ones. |
| `src/kit/svg.ts` | `Canvas`, `wrap`, `fit`, `fitTracked`, `fitSize`, and `fixed`. |
| `src/kit/raster.ts` | `resvg`, the two bundled Inter faces, and `textWidth`. |
| `src/kit/http.ts` | The only way a face reaches the network. |
| `src/kit/theme.ts` | `DESIGN.md`'s dark column and the type scale. |
| `src/main.ts` | The seam the Rust server runs: `describe` and `render`. |

## Claude limits

`claude-limits` offers one text setting, **Usage feed** (`url`), accepting a public
HTTPS URL. It uses the guarded HTTP client and requests a 180-second refresh when
the card is created. The feed is a JSON object with `updated` (an ISO timestamp with
timezone), optional `plan`, and a nonempty `windows` array. Each window carries
`name`, numeric `used_pct`, `resets_at_label`, and optional `resets_in_label`.

The face keeps the producer's neutral bars, plan badge and reset labels, using the
shared Inter type, 8px grid and 24px margins. Two rows per page keep percentages
readable; tap for further windows, up to twelve across six views. A single window
is centered. Percentages clamp to 0–100; missing percentages display “—”. Missing
reset and update times are explicit. Relative reset labels are identified as being
from the update, rather than presented as a running countdown.

An `updated` time older than one hour shows an amber **Stale** sentence even when
the HTTP fetch succeeds. Empty, malformed or oversized-window feeds throw a
`TransientError`; invalid/non-HTTPS URLs throw a `ConfigurationError`. Both reach
the existing `face_status` path and leave the stored frame intact. HTTP failures
and egress refusals retain the guard's own typed sentences.

The two `test/claude-usage*.captured.json` fixtures are actual responses from
2026-10-10 (06:35:15Z and 06:45:15Z feed timestamps). Ten golden/dump cases include
those captures and explicitly mutated edge cases. Run `bun run dump out/` and
inspect `out/claude-limits--*.desk.png` alongside the full-size PNGs.

## Rules that are not obvious

- **Flat fills and strokes only.** No gradients, filters, images or `<defs>`. RLE565
  compresses a flat frame to ~10 KB and expands a gradient to ~330 KB. A test enforces it.
- **Colour lives in a module on black, never full-bleed**, and never saturated across an
  area: the panel is emissive, and a field of colour is a desk lamp.
- **Measure, never guess.** Text is fitted before it is drawn, because a fixed panel has
  no scrollbar. A tracked run is measured with `trackedWidth`/`fitTracked`.
- **`fixed()`, never `toFixed()`**, for any number that reaches the document. The goldens
  are byte-exact and came from Rust, whose rounding `toFixed` does not reproduce.
- **Nothing but the PNG on stdout** in `render`. Diagnostics go to stderr; its last line
  is what the server logs.
- **The weather face is settled.** The owner approved it on 2026-09-17.
