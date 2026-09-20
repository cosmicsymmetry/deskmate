# Server-rendered data cards

## What these are

A weather, Hacker News, RSS or token card is an **image source the server pushes to
itself**. There is no new card kind: the owner creates one ordinary `picture` card
pointing at a source id, and a task inside the server asks the faces package for a frame
and hands it to the same `ImageSourceStore::accept` an external producer's PNG arrives
through.

So everything downstream is the picture-card path that already works on hardware: the
durable asset, the keep-set, the device notify, the staleness inference, the GC. What is
new is only the producer.

`docs/images/producer-guide.md` describes the external-producer route, which is
unchanged and remains the right answer for anything that needs credentials or a machine
this server is not.

## Who does what (since 2026-09-20)

| | Where | Language |
|---|---|---|
| Specs on disk, the browser's settings contract, validation, the refresh schedule, accepting the frame | `crates/server/src/data_cards.rs` | Rust |
| What a face fetches and how it is drawn | `companion/faces/` | TypeScript, on Bun |

The server runs the faces package as a subprocess and speaks two verbs to it
(`crates/server/src/data_cards/faces_package.rs`, `companion/faces/src/main.ts`):

- `describe` prints the catalog -- kinds, labels, fields. The browser's add menu and
  settings form are built from it, so **the server names no face kind**.
- `render` reads `{"kind", "settings"}` on stdin and writes one 448x368 PNG to stdout,
  which goes through `canonical_frame_from_png` exactly as a producer's POST does. Exit
  0 is a frame; exit 2 means the owner must change a setting; anything else is
  transient. Either failure keeps the stored frame without renewing its freshness.

The faces were Rust until 2026-09-20 (`crates/server/src/faces/`, `crates/providers`).
They moved on the owner's direction, for maintenance: a face is design work plus an HTTP
call, and neither is what Rust is for. What made the move safe is that
`@resvg/resvg-js` is the same `resvg` engine behind a binding -- all 19 existing cases
were ported on the condition that their SVG be **byte-identical** to the Rust output,
and the Rust output is what `companion/faces/test/golden/` holds. The owner-approved
weather design moved without a pixel changing.

## Adding a face

1. Write `companion/faces/src/faces/<kind>.ts` exporting a `FaceDefinition`: the fields
   the browser should offer, and `render(settings, now)` resolving to an SVG string.
   Build it from `src/kit/` -- `Canvas`, `wrap`, `fit`, `fitTracked`, `fitSize` -- and
   fetch only through `kit/http.ts`.
2. Add it to `src/registry.ts` and its cases to `src/cases.ts`. Use inputs shaped like
   real responses: every fixture title being short is how an overflow survives until a
   live one.
3. `bun run dump out/` and LOOK at the PNGs, including the `.desk.png` ones -- 0.4x is
   roughly the panel's physical size on a desktop display, and is the honest test of
   whether type is legible from a chair. Then `bun run dump --update` to write the
   goldens.
4. `deploy.sh --faces-only`. No Rust build, no restart: the package is read per
   refresh and the catalog is re-read every minute, so the face appears in the add menu
   on its own.

No card kind, no schema change, no wire change, no firmware change, no binary redeploy.
A face that needs any of those is not a face.

## Setting one up

1. In the browser companion's add menu, select a server face: Weather, Hacker News,
   RSS feed or Token price. The server creates its image source and persists a blank
   face spec.
2. Fill in the fields supplied by the server's descriptor, such as the location,
   feed URL and title, or coin ID and currency.
3. Save the card/configuration. The card remains an ordinary picture card; its
   completed face settings start the server refresher without a restart. A face whose
   fields all have defaults -- Hacker News -- starts refreshing the moment it is
   created. The window's preview shows the frame the source last drew.

## Advanced manual settings

The server persists specs in `data-cards.json` inside `DESKMATE_CONFIG_DIR`;
`DESKMATE_DATA_CARDS` overrides that path. Settings such as `refresh_seconds` and
an optional token `api_key` are deliberately absent from the browser descriptors.

To edit an existing persisted spec, stop the server, back up the file, and change
only the intended entry, retaining every other entry. Then restart the server:
external edits are read at startup, not hot-reloaded, and browser settings updates
rewrite the server's retained spec set. The following shows the nested shape;
do not replace an existing multi-entry file with this example.

   ```json
   [
     {
       "source_id": "src_9f3a...",
       "refresh_seconds": 900,
       "face": { "kind": "weather", "location": "Dubai", "units": "metric" }
     },
     {
       "source_id": "src_77e0...",
       "refresh_seconds": 900,
       "face": { "kind": "hackernews", "list": "top" }
     },
     {
       "source_id": "src_2b71...",
       "refresh_seconds": 900,
       "face": {
         "kind": "rss",
         "url": "https://github.com/oven-sh/bun/releases.atom",
         "title": "Bun releases"
       }
     },
     {
       "source_id": "src_4c08...",
       "refresh_seconds": 300,
       "face": { "kind": "token", "coin_id": "solana", "currency": "usd" }
     }
   ]
   ```

   The face config is a nested object rather than flattened alongside
   `source_id`, and that is load-bearing: serde's `deny_unknown_fields` and
   `flatten` do not work together, and the outer object stays strict -- so
   `"refresh_second"` is a refused start, not a silent default.

A missing file means no server-rendered cards, which is the ordinary case and not an
error. A **malformed** file fails the start, deliberately: a server that came up with
silently missing cards would present as "the panel stopped updating" with nothing in the
log.

**The `face` object itself is open-ended**, because its kinds are the faces package's to
define, not the server's. The shape on disk is unchanged from the Rust faces, so a file
written before 2026-09-20 loads without a migration. What changed is who catches a typo
inside it: `"unit"` for `"units"` is no longer refused at startup -- the face simply
runs on its default -- and a kind the package does not draw is kept but not refreshed,
with a warning, rather than refused.

For a manually created source, the admin bearer remains supported:

```sh
curl -sX POST https://deskmate.rodi.one/v1/images \
  -H "Authorization: Bearer $DESKMATE_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"name": "Weather"}'
```

Use the returned id for the spec's `source_id` and the picture card's source.
The server writes frames directly to its store, so it does not need the returned
producer token.

## The fields

Every field below `face` sits inside it; `source_id` and `refresh_seconds` sit
outside.

| Field | Applies to | Meaning |
| --- | --- | --- |
| `source_id` | all | The minted image source. The picture card names the same id. |
| `refresh_seconds` | all | Clamped to 60 s…6 h. Default 900. |
| `location` | weather | A place name, geocoded by Open-Meteo. The geocoder's own display name is what the face shows, so the panel says what the forecast is actually for. |
| `units` | weather | `metric` (default) or `imperial`. |
| `list` | hackernews | `top` (the front page, default), `best`, `new`, `ask` or `show`. |
| `url` | rss | The feed. RSS 2.0, Atom and RDF all parse. |
| `title` | rss | The eyebrow. Feeds name themselves inconsistently and often at length, so this is the owner's words. |
| `coin_id` | token | CoinGecko's **id**, e.g. `solana` — not the ticker. Lowercase letters, digits and hyphens only. |
| `currency` | token | Quote currency, default `usd`. |
| `api_key` | token | Optional CoinGecko demo key. |

## Where the data comes from

| Card | Endpoint | Key needed |
| --- | --- | --- |
| weather | `api.open-meteo.com` + `geocoding-api.open-meteo.com` | No |
| hackernews | `hacker-news.firebaseio.com/v0`: the ranked id list, then one request per story shown | No |
| rss | whatever `url` names | No |
| token | `api.coingecko.com/api/v3/coins/markets` and `/market_chart` | No, but the free tier is rate-limited |

The token card spends **two** requests per refresh, and only the first is required. A
rate-limited or slow `market_chart` leaves a face with a price and no sparkline, which is
a worse face but a true one; refusing the whole refresh would replace a correct price
with a stale badge because a decoration was unavailable. Keep `refresh_seconds` at 300 or
above on the free tier.

## Who makes the outbound requests

Not the server. Since 2026-09-20 every fetch happens inside the faces subprocess, and
the server's own egress is back to the one host it was reduced to on 2026-09-13:
`oauth2.googleapis.com`, POST only, through `crates/server/src/egress.rs`. The
unrestricted GET path that file grew for the Rust faces is gone again, for the reason it
was removed the first time -- an unguarded `fetch` with no allowlist in front of it is an
invariant left open to the next caller.

The subprocess has its own guard in `companion/faces/src/kit/http.ts`, restating the
properties of `egress.rs` that matter to a fetcher of owner-typed URLs: http(s) only;
private, loopback, link-local, CGNAT and metadata addresses refused on **every redirect
hop** (a public feed can redirect inward); a 1 MB body cap; a wall-clock budget. And it
**pins**: a name is resolved once, every address it returned must be public, and the
request dials one of those addresses as an IP literal with the name in `Host` and as
the TLS server name -- so a DNS server that answers the check and the connection
differently (rebinding) gains nothing, and the certificate is still checked against the
name. Both halves of that were verified against live hosts before being relied on: the
right address with the right name connects, and either one swapped is refused.

It is a smaller guard than `egress.rs` -- one deny list written by hand rather than the
full special-use registry -- and it is the right size for what it protects: this host is
a homelab VM with neighbours, and a feed URL is only as trustworthy as the feed's DNS.

**A denied address is reported as a configuration error, not an I/O error**, because
nothing about it changes on a retry: the message sends the owner to the URL they typed
rather than to their network.

The subprocess gets a **cleared environment**. The server's holds
`DESKMATE_ADMIN_TOKEN`, and a process whose job is to fetch arbitrary URLs has no
business with it.

## Designing against the link, not just the panel

The faces are flat fills and strokes with no gradients, and that is a delivery decision
as much as a visual one. RLE565 compresses a flat-colour frame to ~10 KB and *expands*
high-entropy input 2×, so a gradient sky would cost the full ~330 KB raw on every change.
`Canvas::finish` emits no `<defs>` at all for this reason.

An identical face is a no-op: the store compares digests and moves no bytes, while still
counting the push as liveness. So a weather card refreshing every 15 minutes transfers a
frame only when the picture actually changes.

## Reviewing a face

```sh
cd companion/faces && bun run dump out/
```

That writes one PNG per case from `src/cases.ts`, plus a `.desk.png` at roughly the
panel's physical size. The PNGs are not committed: a golden image of anti-aliased type
fails on a `resvg` upgrade and says nothing about whether the design is good.

What IS committed is each case's **SVG**, in `test/golden/`. That is this package's own
deterministic output, so it moves only when a face's geometry does -- which is the thing
worth being told about -- and a deliberate change is `bun run dump --update`, reviewed
as a diff. Beside that, the suite asserts what a face can be wrong about without anybody
noticing: every case rasterizes to exactly 448x368, **no text is positioned outside the
canvas**, and every face is flat fills and strokes.

To exercise a real API, run the package the way the server does:

```sh
echo '{"kind":"hackernews","settings":{"list":"top"}}' | bun run src/main.ts render > /tmp/hn.png
```

## What live responses have already caught

Worth keeping in mind before adding a face.

- **Text measured without its tracking overflows.** `textWidth` lays a run out through
  the engine that draws it, but with no `letter-spacing`, and every eyebrow is tracked
  out 1.4px for legibility at 15px. "DUBAI, UNITED ARAB EMIRATES" is 27 characters, so
  the drawn run was ~38px wider than the measured one and ran straight through the
  high/low beside it. Use `trackedWidth` and `fitTracked` wherever a run is tracked. No
  fixture caught it because every fixture place name was short.
- **A default HTTP client sends no useful `User-Agent`**, and a Cloudflare-fronted API
  answers that with 403 before it looks at the path. That is how the CoinGecko fetch
  failed while `curl` to the same URL worked. `kit/http.ts`'s `USER_AGENT` identifies
  the faces on every request; `tools/picture-producers/claude_limits_png.py` carries the
  same note for urllib.
- **A layout planner's fallback can outbid its own main case.** The Hacker News face
  scored a lone 56px lead above one index row under a 40px lead, so a real front page
  with a short top story lost its index entirely. Every fixture had a long lead. The
  lone lead now scores below any arrangement that has an index.
