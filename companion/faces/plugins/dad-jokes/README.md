# Dad Jokes

A new dad joke on the first successful refresh of each local day. Tap to fetch
another. Coalesced quick taps ask for one new joke. No settings or credentials.
The whole joke gets the panel: 24–36 px Inter on black, with a quiet source/date
footer. Nothing hides the punchline or clips it to an ellipsis.

## Source, attribution and licence

Only `icanhazdadjoke.com` is allowed. One GET to `https://icanhazdadjoke.com/`
requests JSON with an identifying Deskmate User-Agent as the
[official API documentation](https://icanhazdadjoke.com/api) asks. The API needs no
authentication and explicitly supports third-party integrations. The service is
maintained by Brett Langdon / C653 Labs; see its [about page](https://icanhazdadjoke.com/about).

The API/about pages do **not** publish a blanket content licence for contributed
jokes. We therefore do not claim its joke content is GPL, Creative Commons or public
domain. Joke rights remain with their respective authors; API access permission is
not a general right to redistribute a joke corpus. No corpus or service logo is
bundled. The two small recorded API examples and their previews are attributed to
[the diving joke](https://icanhazdadjoke.com/j/RZv4h3gV0g) and
[the dreams joke](https://icanhazdadjoke.com/j/0189hNRf2g), retrieved 2026-10-02.
The implementation is original GPL-3.0 code under the repository licence.

## Refresh, taps and limits

Successful content is kept for its local date and timezone, including across
scheduled refreshes. A tap, local date change or timezone change fetches again.
The API chooses randomly and may repeat a joke; there is no promised unique daily
sequence. A newly created card can receive a different joke from another card.

The six-hour ordinary refresh is supplemented by the host's approximately
once-per-minute check for local date/zone changes. This is a frozen picture between
pushes. Taps use plugin v1's render fallback; fetching/rendering is not instant.

Responses are validated: a usable ID, supported font characters and at most 240
characters / 60 words. Long jokes, malformed JSON, redirects, timeouts and upstream
errors raise a transient error. The host retains the previous joke/frame and
retries; this plugin never turns an outage into a successful placeholder.
A tap can fail without losing the current joke. Content is upstream, not manually
moderated by this plugin.

One HTTP call plus at most 62 local text measurements fit two planning rounds.
State holds only one joke, its ID and date/zone, well below 16 KB. No personal data,
credentials or joke history is sent upstream.

## Verification and previews

Run `bun run plugin:check dad-jokes` and
`bun test test/plugins/text-cards.test.ts` in `companion/faces`.
Recorded short/longer jokes render through discovery, QuickJS, real measurement and
rasterization. Focused tests cover failed responses, long/malformed content,
coalesced taps, daily caching, dates/timezones, and actual text widths.

[Full preview](previews/full.png) · [40% desk preview](previews/desk.png) ·
[Longer joke](previews/longer-joke.png) · [Longer joke at 40%](previews/longer-joke-desk.png).
PNG verification does not assert physical panel delivery.
