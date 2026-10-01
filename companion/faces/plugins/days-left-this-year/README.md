# Days Left This Year

A quiet calendar countdown for the 448×368 Deskmate panel: a large remaining-days
number, the current year, and a choice of three year-progress faces. No data
services, network requests, secrets, stored state, or tap behavior.

**Id:** `days-left-this-year` · **Version:** `1.1.0`

## Face setting

Choose **Face** in the card's source settings. The choice saves immediately through
the existing settings control:

- **Progress bar**: the original large count and continuous bar; the default for
  new cards and existing cards without a face setting.
- **Squares**: one small square per calendar day.
- **Dots**: one small circle per calendar day.

Both grids contain exactly 365 marks (366 in a leap year), read left to right and
then top to bottom in 25 columns. Bright marks are completed days; dim marks are
days remaining, including today. The final row is intentionally shorter. All faces
use the same count, date and progress calculation. Unknown face values fall back to
the original progress bar.

## What the count means

Today is included. January 1 shows **365** (or **366** in a leap year); December 31
shows **1**. At the next calendar date in a new year the count resets. The face says
“including today” and uses “day” for the final day.

Progress is **completed calendar days / days in that year**. It starts at 0.0% on
January 1 and reaches 99.7% on December 31, never 100% while that year is still in
progress. The label rounds to one decimal place; the bar uses the unrounded ratio.
Gregorian leap rules include the century exceptions: 2000 is leap, 2100 is not.
Calendar arithmetic avoids assuming every day lasts 24 hours.

## Timezone and freshness limitations

The plugin uses `context.now.local`, computed by the host in the owner's configured
IANA timezone. Config v10 stores that preference **per device**, while a source has
one shared image per account. The server therefore uses the first active device
whose saved picture card displays the source, ordered by device id. Before a new
source is attached, it uses the first active device's saved timezone; without one,
it uses UTC. Accounts never borrow each other's settings. If devices sharing a
source need different dates, create separate sources for them.

The requested cadence is **900 seconds (15 minutes)** for newly created cards;
existing cards retain their stored cadence. Successful faces also refresh after a
local date change: the server checks once a minute, so rollover normally reaches
the frame store within **roughly 60 seconds plus rendering time**, followed by
delivery. Saved timezone changes are picked up by the same check. This is not an
exact-midnight wakeup or on-device ticking. The check applies to all successful
server faces because contract v1 has no date-sensitive flag, adding at most one
extra scheduled render per local day when the timezone stays unchanged.

Failures keep the previous frame and the existing retry policy, so an unavailable
server, failed render or failed delivery can leave it stale longer. Missing or
invalid host dates fail the render. This plugin calls no API.

## Reproduce and inspect

From `companion/faces/`:

```sh
bun install --frozen-lockfile
bun run plugin:check days-left-this-year
bun run src/main.ts describe
printf '%s' '{"kind":"days-left-this-year","settings":{}}' \
  | bun run src/main.ts render > /tmp/days-left-this-year.json
bun run plugins/days-left-this-year/preview.ts
bun test
bun run check
bun run lint
bun run format:check
```

The CLI render returns a JSON envelope whose `png` field is base64. The preview
script discovers the shipped plugin and calls the actual `src/main.ts`
`renderRequest` entrypoint with fixed UTC instants. It writes three full-size PNGs
for each of the three faces and their 40% reductions under `previews/`; the small
images are scaled from those same PNGs. `preview.ts` is author tooling, not sandboxed
plugin code.

| Fixture (UTC) | 448×368 | 40% desk scale |
|---|---|---|
| 2026-10-01 12:00:00 | [October](previews/october.png) | [October](previews/october-desk.png) |
| 2026-12-31 23:59:59 | [Last day](previews/last-day.png) | [Last day](previews/last-day-desk.png) |
| 2028-01-01 00:00:00 | [Leap New Year](previews/leap-new-year.png) | [Leap New Year](previews/leap-new-year-desk.png) |

![Days Left This Year on October 1, 2026](previews/october.png)

| Alternative face | October 1 | Last day | Leap New Year |
|---|---|---|---|
| Squares | [Full](previews/october-squares.png) · [Desk](previews/october-squares-desk.png) | [Full](previews/last-day-squares.png) · [Desk](previews/last-day-squares-desk.png) | [Full](previews/leap-new-year-squares.png) · [Desk](previews/leap-new-year-squares-desk.png) |
| Dots | [Full](previews/october-dots.png) · [Desk](previews/october-dots-desk.png) | [Full](previews/last-day-dots.png) · [Desk](previews/last-day-dots-desk.png) | [Full](previews/leap-new-year-dots.png) · [Desk](previews/leap-new-year-dots-desk.png) |

Tests in `test/plugins/days-left-this-year.test.ts` exercise the real QuickJS
sandbox: discovery's empty context, year rollover, leap boundaries and century
exceptions, both sides of local New Year, DST transitions, invalid dates, zero
network calls, and PNG dimensions through the real render entrypoint for all three
faces. They also verify the catalog's Face choices, compatibility with old settings,
and exact bright/dim marker counts in ordinary and leap years. The package's catalog
test also pins the discovered entry.

One-, two- and three-digit counts were inspected at full size and desk scale.
The original progress-bar PNGs remain byte-identical. For version 1.1.0, the real
browser and a disposable local server were also checked: the Face choices appear,
Squares and Dots save and render, and Dots remains selected after reload. Desktop
and narrow-screen layouts were inspected. This is local browser, server-frame and
PNG evidence, not a deployment or physical-panel observation.

## Attribution

Original plugin code, licensed with this repository under GPL-3.0-only. Display
colors and spacing follow `src/kit/theme.ts`; typography uses the host's bundled
Inter Regular and SemiBold under the SIL Open Font License (see
`assets/fonts/OFL.txt`). No new third-party code or assets are bundled. The committed
previews are generated from this plugin by the included fixture script, not artwork
from an external service. Manifest author text is attribution, not a verified
publishing identity.
