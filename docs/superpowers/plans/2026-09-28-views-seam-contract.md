# The views seam: the contract between the faces package and the server

Written 2026-09-28, while implementing Tasks A and B of
`docs/superpowers/specs/2026-09-28-deskmate-frames-live-in-psram-design.md`. The two
halves are built against this document so they meet without either guessing.

**Why this exists:** a tap costs about three seconds today — the face fetches and draws
(~1.75 s for some faces), then 33 chunks go over the tunnel (~1.3 s), then the commit
(~0.08 s since the pool landed). If the frame a tap wants is already on the device, a tap
is one `PushScene`: roughly one round trip. That requires the face to say what views it
has, the server to stage each of them, and a tap to resolve to a staged digest without
rendering anything.

## Deviations from the spec, and why

1. **`views(settings, state?)`, not `views(settings)`.** `rss` and `hackernews` page over
   fetched entries, so how many views exist depends on what was fetched — which lives in
   state, not settings. With no state a face declares only its resting view and gains the
   rest after its first fetch. That is also when staging first has anything to stage.
2. **`onTap(settings, state, event)`, not `onTap(state, event, views)`.** The face owns its
   view list, so handing it back its own views is redundant and invites the two to
   disagree. `onTap` returns the selected view **and** the state to store, because the
   state is where "which view is showing" and "when was it tapped" live — `weather`
   reverts to `now` ten minutes after a tap, and it can only know that from state.
3. **`render` is told which view to draw** (`context.view`), instead of deriving it from
   the tap count. Deriving was the only option while render was the sole entry point.

## The TypeScript contract (`faces/src/face.ts`)

```ts
/** A view a face can draw. Stable, opaque to the server, unique within a face. */
export type ViewId = string;

export interface RenderContext {
  state?: unknown;
  event?: { taps: number; point: { x: number; y: number } | null };
  /** Which view to draw. Absent means the resting view, which is views()[0]. */
  view?: ViewId;
}

export interface FaceDefinition {
  kind: string;
  label: string;
  fields: FieldSpec[];
  tap?: string;
  /**
   * The views this face offers, in priority order; the first is the resting view.
   * Pure: no fetch, no draw. A face without it has exactly one view and keeps
   * today's behaviour.
   */
  views?(settings: Settings, state?: unknown): ViewId[];
  /**
   * Which view a tap selects, and the state to store. Pure: no fetch, no draw.
   * A face with views() must have this, and the reverse.
   */
  onTap?(settings: Settings, state: unknown, event: TapEvent): { view: ViewId; state?: unknown };
  render(settings: Settings, now: Date, context?: RenderContext): Promise<RenderResult> | RenderResult;
}
```

**Purity is the point.** `onTap` and `views` must not fetch and must not draw: the server
calls them on the tap path, where the whole saving is that nothing renders. The faces
suite asserts it by handing them a `fetch` that throws.

## The subprocess verbs (`faces/src/main.ts`)

`describe` and `render` are unchanged in shape. Two are added, both pure, both cheap:

| verb | stdin | stdout |
|---|---|---|
| `views` | `{kind, settings, state?}` | `{views: [ViewId, ...]}` |
| `tap` | `{kind, settings, state?, event: {taps, point}}` | `{view: ViewId, state?: unknown}` |

- A face with no `views()` answers `views` with its single resting view, spelled `""`.
- A face with no `onTap()` answers `tap` with `{view: ""}` — the resting view, unchanged.
- `render` gains an optional `view` in its request, passed through as `context.view`.
- Exit codes keep their meaning: 0 fine, 2 the owner must change a setting, anything else
  transient.

## The resting view is `""`

Not `"default"`, not the first id — the empty string, so that a face that declares no
views and a face's resting frame are the same path on disk and the same key in the store.
`""` is how the server names the frame it already has, which is what keeps every existing
producer's POST and every frame already on the live VM untouched.

## The server side (`crates/server/`)

- **`image_sources.rs`**: a source holds several frames keyed by `ViewId`, and remembers
  which one is selected. **No store schema bump**: `load_frame` probes a file and
  re-hashes it, and `image-sources.json` never records a frame at all, so extra views are
  `image-frames/<id>--<view>.bin` discovered by listing, with `<id>.bin` unchanged as the
  resting view. A v1 store with one file loads as one resting view.
- **`data_cards/worker.rs`**: a refresh renders every declared view within the card's
  share and stages each.
- **`data_cards.rs`**: a tap calls the `tap` verb, selects the returned view, and notifies
  the runtime with that view's digest. No render, no transfer.
- **The budget**: each picture card's share is `floor(15 / picture_cards)`, minimum one,
  leftovers to the cards that declared the most views, in loop order. Views past a card's
  share are rendered on the tap that asks for them, as today.
- **Everything resolves through `AccountSpace`** (`state.space_for_device`,
  `state.account_space`), never a global store — Track A made the image sources, the data
  cards and the device configs per-account, and phase 2 must not reintroduce a global.
- **The digest ceiling is the real bound**: `MAX_ASSET_DIGESTS` is 32 and `asset_sync`
  refuses a larger desired set. With several frames per source, the staged total is what
  must fit, so `image_sources.rs`'s
  `const _: () = assert!(MAX_IMAGE_SOURCES <= protocol::MAX_ASSET_DIGESTS)` is no longer
  the right invariant.

## What must not move

`faces/test/golden/` pins 19 SVGs byte for byte, and the weather design is owner-approved.
This refactor moves *where* the view is decided, not what any view draws: every golden
must stay byte-identical, and `bun run dump --update` is not to be run.
