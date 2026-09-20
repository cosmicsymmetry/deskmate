# Deskmate faces

The server-rendered card faces: weather, Hacker News, RSS, token. Each one fetches its
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
