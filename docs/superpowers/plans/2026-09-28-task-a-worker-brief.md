# Worker brief: Task A, the faces views/onTap contract

You are implementing **only** the faces package half. Another worker is doing the server
half at the same time.

## Hard boundaries

- **Touch only files under `companion/faces/`.** Nothing under `companion/crates/`,
  `companion/apps/`, `firmware/` or `docs/`. If you believe a file outside
  `companion/faces/` must change, stop and say so in your final message instead.
- **Do not run any `git` command.** Do not commit, stage, stash, branch or check out.
  Leave your work as uncommitted changes in the working tree.
- **Do not run `bun run dump --update`.** The goldens must not move; see below.

## The contract

Read `docs/superpowers/plans/2026-09-28-views-seam-contract.md` first. It is the agreement
between your half and the server half, and it is not negotiable — if something in it cannot
work, stop and say why rather than changing the shape.

Implement, in `companion/faces/`:

1. `src/face.ts`: `ViewId`, `RenderContext.view`, and the optional `views()` / `onTap()`
   members exactly as the contract spells them.
2. `src/main.ts`: the two new verbs, `views` and `tap`, with the stdin/stdout shapes the
   contract gives, and `view` passed through on a `render` request. Keep the exit-code
   meanings (0 fine, 2 configuration, anything else transient).
3. The four faces in `src/faces/` declare their views and move their tap arithmetic out of
   `render` into `onTap`:
   - `weather.ts`: two views, resting `""` (current conditions) and the coming days. Today
     `renderWeatherResult` flips on an odd tap count and reverts to `now` more than
     `TEMPORARY_VIEW_MS` after a tap on a *scheduled* refresh. That revert rule stays, and
     it stays in the same place relative to state — `onTap` decides the view a tap selects,
     the scheduled-refresh revert remains render's business.
   - `hackernews.ts` and `rss.ts`: page over fetched entries. The page count comes from
     state, which is why `views()` takes state; with no state, declare only `""`.
   - `token.ts`: `""` plus whatever `TAPPABLE_CHARTS` offers beyond the first; a card
     configured `chart: "none"` declares only `""`.
4. `render` draws the view it is told to draw (`context.view`), defaulting to the resting
   view when absent.

## Purity is the whole point

`views()` and `onTap()` run on the tap path, where the entire saving is that nothing
fetches and nothing rasterises. Assert it: a test that calls each of them with
`globalThis.fetch` replaced by a function that throws, and with no network available.

## What must not move

`faces/test/golden/` pins 19 SVG documents byte for byte and the weather design is
owner-approved. You are moving *where the view is decided*, not what any view draws. Every
golden must remain byte-identical. `bun run dump --update` is forbidden. If a golden moves,
your refactor changed drawing behaviour and is wrong.

## Gates you must leave green, from `companion/faces/`

```
bun test
bun run check
bun run lint
bun run format:check
```

Run all four. `bun run check` type-checks the tests as well as the source, so a test still
passing a prop the code dropped is a type error rather than a silent pass.

## Report back

In your final message: what you changed, the gate output, and anything in the contract that
fought you. If you had to deviate from the contract, say exactly where and why — the server
half is being written against it and a silent deviation breaks the seam.
