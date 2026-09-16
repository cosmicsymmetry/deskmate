# First-class server-rendered data cards

Status: implemented in schema v11. This note supersedes one narrow decision in
`2026-09-11-deskmate-legacy-subtraction-design.md`: its three-kind set and “no new card
kind” conclusion no longer apply to the server-rendered Weather, RSS and Token faces.
The removal of the old device-rendered family and the plugin registry remains settled.

## Why the decision changed

The interim server-rendered implementation represented every face as an ordinary Picture
card plus a row in `data-cards.json`. In the companion, three meaningfully different
cards consequently appeared as `PICTURE / PNG`, distinguished by a subtitle, and the
Picture source dropdown could re-point one face at another.

The owner's direction on 2026-09-16 was explicit: each must be a proper separate card
named Weather, RSS or Token with its respective settings. This also follows the project's
own precedent in `324c981`: rendering identity is stated, not selected, because a
dropdown implies that changing identity is a safe edit when it actually turns one card
into another.

## The replacement decision

Schema v11 adds `weather`, `rss` and `token` beside `clock`, `pomodoro` and `picture`.
Each card owns its face inputs and existing interval refresh policy. The server derives
the internal image source as `card-{id}`; the config neither stores nor displays it.
Picture alone retains a source selector because an external producer is Picture's data,
not a disguised card-kind choice.

The change moves authority, not rendering or delivery. Providers, SSRF-guarded egress,
SVG faces, rasterization, durable assets and the one-image-node scene path stay exactly
where they were. `data-cards.json` and `DESKMATE_DATA_CARDS` disappear. A config PUT
replaces that device's refresher set, so an edit takes effect without a process restart
and a removed card stops fetching.

There is still no manifest, expression language, template/plugin catalog, firmware
change, protocol change or capability change.
