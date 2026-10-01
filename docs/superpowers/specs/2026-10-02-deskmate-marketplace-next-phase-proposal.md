# DRAFT -- NOT APPROVED -- owner decision needed

2026-10-02 · Track B · Decision request, not implementation scope.

The [submission trial](2026-10-01-deskmate-plugin-marketplace-design.md) produced
Days Left This Year through PR #12, then the owner-approved three-face update in
PR #13. The existing manifest, sandbox, add menu and automatic settings save were
enough. The author needed custom preview tooling and had to disclose a host-timezone
bug. This follow-up fixes timezone forwarding and supplies scaffold/check commands
and CI PNG evidence. None of that proves demand for a separate marketplace service.

Cost below follows the repository's boundaries: faces/docs and static web assets
can ship without a Rust restart; server changes require a binary build/restart.
Config-schema, wire and firmware-static changes require separate owner approval.
**All recommended next steps stay outside those three expensive boundaries.**
Hosted allowances remain dependent on Track A's revised ROD-4; no earlier draft's
limits are promises.

## 1. Public directory beyond the add-card menu

| Option | Cost and tradeoff |
| --- | --- |
| Keep the add menu and public plugin READMEs | Docs/faces only; no new service, but people cannot browse a curated list before signing in. |
| Generate a static directory from reviewed manifests and previews | Static web build/rsync; no schema/wire/firmware change or server restart. Adds a small publishing step and needs an explicit “available on hosted” distinction. |
| Add searchable catalog/install APIs and an in-product directory | Web plus server work/restart, moderation and availability state; no device boundary is inherently needed. Hosted admission waits on Track A. |

**Recommend the static directory only when there are several useful submissions.**
For one demonstrated plugin, the add menu plus a linked README is enough. The trial
proved rendering and submission, not a need for installation machinery.

## 2. Publishing identity versus Track A accounts

| Option | Cost and tradeoff |
| --- | --- |
| GitHub PR identity; manifest author remains attribution | Docs/review process only. Works now for outside authors; no verified Deskmate publisher badge. |
| Maintainer-reviewed mapping from GitHub handle to publisher | A reviewed metadata file and optional static presentation; no account schema changes. Manual correction and impersonation checks. |
| Bind publishers to Track A accounts | Server identity/storage/API plus web work and restart; account verification, recovery and transfers become product obligations. Config v10/wire need not change, but this must be designed with A. |

**Recommend GitHub identity for the next few submissions.** The trial completed
with it. Do not imply that free-form author text is verified, and do not force an
author to own a panel or a hosted account merely to contribute code.

## 3. Release, updates, revocation and reviewed checksums

| Option | Cost and tradeoff |
| --- | --- |
| Continue reviewed commits and operator-selected faces deployments | Process/docs only. Review records commit and version, but deployment does not mechanically reject changed code reusing a version. |
| Verify a reviewed checksum per plugin version during deployment | Deployment tooling plus a separately approved release index; no device boundary or server restart for faces-only releases. Hash manifest and executable together; CI proves previews belong to that content. Changes need a version increment and renewed review. The author must not be able to self-approve by editing the index. |
| Runtime release service with per-account pins and revocation | Server/storage/UI plus ongoing operations; restart and Track A admission decisions. Supports staged rollout and rollback, but creates a service before the trial shows one is needed. |

**Recommend a reviewed release index with deploy-time verification before scaling
hosted submissions.** Decide its trusted approval source first (owner-approved index
commit or protected release workflow); a hash in the author's own PR is not approval.
Keep release manual, record deployed revision, and require renewed review for code
or permission changes. Retain the previous reviewed version for rollback.

Revocation must distinguish removal from the catalog (blocks new use) from stopping
existing refreshers. Choose an operator denylist checked during discovery/refresh
for urgent removal; document that an already stored frame remains visible and stale.
Clearing a frame or removing a user's card is a separate owner-visible decision.
Deploy-time hashes alone cannot provide immediate runtime revocation. No denylist,
release index, version enforcement or runtime service is implemented by this proposal.

## 4. Author tooling after this follow-up

| Option | Cost and tradeoff |
| --- | --- |
| Support `plugin:new`, offline `plugin:check`, behavioral tests and CI PNGs | Already supplied by this follow-up; faces/tooling only. Authors work in the public repo and review remains human. |
| Add fixture recording and a local watch/preview UI | Faces/dev tooling, no server/device boundary. Recording needs deliberate sanitization; useful only after real network-plugin submissions identify repeated friction. |
| Hosted browser editor and automatic publishing/review | New untrusted-code service, server/UI/auth work and operational limits; depends on Track A. Still cannot automate visual or permission judgement. |

**Recommend supporting the two commands first.** The trial's bespoke preview script
was concrete friction; an online IDE is hypothetical. Ask the next outside author
to follow the guide unaided, then invest in the steps where that attempt actually
stalls. PNG evidence supplements review; it never proves physical-panel delivery.
