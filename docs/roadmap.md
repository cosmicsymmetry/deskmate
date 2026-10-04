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
- **The public name is Deskboy; Deskmate stays the codename** (owner, 2026-10-01; the
  domain `deskboy.sh` was registered 2026-10-02). Everything a person reads says Deskboy:
  the landing site, and the window's sign-in screens, wordmark, copy and tab title
  (`PRODUCT_NAME` in `companion/apps/deskmate/src/lib/product.ts`). The repository, the
  server binary, the `DESKMATE_*` settings, the hosted app's hostname and the code's own
  identifiers keep Deskmate, as do the engineering documents.
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
  carries. Note the limit of that check: a squash merge gives the
  live commit a different hash from anything on `main`, so the ancestry test fails even
  when the content is there -- `96bed8c` was such a case (its fix reached `main` inside
  PR #8; the branch was deleted 2026-10-04) -- and then compare trees, not hashes. **Only a merge makes a deploy safe to repeat**, because only then are what is
  live and what is on `main` the same thing.
- **The owner sets priorities** and settles any contention between tracks: the lock,
  or anything else two tracks both want.

## Board

**Schema/wire lock: RELEASED.** Track H's brightness change merged as [PR #18](https://github.com/cosmicsymmetry/deskmate/pull/18) (`4f13dd3`, 2026-10-02) and its server/SPA are deployed. Config is v11 (`preferences.brightness`); protocol v2's ApplyConfig key 4 is gated by capability bit 11. Track H subsequently recorded OTA installation and capability/config readback; visible brightness checks remain unobserved. C1 released the previous hold by merging PR #7 on 2026-10-01 (`63633d2`).

| Track | Status | Branch | Waiting on |
|---|---|---|---|
| A | **Accounts + claiming merged** (PR #5). **Sign-in completion merged and deployed** ([PR #20](https://github.com/cosmicsymmetry/deskmate/pull/20), `149d07f`, 2026-10-02 11:30 UTC), carrying the live Track B Moon Phase and C1 changes. Production log-link sign-in, session persistence across restart, sign-out and link replay rejection verified; SMTP credentials remain unset, so sign-in links go to the server log. [Verification record](superpowers/specs/2026-09-23-deskmate-accounts-and-claiming-design.md). Resource-limits/self-host boundary delivered in the ROD-4 issue document; it supersedes the unmerged commercial draft. **Self-host packaging implemented and locally verified**: [guide](self-host.md), [work record](superpowers/specs/2026-09-29-deskmate-self-host-packaging-design.md). Clean checkout builds, Chrome setup/restart/log-mail pass; full companion gates pass. **Merged** as PR #10 (`8f105ea`, 2026-09-29, CI green on all four jobs). The limits spec (ROD-4 rev 3) was reviewed 2026-10-01 -- REWORK, because C1 changed the cost model -- and **revision 4 is being written** on the owner's call to fix the spec before building. No hosted limits or billing implementation shipped. **Client-IP forwarding fixed 2026-10-04** ([PR #34](https://github.com/cosmicsymmetry/deskmate/pull/34), deployed `5c44245`): Caddy trusts only the cloudflared host `192.168.8.10`, takes `CF-Connecting-IP` and sends `X-Forwarded-For {client_ip}`; Deskmate trusts `DESKMATE_TRUSTED_PROXIES=172.26.0.0/16` (Caddy is alone on that bridge). Observed on the VM: a request through Cloudflare with a forged `X-Forwarded-For: 6.6.6.6` reached Deskmate carrying the caller's real public address. Per-IP sign-in limits are per visitor again, and Google starts are 10 per client per 600 s. The inspection and local fixture did not change or verify the live tunnel end to end | `main` | ROD-4 revision 4, then its one VM re-measurement; client-IP forwarding and the two-client check |
| B | **Hosted runtime, author tooling, release verification, operator withdrawal and documentation directory delivered** (PRs #8, #14, #19, #27); eleven plugins remain after owner-directed Calvin removal. [Approved scope and current handoff](superpowers/specs/2026-10-02-deskmate-marketplace-next-phase-proposal.md#current-scope-reconciled-2026-10-04). **2026-10-04:** reconciled stale author-contract/trial statements and verified the scaffold/check workflow in an isolated export. The owner approved a reproduced GitHub stats credential defect: **1.0.1 removes a duplicated Bearer prefix**, with a failing-before/passing-after regression and an appended release hash preserving all fourteen prior entries. Full local gates, all eleven offline plugin checks and byte-identical success/fallback PNG comparisons pass; [PR #36](https://github.com/cosmicsymmetry/deskmate/pull/36) merged as `3e89172` after nine green PR checks and green main CI. **GitHub stats 1.0.1 deployed faces-only at 2026-10-04 18:58:26 UTC on owner approval.** The pre-deploy live ancestry check passed for all three parts at `5c44245`; the deployment carries all merged tracks, including C1, Track H/schema v11, Track A proxy forwarding, and the existing Deskboy changes. Local/VM deploy gates passed; installed release verification covers all eleven plugins, the installed synthetic-token regression passes, and cold discovery returns four built-ins plus eleven plugins. The service stayed active with the same PID/start time; `app.deskboy.sh` answered HTTP 200. Web/binary remain at `5c44245`. These are installed-code and HTTP checks, not a real-credential request or a panel observation. No schema, wire or firmware change. | `track-b/next` | Track A ROD-4 revision 4 and implementation for hosted allowances; owner direction before further product scope |
| C1 | **Done for now (owner, 2026-10-02): the owner measured the deployed tap themselves and called it "pretty much satisfying"; the filmed residual is dropped and the 250 ms chase is closed** ([board notes](hardware/board-notes.md)). **PR #7 merged and deployed** on 2026-10-01 as `63633d2`; firmware `v2.2.0-psram` was previously OTA-verified. **The 250 ms target remains.** Historical glass (2026-09-30, ROD-12, [board notes](hardware/board-notes.md)): 8 staged taps, 0.87–1.13 s contact→stable / **0.30–0.57 s release→stable, median ~420 ms**. ROD-13's rendered-fallback fix is live but has not been seen on the panel. ROD-14 tracing is live; the retained October 1 journal has **one tap, dropped before selection**, with receive→dispatch 23.105 ms and no scene/ACK. **Server reductions merged as [PR #15](https://github.com/cosmicsymmetry/deskmate/pull/15) and deployed 2026-10-01 22:53 UTC** (`aad9017`, then `424c8a7` with Track B): bounded retained selector separate from renders, runtime event wake, direct resident selection, durable rapid-tap ordering and capacity-aware fourth-page staging. Real-Bun local loopback bench, 35 taps/mode: staged **79.484→5.025 ms median, 105.094→6.768 ms p95**; resident page four 5.580 / 6.369 ms, forced fallback 193.749 / 237.121 ms. **All six review findings fixed**: connection-scoped residency, pre-upload reclaim preserving the live frame, replay of superseded tap counts, weak wake ownership, independent idle-child reaping and a six-view ceiling test. Full gates and ten added mutation probes pass; earlier fallback outliers remain disclosed; full evidence and limits are in the [budget](hardware/2026-10-01-tap-latency-budget.md). These are **local server measurements, not glass**; the deployed path has no tap sample yet. Schema v10, wire v2 and firmware unchanged. Both mountings remain retired at 270 only; 90 degrees was never observed | `main` (PR #15 merged) | Nothing. Still unobserved, not scheduled: the rendered fourth-page tap and a `hackernews` card on the glass |
| C2 | Not started | -- | C1 |
| D | **Live at `deskboy.sh`**; **the hosted app lives at `app.deskboy.sh` since 2026-10-04 17:57 UTC** (owner-approved move, done by D in Track A's server configuration: tunnel ingress + DNS for the new host, a Caddy site block, a second Google redirect URI, then `DESKMATE_PUBLIC_URL=https://app.deskboy.sh` and a server restart; `deskmate.rodi.one` keeps `/v1/*` and `/feeds/*` for the panel, image push and producers, and 308s every other path to the new host). Verified from outside: instance, sign-in page and the Google redirect carry the new origin; the old host's API still answers. Verified in the server log: `dev-0005` reconnected within 30 s of the restart and again after the Caddy reload, and is exchanging scenes. Not yet observed: a real Google sign-in on the new address (owner). The site's app link follows (deskmate-site `91770a9`, deployed); the curl example keeps `deskmate.rodi.one` by the 2026-10-01 ruling. Still true: the site says Deskboy throughout, links the public repo, the self-host guide and the sign-in, has a favicon, sitemap, `robots.txt` and a feed; `www.deskboy.sh` redirects to the bare domain; `hello@deskboy.sh` forwards to the owner; Google sign-in is on with its consent screen in testing. **Open, approved:** outbound mail for sign-in links (the server logs them; it speaks SMTP only, `DESKMATE_SMTP_URL`/`DESKMATE_MAIL_FROM`). Full record, blockers and traps: `docs/open-tasks.md` in the `deskmate-site` repo | `deskmate-site`, own repo | Owner: a free mail provider account with SMTP (for outbound mail), one real Google sign-in on `app.deskboy.sh` to confirm the move end to end, and real photographs (the page has one). Sign-up stays a waitlist until there is a panel to buy (F) |
| E | **Approval pack ready (2026-10-04, issue ROD-5).** The log is live on the custom domain: `deskboy.sh/log`, entry 01 and the RSS feed all verified answering. Launch shape **B** (owner): every one-shot channel -- Show HN, Hackaday, the subreddits -- is held until Track F has a store, so **F sets the launch date, not E**. The repo is public (GPL-3.0, quiet) but still **not a channel**: no description, topics, homepage link or social preview, 0 watchers, 0 releases against 5 tags, and **all five tags predate the `LICENSE`**, so a release from an existing tag would publish an unlicensed snapshot -- a first release needs one new tag, which needs owner authorization. Everything owed is drafted against today's facts and waits on one yes per item, on ROD-5: the About box and 12 topics as unexecuted `gh repo edit` commands; the README opening (Deskboy named, `deskboy.sh`, self-host and the plugin sandbox above the fold) held on `track-e/channels`; pieces 03 and 05 as paste-ready log entries, 03 rewritten because Track B's plugin contract made its first line false; a 1280x640 social preview cut from the site's own photograph (no longer blocked on a photo sitting); draft release notes. Social accounts approved but **held**; the owner names the handle (`deskboy` is taken on GitHub and X, free on Bluesky and three Mastodon instances; nothing registered). Nothing has gone outward since the quiet flip | `track-e/channels` | Owner, one yes each: About box and topics; README copy; pieces 03/05 as text; a handle; a release tag. Launch waits on **F** |
| F | Not started | -- | A |
| G | Not started | -- | C1 phase 2, A |
| H | **Done (owner, 2026-10-03): the owner reports the slider works on the glass.** Merged (PR #18), deployed at `4f13dd3`, and `v2.3.0-brightness` is on `dev-0005` by OTA (2026-10-02 11:55 UTC)** -- capability `display-brightness` read by name, live config v11 at 78% ([board notes](hardware/board-notes.md)). [Brightness spec](superpowers/specs/2026-10-02-deskmate-brightness-design.md): schema v11 reads v10, 10..100% with default 78% = existing raw 200; ApplyConfig key 4 gated by bit 11 on initial apply and replay; LVGL queue, RAM only; one SettingsSheet slider, preview undimmed. `v2.3.0-brightness` builds with **zero .bss/.data/IRAM/DIRAM delta** against `c20dbbb`. No flash-first order. Full local gates pass; 20 guard mutations killed; built window changed/saved/reloaded in Chrome, including a local simulated bit-11 link. **Schema/wire lock RELEASED.** OTA installation and capability/config readback are server-side observations; brightness changes on the glass remain unobserved | `main` (PR #18 merged) | Nothing. Unobserved, not scheduled: reboot restoring the saved level, the 90-degree mounting |
