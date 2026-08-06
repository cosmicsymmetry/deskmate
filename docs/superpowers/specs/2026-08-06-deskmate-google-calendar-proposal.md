# Deskmate — Google Calendar sync

**Date:** 2026-08-06
**Status:** Proposal for Rodion. Nothing here is implemented or decided.
**Scope note:** the M4 plan lists "cloud accounts/sync" as explicitly **out of scope for v1**.
Adopting any of this changes approved milestone scope, so it needs your decision before
anything is built.

## The complaint

"The calendar is useless as is, because it doesn't synchronize with Google."

That is accurate, and worth separating into two different failures, because they have very
different fixes.

1. **You have to hand-paste an ICS URL.** Setup friction, once per calendar.
2. **Even when you do, the data is hours stale.** Google's private ICS export is served from
   a cache that Google refreshes on its own schedule — commonly 8–24 hours, and Google does
   not document or commit to a figure. For a "next meeting" display, hours-stale is not a
   slow calendar; it is a wrong calendar.

Failure 2 is the one that makes it useless. Any option that does not fix it is not worth
building.

## Three options

### Option A — keep ICS, document the limitation

Zero code. Tell the user in the card editor that a Google private ICS address updates on
Google's schedule and is not suitable for imminent-meeting alerts.

**Verdict: not sufficient on its own.** It is honest, and the honesty is worth shipping
regardless of what else you choose, but it does not fix the actual problem.

### Option B — OS-native calendar (recommended for macOS)

Read the user's calendars through the operating system: **EventKit** on macOS, the
**Windows.ApplicationModel.Appointments** API on Windows.

The original design spec already anticipates this — §5 lists "OS-native providers (macOS
EventKit calendar, Focus status; Windows equivalents) behind the same data-source trait" as
an after-v1.0 item.

Why it is the strongest option:

- **No OAuth at all.** No client credentials, no token storage, no refresh flow, no consent
  screen, no Google API quotas, no annual verification review.
- **It picks up whatever the user already has.** Google, iCloud, Exchange and local calendars
  arrive through one interface, already synced by the OS at the OS's cadence — minutes, not
  hours.
- **Least new attack surface.** A TCC permission prompt replaces a stored long-lived refresh
  token. There is no credential for Deskmate to leak, because Deskmate never holds one.
- **Setup is a permission dialog and a calendar picker**, not a URL hunt.

Costs and risks:

- Two platform implementations, each behind the existing provider trait.
- Requires the account to be present in the system Calendar app. Most people who use Google
  Calendar on a Mac already have this; some do not.
- EventKit needs an entitlement and a usage-description string, and full-calendar access on
  recent macOS shows a stronger prompt than the older read-only one.
- Not testable from fixtures the way ICS is; needs an integration seam so the provider can
  still be unit-tested against fake event data.

### Option C — Google Calendar API with OAuth (the cross-platform path)

Talk to Google directly.

If you want this, these are the non-negotiables, and I would want your explicit sign-off on
each before writing any of it:

- **OAuth 2.0 for installed apps with PKCE and a loopback redirect.** Not a device-code flow,
  not an embedded webview. A distributed desktop binary cannot hold a secret, and Google
  accepts this for installed apps — PKCE is what makes it safe.
- **Scope `.../auth/calendar.events.readonly` only.** Deskmate displays; it never writes.
  A read-only scope also keeps you out of the heavier verification tier.
- **The refresh token goes in the OS keychain** — macOS Keychain, Windows Credential Manager
  — and never into `config.json`. That file is plaintext, size-bounded, atomically rewritten,
  and copied around during migration testing. A long-lived Google refresh token in it would
  be a credential leak with a long tail.
- **All OAuth lives in Rust.** The webview never sees the token, the authorization code, or
  the client ID. This preserves M3's single-owner rule, which the whole runtime design rests
  on.
- **Incremental sync via `syncToken`**, falling back to a full resync on `410 Gone`, so a
  15-minute refresh is a cheap delta rather than a full fetch.
- **Failure handling reuses what already exists**: revoked token, offline, and rate-limited
  all collapse into the existing last-good + stale-age projection the ICS provider already
  has. A revoked token additionally needs a distinct, actionable state in settings — "Reconnect
  your Google account" — rather than a generic provider error.

Costs and risks:

- A Google Cloud project, an OAuth consent screen, and — once you distribute to people
  outside your own account — Google's verification review. That review is a real calendar
  item, not a formality.
- Ongoing credential lifecycle: rotation, revocation, the seven-day refresh-token expiry that
  applies while an app is in "testing" publishing status.
- The most new attack surface of the three options, and the only one where Deskmate holds a
  credential that can read your calendar.

## Recommendation

**Ship the honesty from A now, build B for macOS, and treat C as the Windows answer or a
later cross-platform fallback.**

A one-line caveat in the card editor about ICS staleness is worth doing regardless — it costs
nothing and stops the current behaviour reading as a bug.

B gets you a genuinely fresh "next meeting" on your own machine without Deskmate ever holding
a credential, and it is already the direction the approved design spec points. It also
generalises: the same provider gives you iCloud and Exchange for free.

C is the right answer only if you need Google specifically on a machine where the account is
not in the system calendar, or when Windows support arrives and its native API proves
insufficient.

## What this does NOT require

Worth stating, because it is the pleasant consequence of the card-model work: **none of these
options change the card model, the config schema, the wire protocol, or the firmware.** A
Google or EventKit calendar is another provider behind the existing
`poll() -> Vec<FieldValue>` trait, feeding the same `calendar` card kind, the same `row-list`
template, and the same `before-event` alert. The only user-visible config change is the
calendar card's `source` gaining a variant beyond `url` and `file`.

## Open questions for you

1. Is macOS-first acceptable, or does this need to land on both platforms together?
2. Do you intend to distribute Deskmate to other people? That decides whether Google's
   verification review is on the critical path for option C.
3. Should multiple calendars merge into one card, or should each calendar be its own card?
   The 8-card cap makes that a real constraint rather than a detail.
4. Is read-only definitely enough — or do you eventually want to acknowledge or decline
   invitations from the device? That would change the scope and the review tier
   substantially, and I would argue against it for a device with one tap gesture.
