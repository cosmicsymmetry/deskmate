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
- **The owner sets priorities** and settles any contention between tracks: the lock,
  or anything else two tracks both want.

## Board

**Schema/wire lock:** **held by C1** since 2026-09-28, for the wire and firmware halves of
`docs/superpowers/specs/2026-09-28-deskmate-frames-live-in-psram-design.md` (the volatile
frame pool: `VOLATILE_ASSET_SLOT_COUNT` 2 -> 16, a new `StatusResponse` key, and the
`docs/protocol/v2.md` amendment). `CURRENT_SCHEMA_VERSION` is untouched and stays free.
Phase 1 needed no lock -- tap-to-face crossed no boundary -- and C1 releases this one when
the batch merges. Release preparation has not merged; the lock remains held and its scope is unchanged.

| Track | Status | Branch | Waiting on |
|---|---|---|---|
| A | **Accounts + claiming merged** (PR #5, 2026-09-28). Not deployed yet: on the owner's direction **C1's next deploy carries it** -- C1 merges main, and the first start of that build migrates the live config into re.aleksandrov1@gmail.com's account (`DESKMATE_PUBLIC_URL` and `DESKMATE_OWNER_EMAIL` are already in the live `server.env`). After it, sign-in is an email link from the server log. Next: plan limits and billing, and self-host packaging | -- | The Web Serial spike at the board (plan Task 1) |
| B | Not started | -- | -- |
| C1 | Frames-in-PSRAM implementation complete; **release preparation blocked on managed GitHub access** (ROD-3, 2026-09-29). Firmware `v2.2.0-psram` was OTA-verified previously. The newer 2026-09-29 board entry records an account migration and staged views on the live server; no new hardware observation is claimed. Release runbook and warm weather-tap measurement are on the issue. Integration with Track B and the local Track D commits, final gates, and CI are still required | `track-c1-frames-in-psram` (existing PR #7, remote state unverified) | Restore managed GitHub identity/credentials; merge and verify the release. Owner separately authorizes any deploy; staged-view tap latency on the glass remains unmeasured |
| C2 | Not started | -- | C1 |
| D | **Live** at deskmate-site.pages.dev (2026-09-25) | `deskmate-site`, own repo | A domain before E launches; real photographs; A for sign-up |
| E | Not started | -- | -- |
| F | Not started | -- | A |
| G | Not started | -- | C1 phase 2, A |
