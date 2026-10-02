# Popular cards and curated images

2026-10-02, Track B. The owner requested every card in the researched popularity
list, followed by their comic/anime ideas except Dilbert. They explicitly authorized
parallel agents, pull requests, merge and deployment without another approval step.
This extends the existing plugin submission flow; it does not approve the separate
marketplace-service proposal.

## Scope

| Plugin | Intended result |
| --- | --- |
| This Day in History | A readable historical event for the owner's calendar date |
| Dad Jokes | A short joke with another available on tap |
| Art of the Day | Curated public-domain art, including Japanese prints and landscapes |
| Calvin and Hobbes | Cycle up to eight supplied official strip image URLs |
| Ink Landscape | A changing original, procedural landscape |
| xkcd | Compact curated strips, with a latest option and source credit |
| Word of the Day | A word and definition that change with the owner's date |
| Anime Images | Curated SFW imagery with artist/source attribution |
| Ghibli Scenes | Official film stills with film and source attribution |

All are ordinary sandboxed picture producers under `companion/faces/plugins/`.
The implementation stays inside the existing plugin, settings and picture-frame
contracts. No config, wire, firmware or browser-component changes are planned.
Track H retains its schema/wire lock. Dilbert is explicitly excluded.

## Visual and interaction direction

Extend the established 448×368 panel language. Text cards use the bundled typeface,
clear hierarchy and enough room for real source text; artwork leads on image cards.
Do not squeeze unreadable comic text to fit or crop away the strip's meaning.
Attribution must remain legible without competing with the content. Inspect actual
renderer output at full size and 40% desk size.

Daily selection uses the injected owner-local date; random selection is stable
between the refreshes or taps documented by each card. Settings use existing fields.
Plugin v1 handles taps through `render` with coalesced `event.taps` and bounded state;
it does not expose the built-in faces' `views`/`onTap` staging interface. These cards
must not promise instant staged interaction. A failed fetch/render keeps the previous
frame and state through the existing transient-error path.

## Sources and implementation boundaries

Use free, publicly accessible sources and minimal explicit host allowlists. Verify
both the source and the final image URLs; the runtime does not follow redirects.
No login, paywall or access-control bypass is part of this work. Respect the existing
request, response, source, execution, embedded-image and state limits.

Plugin code is GPL-3.0-only; this does not relicense remote content. Each README must
identify the actual provider and content terms, including noncommercial restrictions
where present. Do not describe xkcd, Nekos.best or Ghibli stills as public domain.
Do not bundle copyrighted strips or stills into repository fixtures/previews; use
clearly identified synthetic media for reproducible offline integration evidence and
inspect live media separately. Public-domain art and original procedural output may
be committed with provenance. If a proposed live source is inaccessible, report the
specific limitation and use a supported source/configuration instead of a fake feed.

## Delivery and verification

- [x] Implement all nine plugins with appropriate settings and attribution.
- [x] Exercise real sandbox paths: discovery, successful render, malformed/missing
  data, fetch failures, selection state, settings, timezone boundaries and taps.
- [x] Supply offline `check.json` fixtures and accessible full/desk previews.
- [x] Verify live provider requests through the real guarded runtime.
- [x] Run full applicable faces checks and changed-plugin preview checks.
- [x] Complete independent visual/code review and documentation handoff.
- [ ] Open the submission PR and confirm CI passes on the reviewed head.
- [ ] Integrate current main and check ancestry against every live component.
- [ ] Merge and deploy the reviewed merged revision, then verify the installed
  catalog, live rendering and service health.

Physical panel appearance is unobserved unless the owner supplies that evidence.

## Work allocation

Three isolated worktrees own text cards, art/landscape/Ghibli cards, and comic/anime
cards respectively. Track B's primary branch owns integration, review records, PR,
merge and deployment. Other open tracks' changes are preserved and are not deployed
from an unmerged branch as a side effect.

Status: all nine plugins implemented, integrated and independently reviewed; PR #21
awaits final CI, merge and deployment.
Combined faces suite: **754 tests, 4077 assertions, zero failures**. Typecheck, lint
and format pass. The nine offline author checks pass **28 cases / 56 PNGs**, with
no runtime-limit notices. Live guarded renders and tap changes pass for the eight
automatic cards; the supplied-URL Calvin path also renders an official GIF.
Live baseline observed before work: faces
`122faf7`, web/binary `424c8a7`.

### Source findings

Calvin and Hobbes cannot currently discover random strips automatically: GoComics
pages refused automated access, and the independent Comics RSS feed now carries
a notice that its GoComics feed has halted. The card therefore accepts up to eight
official image URLs and cycles those. A public GIF from the publisher's current
`featureassets.gocomics.com` host was reachable from both local and deployment
hosts; this is distinct from permission to republish an archive. No access-control
workaround or unauthorized mirror is used.

The anime provider serves images that can exceed the runtime's 1 MB response cap,
and sometimes JPEG bytes beneath a `.png` URL. The default collection therefore
uses individually verified small images; fresh selection has a bounded fallback.
No sandbox limits are loosened.

Provider preflight from the deployment host returned HTTP 200 for all eight remote
services/asset hosts tested (Wikipedia, icanhazdadjoke, Wiktionary, Met, Ghibli, xkcd,
Nekos.best and GoComics' image server). Full installed-runtime checks remain a
separate deployment gate.

### Review outcome

Independent visual review inspected all nine actual-content/default faces and desk
views, plus relevant variants. Its sole material finding was xkcd #162's dense
dialogue; that comic was removed from the compact selection. The same full/desk
contact sheets were recaptured. The reviewer scored that correction **resolved**,
with no fix-introduced visual regressions, and returned **ship** at that scope.

Independent implementation review found that signature-only image checks could
accept a truncated PNG, silently draw no artwork and advance state. The three media
plugins now check bounded PNG/JPEG/GIF structure before acceptance, including PNG
chunk checksums and required endings. All nine original truncation reproductions
now throw transient errors, and Anime falls back when the first candidate is corrupt.
The reviewer independently reran 47 media tests / 92 assertions and found no material
regression in the fix. These are structural guards, not complete compressed-stream
decoders; each README states that remaining limit.

The reviewed production head is `167513c`. Documentation review passed on that head:
settings, sources, attribution, tap semantics, failure behavior and preview claims
agree with the implementations. No global design-system change or repair of older
PRODUCT.md/DESIGN.md drift belongs to this card batch.

[PR #21](https://github.com/cosmicsymmetry/deskmate/pull/21) uses the plugin submission
template. The initial CI run passed faces, previews, firmware and firmware host
tests; companion failed the unchanged `push_gate_waiter_panics_when_the_gate_is_not_opened`
fixture because its 100 ms scheduling deadline elapsed before the worker reported.
A second run reproduced the same outer scheduling failure even though the gate
correctly panicked (`eventual=Ok(true)`). The test now waits until the worker enters
the gate before observing its result, and uses the surrounding fixture's one-second
budget for cross-thread panic reporting. The actual closed-gate timeout remains
20 ms, and the existing open/join cleanup and closed-gate assertion remain intact.
This changes only a test; no runtime behavior is changed. Independent code review
accepted the correction. The final submission reruns the repository gates; no
skipped or failing gate is accepted as a release pass.

Integration also includes main `149d07f`, the sign-in release that reached production
while this batch was under review. That merge changes no faces-package files.
