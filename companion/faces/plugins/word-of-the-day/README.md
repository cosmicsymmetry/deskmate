# Word of the Day

A daily English word and its live Wiktionary definition. Choose **Curious words**
(default) or **Everyday words**; each is a 48-word rotation. Tap to explore the next
word. The next local day resets to that day's scheduled word. Multiple taps advance
that many words, wrapping within the selected vocabulary.

The word leads in 40–52 px Inter, followed by its part of speech and a complete
24–30 px definition. A quiet source/date footer credits the dictionary.
This is Deskmate's own word selection, not Wiktionary's editorial Word of the Day.

## Source, attribution and licence

Only `en.wiktionary.org` is allowed. The plugin GETs
`https://en.wiktionary.org/api/rest_v1/page/definition/WORD`, taking the first short,
readable English definition. It strips markup, decodes common entities and
normalizes whitespace. It does not request audio, pictures or quoted examples.

Definitions are by Wiktionary contributors under
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/); see
[Wiktionary's copyright terms](https://en.wiktionary.org/wiki/Wiktionary:Copyrights).
For any displayed word, `https://en.wiktionary.org/wiki/WORD#English` is the source
entry and `https://en.wiktionary.org/w/index.php?title=WORD&action=history` identifies
contributors. The on-panel credit and these links apply to the definition text in
renders and fixtures, whose licence remains CC BY-SA 4.0. The word lists and code
are original work under the repository's GPL-3.0 licence, with no upstream code.

The public fixtures were retrieved 2026-10-02:
[assiduous](https://en.wiktionary.org/wiki/assiduous#English),
[verisimilitude](https://en.wiktionary.org/wiki/verisimilitude#English), and
[quietude](https://en.wiktionary.org/wiki/quietude#English).
The JSON responses are recorded unchanged. Previews use those actual definitions.

## Refresh, taps and limits

Daily selection counts civil dates already localized by the host, so DST does not
skip a word. Invalid/absent vocabulary values use Curious words. The selected word,
definition, part of speech, date, zone and tap offset are cached; changing vocabulary,
word, local date or timezone invalidates the cache. A cached refresh does no HTTP.
A tap normally requests a new word; a complete 48-word wrap can reuse the current one.

There is one HTTP request and at most 62 local text measurements, within two planning
rounds. English definitions longer than 160 characters / 60 words or outside the
bundled font's supported scripts are skipped. If no readable definition is returned,
or the endpoint fails, a transient error keeps the host's last frame/state. There is
no fabricated offline definition. The live REST endpoint may change or be retired;
no availability guarantee is implied.

The host checks successful date/zone changes about once per minute in addition to
the six-hour ordinary cadence. Taps use plugin v1's render fallback, so a fetch and
rasterization can delay the change. A picture does not update on its own.

## Verification and previews

Run `bun run plugin:check word-of-the-day` and
`bun test test/plugins/text-cards.test.ts` in `companion/faces`. The recorded cases
cover both vocabularies and a long word. Focused sandbox tests cover unique daily
selection, local New Year, leap day/DST, settings, coalesced taps, failures,
malformed markup, entity decoding, state bounds and actual measured line widths.
All 96 selected words were checked against the live API and replayed through the
sandbox/measurement renderer on 2026-10-02; all had a usable definition.

[Full preview](previews/full.png) · [40% desk preview](previews/desk.png) ·
[Long word](previews/long-word.png) · [Long word at 40%](previews/long-word-desk.png) ·
[Everyday word](previews/everyday.png) · [Everyday word at 40%](previews/everyday-desk.png).
These are actual rendered PNGs, not evidence of physical panel delivery.
