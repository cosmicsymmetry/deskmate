# Server-rendered data cards

## What these are

A weather, RSS or token card is an **image source the server pushes to itself**. There is
no new card kind: the owner creates one ordinary `picture` card pointing at a source id,
and a task inside the server fetches the data, draws the face, and hands the frame to the
same `ImageSourceStore::accept` an external producer's PNG arrives through.

So everything downstream is the picture-card path that already works on hardware: the
durable asset, the keep-set, the device notify, the staleness inference, the GC. What is
new is only the producer.

`docs/images/producer-guide.md` describes the external-producer route, which is
unchanged and remains the right answer for anything this server does not know how to
draw.

## Setting one up

1. In the browser companion's add menu, select a server face: Weather, RSS feed or
   Token price. The server creates its image source and persists a blank face spec.
2. Fill in the fields supplied by the server's descriptor, such as the location,
   feed URL and title, or coin ID and currency.
3. Save the card/configuration. The card remains an ordinary picture card; its
   completed face settings start the server refresher without a restart.

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
       "source_id": "src_2b71...",
       "refresh_seconds": 900,
       "face": {
         "kind": "rss",
         "url": "https://news.ycombinator.com/rss",
         "title": "Hacker News"
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
   `flatten` do not work together, so flattening would have meant giving up the
   typo protection described below. One level of braces buys it back.

A missing file means no server-rendered cards, which is the ordinary case and not an
error. A **malformed** file fails the start, deliberately: a server that came up with
silently missing cards would present as "the panel stopped updating" with nothing in the
log. Unknown fields are refused too, so `"unit"` does not quietly become the default
`"units"`.

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
| `url` | rss | The feed. RSS 2.0 and Atom both parse. |
| `title` | rss | The eyebrow. Feeds name themselves inconsistently and often at length, so this is the owner's words. |
| `coin_id` | token | CoinGecko's **id**, e.g. `solana` — not the ticker. Lowercase letters, digits and hyphens only. |
| `currency` | token | Quote currency, default `usd`. |
| `api_key` | token | Optional CoinGecko demo key. |

## Where the data comes from

| Card | Endpoint | Key needed |
| --- | --- | --- |
| weather | `api.open-meteo.com` + `geocoding-api.open-meteo.com` | No |
| rss | whatever `url` names | No |
| token | `api.coingecko.com/api/v3/coins/markets` and `/market_chart` | No, but the free tier is rate-limited |

The token card spends **two** requests per refresh, and only the first is required. A
rate-limited or slow `market_chart` leaves a face with a price and no sparkline, which is
a worse face but a true one; refusing the whole refresh would replace a correct price
with a stale badge because a decoration was unavailable. Keep `refresh_seconds` at 300 or
above on the free tier.

## The server makes outbound HTTP again

It did not between `e137294` (2026-09-11) and this change. `CLAUDE.md`'s current-state
section says so and carries the two traps this cost; the config contract in
`docs/config/v10.md` is unaffected, because no card kind and no schema field moved.

Every request goes through `crates/server/src/egress.rs`, the SSRF guard the plugin
system used: scheme checks, the RFC1918/loopback/link-local/metadata deny list,
resolve-then-pin against DNS rebinding, per-hop re-validation across redirects, a body
cap and a wall-clock budget. `EgressHttpClient` is the only HTTP client the provider
layer is given, and it is the seam that makes that true rather than intended.

**A denied address is reported as a configuration error, not an I/O error**, because
nothing about it changes on a retry: the card's message sends the owner to the URL they
typed rather than to their network.

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
DESKMATE_FACE_DUMP=/tmp/faces cargo test -p server faces::golden -- --nocapture
```

That writes one PNG per case from `crates/server/src/faces/cases.rs`. The PNGs are
deliberately not committed: a golden image of anti-aliased type fails on a `resvg`
upgrade and says nothing about whether the design is good, and this repository has
already paid for a parity gate that kept dead code alive to have something to compare
against.

What CI asserts instead is what a face can be wrong about without anybody noticing:
every case parses and rasterizes, every frame is the exact size the asset path accepts,
and **no text run crosses the canvas edge** — measured with its tracking, which is the
check a live response earned (see below).

To exercise the real APIs:

```sh
DESKMATE_LIVE_FACES=1 DESKMATE_FACE_DUMP=/tmp/faces-live \
  cargo test -p server faces::live -- --nocapture --test-threads 1
```

Off by default, because CI must not depend on CoinGecko's rate limit or somebody else's
RSS host being up. It is the only test that can prove the live API shapes still parse
into what the faces expect — a fixture cannot, because the fixture was written from the
same reading of the API as the parser.

## Two things the live test has already caught

Both are worth keeping in mind before adding a face.

- **Text measured without its tracking overflows.** `face_render::text_width` lays a run
  out through the same engine that draws it, but with no `letter-spacing`, and every
  eyebrow is tracked out 1.4px for legibility at 15px. "DUBAI, UNITED ARAB EMIRATES" is
  27 characters, so the drawn run was ~38px wider than the measured one and ran straight
  through the high/low beside it. Use `svg::tracked_width` and `svg::fit_tracked`
  wherever a run is tracked. No fixture caught it because every fixture place name was
  short.
- **`reqwest` sends no `User-Agent` unless one is set**, and a Cloudflare-fronted API
  answers that with 403 before it looks at the path. That is how the CoinGecko fetch
  failed while `curl` to the same URL worked. `egress::USER_AGENT` now identifies the
  server on every request; `tools/picture-producers/claude_limits_png.py` carries the
  same note for urllib.
