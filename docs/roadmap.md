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

**Schema/wire lock:** **free.** C1 released it by merging PR #7 on 2026-10-01
(`63633d2`), as the lock line said it would (ROD-3). No track holds the schema or the
wire; config stays v10 and the integration added no wire keys or firmware changes. The
next track to need either takes the lock here first, with the owner's authorization.

| Track | Status | Branch | Waiting on |
|---|---|---|---|
| A | **Accounts + claiming merged** (PR #5). Resource-limits/self-host boundary delivered in the ROD-4 issue document; it supersedes the unmerged commercial draft. **Self-host packaging implemented and locally verified**: [guide](self-host.md), [work record](superpowers/specs/2026-09-29-deskmate-self-host-packaging-design.md). Clean checkout builds, Chrome setup/restart/log-mail pass; full companion gates pass. **Merged** as PR #10 (`8f105ea`, 2026-09-29, CI green on all four jobs). The limits spec (ROD-4 rev 3) was reviewed 2026-10-01 -- REWORK, because C1 changed the cost model -- and **revision 4 is being written** on the owner's call to fix the spec before building. No hosted limits or billing implementation shipped | `main` | ROD-4 revision 4, then its one VM re-measurement |
| B | Hosted runtime merged (#8) and deployed; a plugin card drew on dev-0005 (owner, 2026-09-29). **First submission trial merged** via [PR #12](https://github.com/cosmicsymmetry/deskmate/pull/12), all four CI jobs green; owner reports Days Left This Year works. **2026-10-02:** owner approved Squares and Dots alternatives in a per-card Face setting, alongside the original Progress bar. Version 1.1.0 implemented; 463 faces tests pass, full/desk PNGs checked, and real local browser/server verified settings save, rendering and reload persistence. Follow-up [PR #13](https://github.com/cosmicsymmetry/deskmate/pull/13) open; Astra review found no blocking issues. No deployment in this session. [Trial spec](superpowers/specs/2026-10-01-deskmate-plugin-marketplace-design.md) | `track-b/marketplace` | Follow-up review; broader directory/publishing identity decisions and A's plan limits (ROD-4 revision 4 pending) |
| C1 | **Merged and deployed.** PR #7 merged to `main` on 2026-10-01 as `63633d2` (CI green on all four jobs at the head commit), and the full branch -- faces, web and binary -- deployed from `main`: `deploy.sh --status` reads **`63633d2`** for all three, and the revert hazard was ruled out before the deploy rather than after. Frames-in-PSRAM implemented; the release carries Track A, Track B and the three local Track D commits. Firmware `v2.2.0-psram` was previously OTA-verified; nothing here touched the schema, the wire or firmware statics. **The tap is measured on the glass** (2026-09-30, ROD-12, [board notes](hardware/board-notes.md)): 8 staged taps at **0.87-1.13 s contact->stable / 0.30-0.57 s release->stable**, ~13x the 13.3 s baseline and **short of the 250 ms proposed**. ROD-13's rendered-fallback fix (`4757753`) **is now live** -- `tappedState` verified present in the installed `rss.ts` and `hackernews.ts` -- so a tap past the last staged page should turn a page, **which has not been seen on the panel**. The owner chose to **chase the gap** rather than move the target, and ROD-14 **delivered the budget** ([doc](hardware/2026-10-01-tap-latency-budget.md), `fe19506`), **also live and already emitting**: the tap path is traced under the target `server::tap_latency` -- tap arrival, runtime handoff, selector, staged lookup, the staged/fallback decision, and `PushScene` send/ACK -- and lines appeared within a second of the restart under the VM's `RUST_LOG=info,server=debug`. The budget's conclusion stands: **no segment can yet be named dominant**, because there are still no samples from a real tap and the journal cannot split device->server from server->panel; report only the residual `release->stable - (server receive -> PushScene send_start)` and **do not halve an RTT and call it measured**. Seven candidate reductions are costed by boundary and none implemented, deliberately. Both mountings are **retired at 270 only** ("count 90 as done"): 90 degrees was never seen, and closing ROD-12 is not verification of it | `main` (PR #7 merged) | Owner at the desk, with the panel powered: the rendered tap on the glass, a filmed tap to produce the residual, and a `hackernews`-kind card created in the window (61 KB / 33 chunks, never carried). No code or deploy is owed |
| C2 | Not started | -- | C1 |
| D | **Live** at deskmate-site.pages.dev (2026-09-25). The dev-only frame exporter (`app-core/examples/frame_export.rs`) merged to `main` 2026-09-28, so the pack regenerates from a clean checkout; it was re-run and its frames re-checked against the post-A, post-C1 workspace | `deskmate-site`, own repo | A domain before E launches; real photographs; A's **deploy** before the page can offer sign-up |
| E | Not started | -- | -- |
| F | Not started | -- | A |
| G | Not started | -- | C1 phase 2, A |
