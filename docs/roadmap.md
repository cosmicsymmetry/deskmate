# Deskmate Roadmap

Brainstormed with the owner on 2026-09-22, from scratch. Nothing from the previous roadmap
carries over. This document stays at the level of tracks and how they run; each track
gets its own brainstorm, spec and plan when it starts.

## The story

Someone finds Deskmate through the landing page or a social post and buys a panel. It
arrives flashed. They create an account, claim the panel, and build their loop from our
cards and reviewed marketplace plugins. The free tier has limits; a paid plan lifts them.
Some cards respond to a tap. Anyone can self-host the open core for free, and anyone
can write a plugin and either self-host it or submit it to run on ours.

## Decisions

- **The device is ours.** A user gets a panel by buying an assembled, pre-flashed one
  from us. The firmware stays open source, but DIY is not the path the product sells.
- **Hosted: free tier plus a paid plan. Self-hosted: free, and open core** (amended
  2026-09-23, Track A brainstorm; it said "always free" before). The public repo builds a
  complete self-host server. Paid features live in a private crate that only our hosted
  build compiles in, so there are no licence keys and nothing to enforce; a self-hoster
  also brings their own keys (Google, SMTP, APIs) and has no reviewed plugin directory.
  If self-hosters later want to pay, that code can move into a source-available `ee/`
  directory with signed keys. Open-source, free, ready-made components are preferred
  wherever they fit (accounts, auth, billing plumbing), and nothing may cost money to run.
- **The repository is GPL-3.0-only** (owner, 2026-10-01, ROD-9; it replaced the
  `MIT OR Apache-2.0` that `companion/Cargo.toml` had declared with no `LICENSE` file).
  Anyone who ships a modified firmware or server must publish the source. Plain GPL, not
  AGPL, so the hosted build's private crate stays private: it is run, never distributed.
  Bundled third-party code keeps its own licence (ESP-IDF Apache-2.0, LVGL MIT, Inter OFL).
- **Plugins on our infrastructure are reviewed first.** Anyone can submit; unreviewed
  plugins can still run on their author's own server and push pictures in.
- **Interaction is staged.** First a tap goes to the plugin and it redraws; later, cards
  that need it respond instantly on the device.

## Tracks

| Track | Covers | Starts | Waits on |
|---|---|---|---|
| **A. Accounts, hosting, billing** | Sign-up and sign-in, settings, plan limits, claiming a panel, self-host packaging | Now | Nothing. The others build on it |
| **B. Plugin marketplace** | The plugin contract, submission and review, the directory, self-hosted push | Now (the contract) | A, for publishing identity and plan limits |
| **C1. Interaction: tap to plugin** | A tap is sent to the plugin, which answers with a new picture | Now | Nothing |
| **C2. Interaction: on-device** | Instant responses the device handles itself | After C1 | C1, plus a firmware image |
| **D. Landing page** | Research first (vibebuddy.sh is the reference the owner likes), then the site | Research now | A, for sign-up and checkout |
| **E. Socials and marketing** | Channels, content, launch. Free where possible | Now | D, for somewhere to send people |
| **F. Store and fulfilment** | Checkout, flashing, shipping | Later | A, and a device worth selling |
| **G. Desk agent** | A headless daemon on the owner's machine that runs local actions a server cannot: open a URL, focus an app, run a macro | Later | C1 phase 2, and A for an identity |

## How the tracks run in parallel

- **One session per track.** A, B, C and F are each a long-running Claude Code session
  in its own git worktree and branch, running its own brainstorm, spec, plan and build,
  with `codex exec` workers for implementation. D and E are separate sessions with no
  code worktree; the landing site lives in its own directory and deploys on its own.
- **This file is the shared board.** Each track keeps its line below current: what it
  is doing, its branch, what it waits on.
- **One track at a time holds the schema and the wire.** A, B and C all change the
  config schema or the wire protocol. Only the track named in the lock line changes
  either, and it releases the lock when it merges.
- **Merge to `main` often,** through a pull request, with the full gates run by the
  session itself and CI confirmed green with `gh run list`.
- **A deploy from a branch that does not contain what is live silently reverts it.**
  `deploy.sh` overwrites the binary with no warning and no check. Track B's deploy
  (`96bed8c`) replaced C1's server half on 2026-09-29 10:12 UTC and it stayed replaced
  for 24 hours: staging vanished, commits went back to ~1.6 s, and every C1 claim made in
  that window was measured against the old path. **Before deploying, confirm your branch
  contains every track already live** -- `git merge-base --is-ancestor <live-sha> HEAD`,
  against what `deploy.sh --status` reads, before the deploy and not after -- then
  integrate first, deploy second, and say in the board line which tracks your deploy
  carries. Note the limit of that check: `96bed8c` itself still lives only on
  `track-b/plugin-contract` and has never reached `main`, so a deploy from `main` drops
  it too. **Only a merge makes a deploy safe to repeat**, because only then are what is
  live and what is on `main` the same thing.
- **The owner sets priorities** and settles any contention between tracks: the lock,
  or anything else two tracks both want.

## Board

**Schema/wire lock:** **held by Track H (brightness)**, on the owner's authorization (2026-10-02: "spin up another agent that will work on brightness setting"). H bumps the config to v11 (`preferences.brightness`), adds an `ApplyConfig` key under protocol v2 gated by capability bit 11, and changes firmware; the lock is **released on merge** of H’s PR. C1 released the previous hold by merging PR #7 on 2026-10-01 (`63633d2`).

| Track | Status | Branch | Waiting on |
|---|---|---|---|
| A | **Accounts + claiming merged** (PR #5). Resource-limits/self-host boundary delivered in the ROD-4 issue document; it supersedes the unmerged commercial draft. **Self-host packaging implemented and locally verified**: [guide](self-host.md), [work record](superpowers/specs/2026-09-29-deskmate-self-host-packaging-design.md). Clean checkout builds, Chrome setup/restart/log-mail pass; full companion gates pass. **Merged** as PR #10 (`8f105ea`, 2026-09-29, CI green on all four jobs). The limits spec (ROD-4 rev 3) was reviewed 2026-10-01 -- REWORK, because C1 changed the cost model -- and **revision 4 is being written** on the owner's call to fix the spec before building. No hosted limits or billing implementation shipped | `main` | ROD-4 revision 4, then its one VM re-measurement |
| B | Hosted runtime merged (#8) and deployed; first submission Days Left This Year merged (#12), followed by the owner-approved Progress bar/Squares/Dots choices (#13, now merged). **2026-10-02 author follow-up:** owner timezone forwarding for all renders, minute-level date-change refresh, `plugin:new`/offline `plugin:check`, and changed-plugin CI PNG artifacts, plus a **security fix**: the shared renderer now refuses every external resource (CSS `url()`, image `src`, nested SVG hrefs, fonts) so plugin output can no longer make the host fetch past its manifest's `hosts` allowlist (502 adversarial probes, zero fetches). **Merged as [PR #14](https://github.com/cosmicsymmetry/deskmate/pull/14) and deployed 2026-10-01 23:49 UTC as `424c8a7`** (faces, web and binary), after two independent reviews and six green CI jobs. [Next-phase proposal](superpowers/specs/2026-10-02-deskmate-marketplace-next-phase-proposal.md) is **DRAFT, NOT APPROVED**. No schema, wire or firmware changes. [Trial record](superpowers/specs/2026-10-01-deskmate-plugin-marketplace-design.md). **Moon Phase requested 2026-10-02:** offline plugin implemented with hemisphere setting; independent review found no blockers. Current main integrated; 612 faces tests pass, typecheck/lint/format pass, and the offline author checker passes all ten full/desk preview cases. [PR #16](https://github.com/cosmicsymmetry/deskmate/pull/16) open; CI pending. No deploy or physical-panel observation. [Card spec](superpowers/specs/2026-10-02-deskmate-moon-phase-design.md) | `track-b/marketplace` (Moon Phase follow-up) | Owner decisions on directory, publisher identity and release/revocation; Track A revised ROD-4 for hosted allowances |
| C1 | **Done for now (owner, 2026-10-02): the owner measured the deployed tap themselves and called it "pretty much satisfying"; the filmed residual is dropped and the 250 ms chase is closed** ([board notes](hardware/board-notes.md)). **PR #7 merged and deployed** on 2026-10-01 as `63633d2`; firmware `v2.2.0-psram` was previously OTA-verified. **The 250 ms target remains.** Historical glass (2026-09-30, ROD-12, [board notes](hardware/board-notes.md)): 8 staged taps, 0.87–1.13 s contact→stable / **0.30–0.57 s release→stable, median ~420 ms**. ROD-13's rendered-fallback fix is live but has not been seen on the panel. ROD-14 tracing is live; the retained October 1 journal has **one tap, dropped before selection**, with receive→dispatch 23.105 ms and no scene/ACK. **Server reductions merged as [PR #15](https://github.com/cosmicsymmetry/deskmate/pull/15) and deployed 2026-10-01 22:53 UTC** (`aad9017`, then `424c8a7` with Track B): bounded retained selector separate from renders, runtime event wake, direct resident selection, durable rapid-tap ordering and capacity-aware fourth-page staging. Real-Bun local loopback bench, 35 taps/mode: staged **79.484→5.025 ms median, 105.094→6.768 ms p95**; resident page four 5.580 / 6.369 ms, forced fallback 193.749 / 237.121 ms. **All six review findings fixed**: connection-scoped residency, pre-upload reclaim preserving the live frame, replay of superseded tap counts, weak wake ownership, independent idle-child reaping and a six-view ceiling test. Full gates and ten added mutation probes pass; earlier fallback outliers remain disclosed; full evidence and limits are in the [budget](hardware/2026-10-01-tap-latency-budget.md). These are **local server measurements, not glass**; the deployed path has no tap sample yet. Schema v10, wire v2 and firmware unchanged. Both mountings remain retired at 270 only; 90 degrees was never observed | `main` (PR #15 merged) | Nothing. Still unobserved, not scheduled: the rendered fourth-page tap and a `hackernews` card on the glass |
| C2 | Not started | -- | C1 |
| D | **Live** at deskmate-site.pages.dev (2026-09-25). The dev-only frame exporter (`app-core/examples/frame_export.rs`) merged to `main` 2026-09-28, so the pack regenerates from a clean checkout; it was re-run and its frames re-checked against the post-A, post-C1 workspace | `deskmate-site`, own repo | A domain before E launches; real photographs; A's **deploy** before the page can offer sign-up |
| E | **Publishing.** Plans drafted and five pieces written (issue ROD-5); the dev log shipped via ROD-10 and **Deskmate's first public writing is live** at `deskmate-site.pages.dev/log/thirteen-seconds`, with an RSS feed and a footer link. Launch shape is **B** (owner): every one-shot channel -- Show HN, Hackaday, the subreddits -- is held until Track F has a store, so **F sets the launch date, not E**. The repo going public on 2026-10-01 was deliberately quiet and spent no one-shot. Social accounts are approved but **held** by the owner, who will name the handle; there is no mailer and no domain, so `/log`'s feed plus "watch the repo" are the only ways to follow the project. The repo is **not yet set up as a channel** -- no description, topics, homepage link or social preview, and 0 releases against 5 tags, so "Releases only" watchers currently receive nothing (`github-repo-pack` on ROD-5 specifies the fix; a tag needs owner authorization) | -- | Owner: approve the repo About-box and README copy; approve pieces 03/05 as text; a handle; whether to cut a release. Launch waits on **F** |
| F | Not started | -- | A |
| G | Not started | -- | C1 phase 2, A |
| H | **Implemented and locally verified (2026-10-02), PR ready.** [Brightness spec](superpowers/specs/2026-10-02-deskmate-brightness-design.md): schema v11 reads v10, 10..100% with default 78% = existing raw 200; ApplyConfig key 4 gated by bit 11 on initial apply and replay; LVGL queue, RAM only; one SettingsSheet slider, preview undimmed. `v2.3.0-brightness` builds with **zero .bss/.data/IRAM/DIRAM delta** against `c20dbbb`. No flash-first order. Full local gates pass; 20 guard mutations killed; built window changed/saved/reloaded in Chrome, including a local simulated bit-11 link. **Schema/wire lock released on merge.** No merge, deploy, publish, serial access or hardware verification performed | `track-h/brightness` | PR review; then owner's USB flash, on-board OTA download and brightness visibility at both mountings (UNOBSERVED) |
