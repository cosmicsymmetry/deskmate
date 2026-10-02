# Popular cards and curated images

2026-10-02, Track B. The owner requested every card in the researched popularity
list, followed by their comic/anime ideas except Dilbert. They explicitly authorized
parallel agents, pull requests, merge and deployment without another approval step.
This extends the existing plugin submission flow; it does not approve the separate
marketplace-service proposal.

## Current scope amendment

After the initial deployment, the owner said **“skip calvin then”** (2026-10-02).
Calvin and Hobbes is removed from the shipped plugin directory and catalog, together
with its fixtures/previews and plugin-specific tests. The eight automatic cards
remain. The catalog therefore contains 15 total faces after this follow-up deploy.
The nine-card scope and validation counts below record the original release.

## Original scope

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
Track H owned the independent brightness/schema work, which merged before this batch.
Dilbert is explicitly excluded.

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
- [x] Open the submission PR and confirm CI passes on the reviewed head.
- [x] Integrate current main and check ancestry against every live component.
- [x] Merge and deploy the reviewed merged revision, then verify the installed
  catalog, live rendering and service health.

Physical panel appearance is unobserved unless the owner supplies that evidence.

## Work allocation

Three isolated worktrees own text cards, art/landscape/Ghibli cards, and comic/anime
cards respectively. Track B's primary branch owns integration, review records, PR,
merge and deployment. Other open tracks' changes are preserved and are not deployed
from an unmerged branch as a side effect.

Status: all nine plugins implemented, reviewed, **merged in PR #21 as `8b733db`**
and deployed faces-only at **2026-10-02 11:53:04 UTC**.
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
Nekos.best and GoComics' image server). Installed-runtime checks subsequently
passed as recorded below.

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
accepted the correction. Final submission `42ca1e6` passed all six jobs in
[CI run 37002508098](https://github.com/cosmicsymmetry/deskmate/actions/runs/37002508098);
no skipped or failing gate was accepted as a release pass. All 56 PNGs in that
submission’s CI artifact were byte-identical to the reviewed local checker outputs.

### Release record

The merge preserves sign-in (`149d07f`), brightness (`4f13dd3`) and Deskboy naming
(`0e132ae`), which reached main while this batch was under review. Each upstream PR
passed its applicable CI; none changed the faces package. The combined tree was
checked locally with Rust fmt/clippy, **878 passing Rust tests** (two existing
ignored tests), and doctests. GitHub's actual merge `8b733db` was tree-identical to
that locally checked integration before deployment.

Immediately before deployment, all live component revisions were verified as
ancestors of `8b733db`: faces/binary `4f13dd3`, web `0e132ae`. The clean merged
checkout shipped through `deploy.sh --faces-only`; the deploy script passed its
local and Linux faces suites and cold catalog before installation. It recorded
faces `8b733db397cb11592ac73fbff7d26e8ef3516e0c` at 11:53:04 UTC. Web and binary
revision records were unchanged by this deployment.

The installed CLI was then invoked with a cleared subprocess environment, matching
the server's entry point. Its catalog contained all **16** expected built-in/plugin
faces. All **18** checks passed: default and changed-tap output for eight automatic
cards, actionable Calvin setup refusal, and a supplied official Calvin GIF. All
successful frames measured 448×368; the largest PNG was 295,449 bytes. These checks
read public sources without changing any account's saved card loop.

The service was active/running; public `/` returned HTTP 200 and unauthenticated
`/v1/app/devices` returned the expected 401. Concurrent service restarts were observed
outside this faces-only script, so process identity is not claimed unchanged.
Physical panel appearance remains unobserved.


### Calvin removal follow-up

Owner authorization covers removing Calvin, merging and redeploying without another
approval. The existing exact catalog test now expects the remaining 15 faces; xkcd
and Anime retain their media validation coverage without depending on deleted Calvin
fixtures. Their READMEs no longer describe Calvin-only GIF checks. No saved account
configuration is rewritten. Local validation passes: **737 faces tests / 4046
assertions**, typecheck, lint, formatting, cold discovery (15 faces, no Calvin), and
xkcd/Anime author checks (8 cases / 16 PNGs). PR #24 merged and deployed; the
final policy-preserving reconciliation is recorded below.


### Concurrent release-policy deployment

PR #24 passed all six CI jobs (attempt two, after an unchanged Rust pause-test race),
merged as `9e2bb7e`, and shipped at 2026-10-02 12:19:38 UTC. The installed catalog
then matched all 15 expected entries and the Calvin directory was absent.

During that deployment another track shipped release-policy revision `72173e1`
(seven green CI jobs) to the binary/web. Reconciliation preserves that entire live
revision alongside the Calvin removal; this avoids leaving the older faces host
without the operator withdrawal policy. All old approval entries remain immutable.
xkcd/Anime require documentation-only 1.0.1 entries because the newly deployed
policy hashes every file, including README changes already merged in PR #24. Their
rendering source and permissions remain unchanged. The generated directory omits
Calvin; its old approval entry remains historical.

Reconciled local checks: 762 faces tests / 4177 assertions, typecheck, lint,
formatting and strict approval verification for all eleven installed plugins pass.


### Final removal deployment

**Complete:** [PR #27](https://github.com/cosmicsymmetry/deskmate/pull/27) merged as
`8cc69ad5c5299b301b49698d57e04c121a95f55e` and deployed faces-only at **2026-10-02T12:39:59Z**. This makes the
Calvin removal and the previously deployed release/withdrawal policy repeatable
from main. All seven jobs passed in [CI run 37006723615, attempt two](https://github.com/cosmicsymmetry/deskmate/actions/runs/37006723615);
the first attempt timed out in the unchanged RSS subprocess integration fixture.
The retry used identical source. Independent reconciliation review found no material
issues, and all 16 xkcd/Anime CI PNGs matched the reviewed local renders.

Before shipping, the merged revision was checked to include every live component:
faces `c8fe780`, web/binary `72173e1`. The deploy again passed local/Linux suites
and strict approval verification. The installed CLI, using the operator's actual
policy location, has **15 catalog entries, no Calvin, and 11 approved plugins**.
An isolated temporary policy verified that withdrawal still blocks rendering with
exit 2; the real operator policy was never modified. Service active, root HTTP 200,
unauthenticated app API HTTP 401. Web/binary stayed at `72173e1`. No account loop
was rewritten; physical-panel appearance remains unobserved.
