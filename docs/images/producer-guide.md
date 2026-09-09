# Picture-card producer guide

## What a picture card is

A picture card is a full 448x368 face produced outside this repository and sent to a
named Deskmate image source. The server stores the decoded frame as a durable asset, so
returning to the card moves no image bytes. A producer needs no SDK or manifest.

## Send a picture

```sh
curl -X POST --data-binary @panel.png -H 'Content-Type: image/png' https://deskmate.rodi.one/v1/images/TOKEN
```

An identical picture is a no-op: the server does not transfer the asset again. The
accepted push still counts as liveness and updates the source's observed cadence.

## Image requirements

| Property | Requirement | Why |
| --- | --- | --- |
| Dimensions | Exactly 448x368 | The picture fills the panel's single landscape canvas; there is no scaling contract. |
| Bit depth | 8-bit | Other PNG bit depths are outside the accepted canonicalization path. |
| Colour type | RGB or RGBA | The panel has no alpha channel. RGBA is composited over black. |
| Request body | At most 1 MiB | The cap matches the device's own asset-byte limit. |

PNG is the only accepted ingest format. SVG remains server-authored because accepting
untrusted SVG would add external-reference, entity-expansion, and font-loading surfaces.

## Authentication

The source token is accepted in either of two carriers:

| Carrier | Form |
| --- | --- |
| Path | `POST /v1/images/{token}` |
| Header | `Authorization: Bearer <token>` |

Both carriers use one verification function. A path token can appear in access logs;
use the bearer carrier when the producer should keep the token out of its URL. The token
is write-only, scoped to one source, and can be revoked independently.

## Errors

Each error response is one line and is intended for the producer.

| Status | Meaning | Producer action |
| --- | --- | --- |
| 401 | The token is invalid or has been revoked. | Replace it with the token from a newly minted source. |
| 413 | The request body is larger than 1 MiB. | Reduce the PNG file size and send it again. |
| 415 | The body is not a PNG. | Encode the picture as PNG and send it again. |
| 422 | The PNG has the wrong dimensions or bit depth. | Render it at exactly 448x368 with 8-bit depth. |
| 429 | This source accepted a push less than 5 seconds ago. | Wait until the 5-second minimum interval has passed. |

`MIN_PUSH_INTERVAL = 5 s` is per source. It is far below a legitimate producer cadence
and above an accidental double-post, so a runaway producer cannot repeatedly churn the
asset partition.

## Cadence and staleness

The producer declares no refresh interval and no stale state. The server infers both
liveness and staleness from the source's last 8 accepted push times. Identical-picture
no-ops are accepted pushes and therefore remain part of that history.

1. Compute the consecutive intervals between accepted push times.
2. Once at least 3 intervals exist, take their median.
3. Multiply the median by 3.
4. Clamp the result to the range from 15 minutes through 48 hours.

With fewer than 3 intervals, the source is never stale because one or two gaps do not
establish a cadence. The median prevents one late push from permanently stretching the
deadline. The 15-minute floor keeps a frequent producer from being marked stale after
only a few minutes; the 48-hour ceiling keeps a daily producer from receiving three days
of silence. The source becomes stale only when the time since its last accepted push is
greater than the deadline. Any accepted push clears stale immediately, including an
unchanged picture.

## Designing a good panel

Prefer flat-colour UI panels. RLE565 compresses that kind of frame to ~10 KB across the
link but expands high-entropy input 2x. Photographic content therefore uses the raw
fallback and costs the full ~330 KB.

Keep the bottom strip visually calm. A stale footer overlays the picture when delivery
falls behind; the picture remains full-bleed and reserves no strip for that footer.
