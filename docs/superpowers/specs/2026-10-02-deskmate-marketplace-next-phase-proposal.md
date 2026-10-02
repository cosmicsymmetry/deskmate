# APPROVED by the owner 2026-10-02 -- all four recommendations

2026-10-02 · Track B · Approved decision record.

The owner approved all four recommendations as written: “do as recommended”.
The original alternatives and recommendations below remain as decision history.

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
Deploy-time hashes alone cannot provide runtime revocation. The approved follow-up
implements the release index and operator denylist; a runtime release service remains
outside the decision.

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

## Decisions taken while implementing

- Directory and identity remain documentation: a generated, test-checked
  [plugin index](../../plugins/index.md) links each README. GitHub account and PR
  history identify contributors; manifest author text is unverified attribution.
  No panel or hosted account is required to submit.
- The [canonical release hash](../../plugins/releases.md) covers every regular file,
  including tests, docs and assets, with UTF-8 byte ordering and length framing.
  Symlinks and special files are refused. No extension-based exclusions; consequently
  any file change inside a reviewed folder requires a new version and review.
  The two reviewed shipped plugins seed the index; GitHub stats gains a setup README.
  Moon Phase joins the seeds at its exact merged content: the owner accepted PR #16
  and explicitly directed its inclusion. All three plugins pass strict install
  verification and appear in the documentation directory.
- Owner-authored index PRs are the approval source. Previous entries are immutable
  rollback history. A trusted-base `pull_request_target` workflow checks PR identity
  without executing submitted code; CODEOWNERS is documentation, with no repository
  settings changed. An outside PR can pass with a pending new version, but cannot
  change the index or reuse an indexed version for different bytes.
- The deploy verifies locally before faces-related VM contact and re-verifies the
  staged target bytes immediately before installation. All modes that ship faces
  share this path. Its verification command is tested in isolation: `--dry-run`
  still touches the VM and is not used for this work.
- Operator withdrawal defaults to the instance config root's `plugin-denylist.json`,
  outside deployed faces; the cleared child environment receives only its explicit
  path. Discovery retains private withdrawal metadata for existing-card status while
  hiding those plugins from creation. Every refresh checks again. Missing file means
  no denials; malformed/unreadable policy fails closed for plugins and logs an error.
  Existing pixels remain stored and visible; built-ins are unaffected. Catalog changes
  appear within its minute-level reload; refresh and recovery keep the existing cadence.
- Author previews stay offline and independent of instance secrets/withdrawal policy.
  The next outside author is invited to use the existing guide unaided and report
  where they get stuck. No preview UI, install API or account/publisher service is added.

## Local verification (2026-10-02)

- Rust: fmt, clippy with warnings denied, workspace all-targets (867 passed,
  two existing ignored) and separate doctests pass. Web: 216 tests, typecheck,
  lint, formatting and build pass. Faces after integrating main's Moon Phase:
  631 tests, typecheck, lint, formatting, dump, cold describe and all three offline
  author checks pass. Two repository integration tests and seven approval-policy
  tests pass; actionlint and shell syntax validation pass.
- All 32 deliberate guard mutations were caught, covering hash/content/version
  identity, approval and rollback history, deployment refusal, discovery, refresh,
  malformed policy, status/menu/API behavior, logging and cleared-environment paths.
  CLI/deploy mutations were repeated after moving their repository-only tests out
  of the separately shipped faces bundle.
- The faces bundle was copied to an isolated temporary directory without repository
  docs or deploy scripts. Its complete suite and cold describe pass. Reviewed fixture
  bytes pass the real verification CLI; changed bytes stop the extracted deploy
  preflight before gates or VM-command stubs. All three seeded plugins, including
  the owner's accepted Moon Phase release, pass strict verification. Unindexed
  fixture versions pass CI but fail installation. Before the subsequent deployment
  authorization, neither deploy nor its dry-run was run, and no VM contact or
  hardware verification was performed.
- Built-in SVG goldens are unchanged. All 18 Days Left checker PNGs match the prior
  accepted output byte-for-byte; its three desk-scale layouts were inspected again.

An initial run with Rust, web and faces gates concurrent failed three unchanged C1
selector subprocess tests (idle reaping, inherited-pipe timing and a worker refusal).
Ten consecutive focused selector runs then passed, followed by the complete Rust
workspace run. Selector lifecycle code and its deadlines are unchanged by this work;
the initial failures are disclosed rather than hidden by a timeout change.

The owner subsequently authorized deployment. Its first attempt stopped before
installation: the VM build user could not traverse the live config directory,
which correctly made ordinary package tests treat the denylist as unreadable.
Tests now use an isolated policy path, and the staging catalog check receives a
temporary empty policy removed on exit. The live service's path and policy guards
are unchanged. All 631 faces tests pass under an inherited malformed policy; three
repository integration checks pass, and deleting either isolation guard fails its
regression check (34 caught mutations total).
