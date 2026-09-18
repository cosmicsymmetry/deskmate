# Server-rendered data cards

Status: implemented on `feat/server-side-cards`. Owner direction, 2026-09-12, verbatim:

> Let's create a few new cards in a separate worktree. Fully rendered.
>
> I want to have rss feed card, beatifully designed. A crad to display token prices,
> Solana for instance. And a rich and sleek weather card. Take inspiration from pixel
> weather app.

and, after a first proposal built the three as external picture-card producers:

> No, it should be server-side. Proceed

This is the "later on" the 2026-09-10 brief promised. That brief's own words, when the
device-rendered data cards were deleted:

> we can have way more flexible, for instance, weather card rendered on the server rather
> than on the device ... Later on we'll recreate that functionality as server-side cards.

## What this reverses, and what it does not

`e137294` (2026-09-11) removed manifest-based plugins and stated two consequences. One of
them is now false and one is still true.

- **False:** "The server now makes no outbound HTTP at all." It does for these three
  cards, with every provider request routed through the shared SSRF guard.
- **Still true:** "a server-side card can no longer change between pushes." These faces
  are frozen frames. Clock and pomodoro remain the only faces that tick.

What is **not** reversed is the thing the owner actually rejected. The plugin system was
~11,200 lines of TOML manifest compiler, expression language and registry on top of
~5,100 lines of fetch/guard/render plumbing. Only the plumbing comes back. There is no
manifest, no expression language, no plugin catalog, and no dropdown that turns a card
into a different card. The three faces are native Rust.

## Architecture

A server-rendered card is **an image source the server pushes to itself.**

```
DataCardSpec (data-cards.json)
  -> provider (RSS | Open-Meteo | CoinGecko), fetched via EgressHttpClient
  -> ProviderSnapshot<T>            (last-good state survives a failed fetch)
  -> faces::adapt                   (tenths -> degrees, dates -> ages, marks)
  -> faces::{rss,token,weather}     (view model -> SVG string, pure)
  -> face_render::frame_from_svg    (resvg -> RGB565 + SHA-256)
  -> ImageSourceStore::accept       <- the exact call a producer's PNG arrives through
  -> [existing] durable asset, keep-set, device notify, staleness, GC
```

Everything below `accept` is unchanged and already proven on hardware. **This change adds
a producer, not a delivery path.**

### Three decisions worth recording

**1. Rastered, not scene-native.** `protocol::MAX_SCENE_NODES` is 24 — a fixed
`scene_node_t nodes[24]` array in `firmware/main/core/scene_model.h`, so raising it is a
firmware change owing an on-board OTA re-verification. A `SceneLine` holds at most 8
points, and only the Caption (18px) and Body (28px) baked tiers contain letters at all;
Display and Hero are digits-only subsets of `0-9 : - % °`. A layered weather illustration
with a six-column hourly strip is not expressible in that budget, and a 24-hour sparkline
would arrive as seven straight segments.

This does **not** add a second renderer, which `CLAUDE.md` forbids. There is still exactly
one *scene* renderer; a picture card already delivers its face as a durable RGB565 asset
behind a one-`Image`-node scene (`frame_face_scene`), and these cards use that.

**2. Not new card kinds.** Schema stays v10. Each card is a `picture` card plus an entry
in a server-side file. Native kinds would put these in the companion SPA with their own
editors, and that is worth doing — but it is a schema bump plus TS contract plus editors
plus migration, and it changes none of the fetch, the faces, the frames or the delivery.
Putting fields in the config document that no window can author and no migration can
repair is how this project came to delete two card families.

**3. Flat fills only, no gradients.** RLE565 compresses a flat-colour frame to ~10 KB and
*expands* high-entropy input 2×, so a Pixel-Weather-style gradient sky would cost the full
~330 KB raw on every change. `Canvas::finish` emits no `<defs>` for this reason. The
adaptation shows in the weather face: deep flat condition grounds in a rounded hero module
on black, rather than a full-bleed saturated field — which `DESIGN.md` rules out anyway,
since `--stage` is black in both schemes because an unlit AMOLED pixel emits nothing.

## The faces

Each is a pure function from a view model to an SVG string, so a layout assertion needs no
network and no golden image.

- **RSS.** One lead story at 31px semibold over up to three lines, marked with a `--good`
  accent bar (the role `DESIGN.md` defines as "fresh"), then a quiet index of up to three
  followers with relative ages. The layout is **computed, not fixed**: the lead block is
  measured first and the remaining height is divided among however many rows fit at a
  readable size, so a one-line lead does not leave a hole.
- **Token.** Two-size price with the currency mark cap-top aligned and the decimal
  separator at the *integer's* size — a 42%-size period beside a 112px numeral is
  invisible and "$101" + small "96" reads as 10,196. A sub-unit price is set at one size,
  because shrinking the fraction there sets the one meaningless glyph large. Full-resolution
  sparkline with a `--good`/`--bad` area fill and a marker on "now", plus a 24h range track
  showing where the current price sits.
- **Weather.** A condition-coloured rounded hero (place and high/low on one eyebrow line,
  the reading at up to 112px, the summary below, a geometric illustration to the right)
  over a six-column hourly strip on a surface module. Eleven conditions, each with its own
  deep ground, bright accent and glyph built from circles, ellipses and paths parameterized
  by one scale — so the same code draws the 52px hero sun and the 15px strip sun.

## Security

Provider URL validation rejects invalid syntax, non-HTTP(S) schemes, missing hosts and
embedded credentials. The server constructs the weather, RSS and token providers with
`EgressHttpClient`, which implements their `HttpClient` trait and routes every GET through
`crates/server/src/egress.rs`. That guard adds address-level SSRF denial, including the
RFC1918/loopback/link-local/`169.254.169.254` deny list, resolve-then-pin against DNS
rebinding, per-hop re-validation across redirects, a body cap and a wall-clock budget.

Feed and ticker text reaches an SVG document, so `faces::svg::escape` handles all five
predefined entities and drops the control characters XML 1.0 cannot carry. A hostile
headline degrades to ugly text, never to a refused parse or a blank panel. `usvg` forbids
scripts, event handlers and network references outright; a coin id is restricted to
`[a-z0-9-]` before it reaches a URL path rather than escaped after.

One addition: `egress::USER_AGENT`. `reqwest` sends no `User-Agent` unless one is set and
a Cloudflare-fronted API answers that with 403 before reading the path, which is how the
CoinGecko fetch failed while `curl` to the same URL worked.

## Verification

- `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo fmt --all --check`
  clean.
- 55 face tests: escaping, wrapping, tracking, fitting, every WMO mapping, every condition
  glyph at both scales, sub-zero and five-figure and sub-cent numerals, empty and undated
  feeds, hostile input for all three faces.
- Structural gate over every reviewable case: it parses, it rasterizes, the frame is
  exactly the size the asset path accepts, and **no text run crosses the canvas edge**
  measured *with its tracking*.
- 24 provider tests, including the two-request split, last-good-on-failure, path-escaping
  coin ids refused before any request, and an API key that never reaches an error message.
- `faces::live` (off by default, `DESKMATE_LIVE_FACES=1`) drives the real APIs end to end
  through the guard. It is the only test that can prove the live shapes still parse into
  what the faces expect, and it has already earned its keep twice — see the two traps now
  in `CLAUDE.md`.
- **Not verified on hardware.** No frame from these faces has reached `dev-0005`, which is
  still flashed with protocol v1 and therefore unreachable. See below.

## What this owes

- **A frame on the physical panel.** The delivery path is the picture card's and is proven,
  but "proven for a PNG a producer pushed" is not "observed for a frame the server drew".
  Owed once the board is flashed with `v2.1.0-proto2`: one weather, one token and one RSS
  face on `dev-0005`, at both mountings, plus the RLE565 transfer size for each so the
  flat-fill argument above is measured rather than asserted.
- **Native card kinds (a prospective schema v11).** Editors in the companion SPA for the location,
  the feed URL and the coin id, replacing the spec file. Strictly additive on top of this.
- The spec file is the honest interim: it says where the authority currently sits.
