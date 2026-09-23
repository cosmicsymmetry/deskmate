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

**Schema/wire lock:** free.

| Track | Status | Branch | Waiting on |
|---|---|---|---|
| A | Accounts + claiming built and reviewed (plan tasks 2-12, 13.1-13.3); PR open. Needs no schema/wire lock | `feat/track-a-accounts` | Owner: the Web Serial spike at the board, and the deploy (live runs C1's build) |
| B | Not started | -- | -- |
| C1 | Not started | -- | -- |
| C2 | Not started | -- | C1 |
| D | Not started | -- | -- |
| E | Not started | -- | -- |
| F | Not started | -- | A |
