# Plugin marketplace: first submission trial

2026-10-01. Track B.

**Status, reconciled 2026-10-04:** the trial is delivered (PRs #12 and #13).
The owner subsequently approved all four recommendations in the
[next-phase decision record](2026-10-02-deskmate-marketplace-next-phase-proposal.md),
implemented by PRs #14, #19 and #27. The approval, checksum and remaining-decision
statements below describe the original trial; they are not today's backlog.
The [author contract](../../plugins/contract-v1.md),
[release policy](../../plugins/releases.md) and roadmap are current.

## Status and owner decisions

The owner chose **Days Left This Year** as our own first plugin submission, and
**GitHub pull requests** as the submission path. The owner then explicitly directed
a separate GPT-6-Astra agent to create and submit the plugin, authorizing prerequisite
commits, pushes and merges. This authorizes this narrow working trial ahead of the
broader marketplace design approval required by the original handoff.

The broader marketplace design is **not approved**. A public directory, publishing
identity inside Deskmate, automated review and hosted plan allowances remain open.
The trial exercises the existing runtime and catalog; it does not decide those
product questions or claim that hosted plan limits have shipped.

## The trial

One agent acts as an author: reads the public contract and submission guide, adds a
plugin in an isolated checkout, verifies it, and opens a PR against `main` using the
plugin submission template. A separate reviewer inspects the exact submitted code,
manifest, output and evidence, and reports concrete findings in the PR. The owner
makes the acceptance decision for the first plugin. Requested fixes return to the
author as commits on the same PR and invalidate review of affected behavior.

The repository is already public. Submission uses GitHub's existing contributor
identity and PR conversation. A manifest's `author` is display text, not verified
Deskmate account identity. No new account linking or review service is needed to
exercise this path.

The public instructions are [Submitting a plugin](../../plugins/submitting.md).
The optional GitHub template is `.github/PULL_REQUEST_TEMPLATE/plugin.md` so that
ordinary non-plugin PRs retain their existing workflow.

## Days Left This Year

- A sandboxed producer with id `days-left-this-year`, exporting `plan` and `render`.
  It produces an ordinary picture card, never a new card kind.
- A large remaining-days count, the year, and a proportional year-progress graphic
  on the existing 448×368 canvas. Detailed visual treatment is the author's work.
- **Owner-approved addition, 2026-10-02:** a per-card **Face** setting offers
  **Progress bar**, **Squares**, and **Dots**. Progress bar preserves the original
  face and is the default for existing cards. Both grid alternatives use exactly
  one mark per calendar day (365 or 366), with completed days bright and remaining
  days dim, in row-major order. This uses the existing manifest enum field and
  automatic settings save; no new browser form, schema or protocol is needed.
- No outbound requests, secrets, account credentials or tap behavior are required.
- The author must state whether today is included and test that exact convention
  on January 1, December 31, the year rollover, and leap-year boundaries.
- The count uses the host-provided calendar date. The initial trial exposed that
  the server omitted the owner's timezone. The 2026-10-02 follow-up forwards the
  existing saved preference and checks successful faces for date changes once a
  minute; [contract v1](../../plugins/contract-v1.md#now) states the shared-source
  rule and freshness bound. This remains a host fix, not plugin timezone arithmetic.
- The frame changes on scheduled renders. The plugin must document its requested
  cadence and cannot promise an exact midnight update or device-local ticking.

## Submission and review

A submission includes the manifest, executable source, meaningful tests, a short
README and reproducible preview evidence. Review covers identity collisions,
declared network/credential reach, resource use, failures, date semantics and visual
legibility. Existing contract-v1 guards remain the runtime authority.

Review identifies the PR head commit and plugin version. Runtime discovery does not
enforce a reviewed checksum or semantic-version policy, so the PR record and deployed
commit are the operational evidence, not a new technical approval gate. Updates
follow the same PR review process; a merge is not permission to deploy.

For this trial, accepted code goes into `companion/faces/plugins/`. It can appear in
the existing add-card menu when an operator separately deploys the faces package.
There is no new directory UI, installation API or automatic publication service.
The trial ends at a reviewable plugin PR; merging or deployment of the plugin is not
necessary to prove submission, and a panel observation is not claimed from a PNG.

## Track A dependency and boundaries

Authoring, local rendering, PR submission and code review can proceed now. Hosted
allowances and admission policy remain dependent on Track A's revised ROD-4 design
and implementation. No rejected revision's CPU/cadence numbers become promises in
the author guide.

No config-schema, wire or firmware-statics changes; the shared lock remains free.
No deployment is part of this trial. All committed examples and evidence must be
safe for a public repository.

## Acceptance evidence

1. An author can follow the public guide and submit a standalone plugin PR.
2. Discovery finds it; `plan({})` does not break discovery.
3. The real sandbox and faces entrypoint render a 448×368 PNG without networking.
4. Tests establish the remaining-days and progress conventions at date boundaries.
5. The faces test, typecheck, lint and format gates pass; PR CI results are recorded.
6. A reviewer inspects the full-size and desk-scale output and records findings
   separately from the author's claims. Timezone/cadence limitations stay visible.

## Still to decide after the trial

Whether discovery needs a public directory beyond the existing add-card menu; how
publishing identity relates to Track A accounts; operational release/revocation and
update policy at marketplace scale; and whether the author experience needs tooling
beyond the existing faces entrypoint. Trial findings will inform those decisions.
