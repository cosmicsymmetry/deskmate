# This Day in History

One selected Wikipedia event for the owner's local calendar date. Tap to move to
another event for the same date; multiple taps advance that many positions, wrapping
at the end. The feed's order is preserved. No settings or credentials are needed.

The 448×368 face uses bundled Inter on black. The event's year leads, today's date
sits alongside it, and the event is complete rather than silently clipped. Host
measurements wrap the body at 24–30 px; a source and position line sits below it.

## Source, attribution and licence

Only `en.wikipedia.org` is allowed. One GET to
`https://en.wikipedia.org/api/rest_v1/feed/onthisday/selected/MM/DD` supplies the
[Wikifeeds selected-events feed](https://www.mediawiki.org/wiki/Wikifeeds_API).
No pictures, tracking resources, credentials or page HTML are requested.

Text is credited to Wikipedia contributors and reused under
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/), subject to the
[Wikipedia copyright terms](https://en.wikipedia.org/wiki/Wikipedia:Copyrights).
The dated selection, e.g. [October 2](https://en.wikipedia.org/wiki/Wikipedia:Selected_anniversaries/October_2), and
[its history](https://en.wikipedia.org/w/index.php?title=Wikipedia:Selected_anniversaries/October_2&action=history)
provide source/author context; the API also identifies the linked event articles.
The rendered and recorded Wikipedia text retains that licence. The implementation
is original code under this repository's GPL-3.0 licence. No upstream code is copied.

`check.json` contains text/year fields from the public October 2 feed retrieved on
2026-10-02; unused article and image metadata was omitted. Previews render those
actual recorded entries. Whitespace is normalized; event wording is not rewritten.

## Refresh, taps and limits

The successful local-day response is cached in plugin state. A new local date or
zone fetches a new feed. Same-day scheduled refreshes and taps need no network.
At most 16 events are kept, each at most 235 characters / 60 words. Overlong or
unsupported-script entries are omitted, so the position count is the readable
selection, not the feed's total. English only. Historical events can concern
violence or tragedy; this is the editorial selection, not a children's feed.

The host checks successful date/zone changes approximately once a minute; rendering
and delivery add time. The ordinary refresh is six hours. Plugin v1 uses the
existing render-on-tap fallback, not staged instant views. A failed response,
malformed feed, empty readable selection or text that cannot fit raises a transient
error, preserving the last frame/state and using the host retry schedule.

One HTTP call and at most 62 text measurements fit two planning rounds. Saved state
is bounded well below 16 KB. No data leaves the device/account except the requested
month/day and the identifying public Deskmate User-Agent.

## Verification and previews

From `companion/faces`, run `bun run plugin:check this-day-in-history` and
`bun test test/plugins/text-cards.test.ts`. The fixtures exercise the initial event
and a tap. Focused sandbox tests cover failures, malformed data, cache corruption,
New Year in opposing timezones, leap day, DST, coalesced taps, escaping and measured
line widths. The real catalog/render entrypoints also produce a 448×368 PNG.

[Full preview](previews/full.png) · [40% desk preview](previews/desk.png) ·
[Second event](previews/second-event.png) · [Second event at 40%](previews/second-event-desk.png).
These are host-rendered PNG evidence, not proof of physical panel delivery.
