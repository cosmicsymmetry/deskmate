# Terminal Command plugin

2026-10-04, one-off Track B contribution requested directly by the owner:
“create a card that will display a random popular Linux or Mac terminal command.”
Follow the existing author tooling and owner visual-acceptance process. This card
exists independently of the Track B session's shared roadmap row, which stays untouched.

## Scope and direction

- Ordinary sandboxed picture producer, `terminal-command` 1.0.0. Config v11,
  protocol v2, firmware, browser components and host runtime remain unchanged.
- Read mode: learn one useful terminal command at a glance. Thirty bundled
  invocations with original plain-English explanations and accurate platform labels;
  no execution, network, credentials, destructive commands or remote shell pipes.
- Extend the existing black 448×368 face, bundled Inter 400/600, `#f5f5f7` primary
  and `#a0a0a8` secondary ink. The command leads at 32–44 px; explanation at 26 px,
  platform/tap footer at 18 px; 32 px side margins. No header or enclosing boxes.
- Long commands wrap only at authored argument boundaries with literal shell `\`
  continuations. Commands and explanations are measured by the host; tests check
  the static footer's ink bounds too. Explanations wrap into
  at most two balanced lines. No ellipsis, tracking, or fractional SVG coordinates;
  therefore neither the letter-spacing trap nor `toFixed` rounding applies.
- One existing enum field: Both (mixed collection, default), Linux or macOS.
  Each restricted pool contains the 24 shared commands plus three native examples.
  The face labels the selected command's support, not merely the setting.

## Selection and content contract

A seeded Fisher–Yates shuffle depends on platform and calendar block; the local
civil day advances through its 30-entry deck (27 for a restricted pool). This is
repeatable pseudorandom selection, independent of process randomness and UTC
midnight. All commands occur once per block, but block transitions may repeat.
Taps advance an offset, respecting coalesced counts and wrapping; the host bounds
an event to 32 taps. Refresh preserves the selection until local date, zone or
platform changes. State is bounded and validated; content always comes from the
bundle. Identical settings/dates produce identical daily picks across cards.

Request six-hour refresh; rely on the existing minute-level date/zone check for
calendar changes. Use plugin v1 render fallback for taps, with no staging/latency
promise. Invalid clocks or missing measurements fail through the existing transient
path and retain the last frame/state.

The plugin README records every invocation, flags and primary reference. All 27
macOS-supported examples were checked against installed macOS 26.6.2 manuals and
executed using stock tools and disposable files. Linux behavior was checked against
GNU, procps-ng, util-linux and the utility authors' documentation. Linux execution
is not claimed. GNU `date -d` and BSD `date -v` remain separate; examples assume
installed standard utilities, normal shell behavior and existing sample file paths.

## Acceptance and evidence

- [x] Started with `plugin:new`, then the offline scaffold `plugin:check`.
- [x] Sandbox tests cover full filtered tap cycles, daily decks, coalescing,
  refresh stability, invalid state/settings/clocks and local-date boundaries.
- [x] Every command is rasterized at 448×368 and actual text ink is within margins;
  all 34 preview fixtures cross the real server entrypoint.
- [x] Full/desk checker PNGs and the 30-command desk sheet inspected locally.
  The preview-only zero-tap fixture was corrected: the host clamps an explicit zero
  to one, so the default fixture must omit `event`. Entrypoint tests enforce this.
- [x] Independent visual/code review returned **ship**, inspecting all 30 full/desk
  commands and six sample pairs, with no material findings. Documentation review
  found one wording fix: the host measures commands/explanations, while the fixed
  footer is checked by tests. Corrected both records. This is an ordinary extension;
  existing global product/design document drift remains out of scope.
- [x] After fast-forwarding main's PR #36, faces tests pass: **819 tests, 7,100
  assertions, zero failures** (11 plugin tests / 2,845 assertions). Typecheck,
  lint, formatting, cold discovery (16 faces), all 34 offline author cases with
  no errors/notices, strict release verification for all 12 installed-source
  plugins, and three repository release checks pass. Plugin-local JS/JSON was
  also formatted/linted explicitly because the package config excludes plugin
  folders. All 12 committed sample PNGs match checker output byte-for-byte.

The owner-authored submission PR records its conventional commit, exact submitted
head and CI result. Its release entry appends the canonical hash of the complete
plugin folder, including tests, documentation and committed PNGs; all historical
release entries remain unchanged. The PR is the visual-acceptance handoff.

Committed samples live in `companion/faces/plugins/terminal-command/previews/`;
all 34 reproducible pairs and five full-size contact sheets live in
`companion/faces/out/plugins/terminal-command/`. The README gives reproduction steps.
No physical-panel observation is claimed. No merge or deploy is authorized by this
request: the owner accepts the plugin visually first.
