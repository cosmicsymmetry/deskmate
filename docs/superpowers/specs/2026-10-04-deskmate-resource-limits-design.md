# Deskmate resource limits and the self-host boundary

2026-10-04 -- Track A -- **revision 6** of the ROD-4 limits spec, proposed for review.
The board called this "revision 4"; the tracker's revisions 4 and 5 (Vault, 2026-10-01)
were interim drafts against `cfdb3a0`, and this supersedes them. The repository file is
the source; the ROD-4 plan document carries the same text.

Scope is unchanged from the owner's 2026-09-29 direction: **resource limits and the
build boundary only.** No price, currency, named tier, billing provider, checkout or
trial. "Free" and "paid" are the two hosted account classes the roadmap already names.
This document authorises no implementation, deployment, spend, schema bump, wire change
or firmware change.

## 1. What changed since revision 5, and why it had to be rewritten

Revision 5 was analysed at `cfdb3a0`. C1's PR #15 merged after it (2026-10-01 22:53
UTC, deployed the same night) and changed the three things revision 5 spent most of its
length on. Every seam below was re-read at `main` `70a0639`, which is what is live
(`deploy.sh --status`: faces, web and binary all `5c44245`, 2026-10-04 09:53 UTC; the
server code is identical between the two).

| Revision 5 said | `main` today | Consequence |
|---|---|---|
| Frames must be pruned to 3 per source and the account keep-set held under the 32-digest wire limit by new "R3" work | `image_sources.rs:40-46`: the store admits staged views against a fixed **15-frame account budget** (8 reserved resting frames plus 7 shared staged slots), `worker.rs:328` caps a source at **4 views including rest**, and oversized legacy caches are trimmed on load | R3 is done, differently. The retained-frames row is a structural fact in every edition, not hosted work |
| A staged tap can queue behind a 45 s render; add a selector-only permit and an interactive lane | `faces_package/selector.rs`: the selector is a **retained worker process** that never renders, serialised by its own mutex, answering in ~0.3 ms locally; renders release the per-source transition lock while drawing | The lane proposal is moot. What remains is a different hazard, in section 3 |
| Hosted catalog admission capped at eight plugin folders | `companion/faces/plugins/` holds **eleven** reviewed plugins plus four built-ins | The cap was already exceeded on the day it was written; drop it |
| Pilot built-ins modelled at 60 s | `data_cards.rs:144`: the default cadence is **900 s**; three plugins declare 900 s, eight declare 21,600 s; 60 s is the floor, and the window exposes no cadence control at all | The pilot's automatic load was overstated by up to 15x |

Nothing from any revision has been implemented. `entitlements.rs` still has its four
methods; the source-cap check still runs outside the mint lock (`images.rs:262` versus
`image_sources.rs:202`); the card error still says "This account" for a per-panel
limit (`app_api/mod.rs:570`); `docs/self-host.md:14` still promises one frame per source.

## 2. Evidence

**Measured, carried forward (Vault, 2026-10-01, live VM, pre-PR #15 package, four
built-ins plus ONE plugin in the catalog, offline fixtures, n=3 per verb, GNU `time`,
transient unit as `nobody` with 0.5 CPU and 512 MiB).** Reproduction bundle:
Paperclip attachment `703ba01d-5d29-41f2-9880-36658c06d88e`.

| Child | CPU-s (user+sys) | Peak RSS | Planning value |
|---|---:|---:|---:|
| Weather render | .14-.15 | 71-73 MiB | .20 |
| Weather views | .13 | 65-68 MiB | .15 |
| Plugin render (github-stats) | .27-.32 | 93-104 MiB | .40 |
| Plugin views | .17-.19 | 73-80 MiB | .25 |
| Describe (five-face catalog) | .14-.19 | 74-80 MiB | .25 |

Older ROD-8 evidence (2026-09-29, deployment `96bed8c`) still stands for what it
measured: a legal near-limit SVG render at **5.34-5.38 CPU-s**; eight synthetic links at
~.016 CPU aggregate and ~250 KiB idle per link; ~1.84 MiB per actively delivering
panel; PNG decode plus persistence ~.0025 CPU-s per write; a paced refusal still
~.0015-.002 CPU-s because pacing runs after decode.

**Observed on the live VM, read-only, 2026-10-04 17:53 UTC** (this revision, `ssh` and
`systemctl show`, no sudo, nothing restarted):

| Fact | Value |
|---|---|
| Host | 12 logical CPUs, 11,958 MiB RAM, 8,307 MiB available, load 0.08 |
| Service run 09:53:31-17:57:27 UTC (8 h 04 min) | systemd's own stop line: **3 min 58.6 s CPU = .0082 CPU average**, **554.9 MiB memory peak** (the service cgroup, children included) |
| Live server-rendered sources | **eight**, all plugins: `days-left-this-year` and `ink-landscape` at 900 s; `anime-images`, `art-of-the-day`, `dad-jokes`, `ghibli-scenes`, `this-day-in-history`, `xkcd` at 21,600 s |
| `Delegate` on the unit | `no` (systemd default; the unit file has no `Delegate=` key) |

The service was restarted at 17:57:27 UTC by something outside this session; the
deployed revision did not change (`deploy.sh --status` still `5c44245` at 09:53). That
restart ended the 8 h window and started M4's.

Two things follow. The 555 MiB peak is above revision 5's 512 MiB "server/store"
partition with one account's eight sources, so that split was already wrong. And at
these cadences the once-a-minute `describe` is the dominant steady cost: ~484
describes at the measured .25 s is ~121 of the 239 CPU-s.

**Measured on the live VM for this revision, 2026-10-04 ~18:05 UTC** (section 9 has
the method; harness and raw output are committed beside this spec under
`assets/2026-10-04-limits-measurement/`). Faces tree identical to live `5c44245`,
**fifteen-face catalog** (four built-ins, eleven plugins), offline fixtures including
the captured Hacker News front page, n=3 per row, transient unit as `nobody`, 0.5 CPU:

| Child | CPU-s | Peak RSS | Wall s | Planning CPU-s |
|---|---:|---:|---:|---:|
| `describe`, fifteen faces | .24-.26 | 85-88 MiB | .49-.50 | **.30** |
| Weather render / views | .13-.14 / .11-.12 | 71-74 / 65-68 MiB | .29 / .21-.29 | .15 / .15 |
| Hacker News render (resting) | .20-.21 | 72-74 MiB | .40-.46 | .25 |
| Hacker News views (answers five pages) | .12 | 66-69 MiB | .20-.28 | .15 |
| Hacker News staged render, pages 2/3/4 | .22-.24 / .25 / .19-.22 | 70-74 MiB | .39-.56 | .25 each |
| `github-stats` render / views | .36-.39 / .24-.25 | 96-99 / 79-85 MiB | .71-.79 / .50-.58 | .40 / .30 |
| `ink-landscape` render / views | .50-.53 / .24-.25 | 98-107 / 85-86 MiB | 1.01-1.08 / .50-.58 | **.55** / .30 |
| `ink-landscape` one-shot tap (exits 1 by design) | .24-.27 | 80-86 MiB | .49-.58 | .30 |

Against Vault's five-face figures, every plugin-kind child is about **.08 CPU-s
dearer**, which is ten more folders of discovery at roughly .008 s each. Built-in
children did not move: they skip discovery (`requestedFace` returns a built-in before
`warmSandbox`).

**The retained selector worker**, one process, same unit:

| Phase | Result |
|---|---|
| Start-up plus first exchange | .30 s wall, .13 CPU-s, 73 MiB RSS |
| Ten Hacker News taps (built-in, `onTap`) | median **0.1 ms**, max 0.2 ms, ~.001 CPU-s each |
| Ten `ink-landscape` taps (plugin, fails by design) | median **91 ms**, max **191 ms**, .038 CPU-s each; RSS 73 to 88 MiB |
| Five Hacker News taps afterwards | median 0.1 ms; RSS stays at 88 MiB |

**Still not measured:** RSS and token at any setting; any staged render whose network
fetch is real; describe or a plugin child with a populated denylist (the run used an
empty one, the same file shape the server passes); the tunnel; anything multi-account.

## 3. The cost model, against `main` today

The unit of charge is the **faces child**, plus one new thing, the **selector
exchange**. The server reaches the package only through `faces_package::run` (one Bun
process per verb) and `Selector::tap` (one request on the retained worker). Nothing
finer is attributable.

### A scheduled refresh

`worker.rs:refresh_once` then `stage_other_views`:

1. One `render` child for the resting view.
2. One `views` child, always, even for a plugin (which answers `[""]`; the child still
   starts and still pays discovery).
3. One further `render` child per staged view, up to **three** (`MAX_STAGED_FRAMES_PER_SOURCE`
   4 including rest). Weather declares one extra view; Hacker News and RSS declare pages.

| Refresh of | Children | Measured CPU-s | Planning CPU-s |
|---|---|---:|---:|
| A plugin | render + views | .60-.64 (`github-stats`), .74-.78 (`ink-landscape`) | **.85** |
| Weather | render + views + 1 staged render | ~.40 | **.45** |
| Hacker News (five pages; staging takes three) | render + views + 3 staged renders | .98-1.04 | **1.05** |
| RSS, token | same shape as Hacker News | not measured | 1.05, by analogy |

Two facts about staging that the limits must respect, both from reading the code:

- **A staged render is spent before admission.** `stage_other_views` draws the view and
  only then calls `accept_staged_view`, which refuses with `StagingCapacity` when the
  seven shared slots are full. A refused page therefore costs a full render every
  refresh. Checking the budget before drawing is a small worker change; note it for
  C1, do not model around it.
- **A staged slot is not released when a view name goes obsolete.** `accept_into_view`
  only inserts. A token card whose chart setting moves from `line` to `candles` keeps
  `line` in one of the account's seven slots until restart (load-time trimming keeps
  the first seven it meets). Bounded, slow, and harmless to the wire; a note, not a cap.

### A tap

`device_link.rs:ServerTapSink` leaves the runtime thread with one `spawn_blocking` per
tap; `data_cards::tapped` drops the event before any work unless the catalog says the
face declares a tap (`face.tap.is_some()`); `select_staged_view` takes the source's
transition lock, then asks the selector.

| Tap on | Path | Cost |
|---|---|---|
| A face that declares no tap (three plugins, external producers) | dropped at `tapped` | nothing but the log line |
| A built-in, view already staged | retained-worker exchange, then one `PushScene` | **no child**; 0.1 ms exchange on the VM, ~5 ms end to end on the loopback bench |
| A built-in, view not staged | exchange, miss, then the refresher's render fallback | one render plus views, .30-.40, coalesced at 32 taps per source |
| A plugin that declares a tap (eight of eleven) | exchange **that fails by design**, then render fallback | .04 exchange + .85 render and views = **.90** |

The last row is the hazard that replaces revision 5's lane problem. For a plugin kind
the retained worker's `select` is `main.ts tap`, which calls `requestedFace`, which runs
`warmSandbox()` and `allFaces()`, which **re-discovers every plugin folder on every
call** (`registry.ts`: "re-runs discovery on every call"), and then throws "this face
handles taps through render" because plugins have no `onTap`. That discovery happens
while `Selector::tap` holds the **process-wide selector mutex**. Measured: **median
91 ms, worst 191 ms** per plugin tap at eleven folders, and the worker's RSS grows about
1.4 MiB per such exchange (it retires at 128 MiB, so roughly every 38 plugin taps).
A staged built-in tap that arrives during one waits that long, which is a third to
three quarters of C1's 250 ms target spent behind someone else's card. It grows with
the catalog. The fix is cheap and Rust-light: `describe` says whether a kind has a
selector (R1 below), and the server skips the exchange when it does not.

### Frames

A canonical frame is 448 x 368 x 2 + 12 = **329,740 B**, held in RAM (an `Arc<[u8]>`
per view, loaded at startup) and on disk. The account bound is structural and the same
in every edition: **at most 15 frames per account, at most 4 per source, one per
external source** -- 4,946,100 B (4.72 MiB) per account at the ceiling. `desired_assets()`
feeds the same set to the panel, so the wire's 32-digest keep-set limit has 17 digests
of headroom and nothing in this spec can approach it.

### Cadence

`MIN_REFRESH` 60 s, `MAX_REFRESH` 6 h, default 900 s. A plugin seeds its cadence from
its manifest at creation (`cadence_for_new_spec`); the owner can change it only by
editing `data-cards.json` by hand. Retries back off from 60 s doubling to the cadence;
the first attempt after a restart is immediate.

### What the live numbers imply

At declared cadences the per-account CPU bucket proposed below is a **backstop, not a
governor**: a 900 s plugin costs 96 x .85 = 81.6 CPU-s a day, 9% of the free rate's
864 a day. The bucket exists for three things that cadence cannot bound: a plugin at
the 60 s floor (.85 x 1,440 = 1,224 CPU-s a day, 1.4x the free rate), a tap storm on a
plugin that declares a tap, and hostile or merely expensive raster work (the 5.4 s
SVG, 96 x 5.8 = 557 CPU-s a day at 900 s, which the free rate still covers).

**The paid frame does not fit the measured cost without R1.** Eight plugins at 60 s
cost 8 x 1,440 x .85 = 9,792 CPU-s a day against paid's 8,640, so they cannot all hold
the floor; the ledger would stretch them to about 68 s and leave nothing for taps.
With R1 (section 8: no views child for a kind that declares no views, which is every
plugin) a plugin refresh is one render, .40-.55, and eight at 60 s cost at most
6,336 a day, leaving a tap every ~34 s account-wide. **R1 is therefore a prerequisite
for offering the paid frame as written**, not an optimisation. The alternative is a
different paid number, which is the owner's call.

## 4. Numeric limits

The numbers in the hosted columns are the **proposed frame carried from revisions 3-5.
The owner has not set them**; the roadmap says only that the free tier has limits and a
paid plan lifts them. Everything in the SelfHosted column is what the public build does
today or will do after the first implementation task, which changes none of its values.

| Resource | Hosted free | Hosted paid | SelfHosted | Where it is enforced |
|---|---|---|---|---|
| Panels per account, pending and active | 1 | 3 | none (`None`) | `claim.rs:94`, exists. Admin mint (`admin.rs:33`) is owner-only and bypasses it by design |
| Cards per panel | 8 | 8 | 8 | `MAX_CONFIG_CARDS`, structural (config v11, wire). `app_api/mod.rs:562`, exists; error wording is wrong |
| Image sources per account | 4 | 8 | 8 | `MAX_IMAGE_SOURCES`, structural. `images.rs:262` exists but races the mint lock |
| Configured plugin sources per account | 1 | 8 | none | **New.** Needs origin from the catalog (R1). Counts source configurations whose kind resolves to a plugin, referenced by a card or not |
| Minimum automatic cadence for a plugin source | 900 s | 60 s | 60 s | `worker.rs:22` floor exists; the hosted floor is a policy clamp applied at schedule time and on edit. Built-ins keep 60 s-6 h in every edition |
| Plugin CPU per account | 36 CPU-s per hour, 10 CPU-s stored | 360 CPU-s per hour, 10 CPU-s stored | none | **New.** Account ledger; section 5 |
| Staged taps (selector exchange) | unmetered | unmetered | unmetered | Bounded by the worker's own limits: 64 KiB in, 16 KiB out, 5 s per exchange, 256 requests or 60 s per process, retire at 128 MiB RSS |
| Render-fallback taps | earliest start 5 s apart per source, charged to the CPU ledger | same | no floor; today's coalescing (32 per source) | **New** pacing in `worker.rs`'s tap branch; hosted only |
| External PNG `POST`, per source | 1 accepted write per 5 s; body <= 1 MiB | same | same | `MIN_PUSH_INTERVAL` and `DefaultBodyLimit`, exist. Admission currently runs after decode |
| External PNG `POST`, per account | .8 attempts per second, burst 4 | 1.6 per second, burst 8 | none | **New** pre-decode admission; the bucket survives source rotation |
| Retained frames | <= 4 per source, <= 15 per account | same | same | `RESIDENT_FRAME_BUDGET`, structural since PR #15 |
| QuickJS heap and evaluation deadline | 16 MiB, 2 s | same | same | `plugins/sandbox.ts`, unchanged |
| Plugin plan guards | 3 rounds, 8 requests, 4 MiB of bodies, 64 measurements, 16 KiB state, 1 MiB source | same | same | `plugins/run.ts`, unchanged |
| Faces child wall deadlines | render 45 s, views and tap 5 s, describe 15 s | same | same | `faces_package.rs:60-71`, unchanged |
| Faces child OS supervision | 10 CPU-s and 256 MiB per child, .5 CPU pool | same | **off** | **New**, operator settings, section 6 |

Paid lifts panels, source and plugin counts, the plugin cadence floor, the CPU rate and
the account `POST` bucket. It lifts nothing structural and nothing in the sandbox.

A free account that is over a cap after a downgrade keeps everything it has: nothing is
deleted, the excess plugin sources stop refreshing deterministically (the most recently
minted first, which is the store's own order), and the window shows which. Lowered caps
refuse new resources only.

## 5. The CPU ledger and tap semantics

**Expected-cost reservation with debt**, integer microseconds, one ledger per hosted
account, persisted beside `data-cards.json`. Reserve .55 s before a plugin render and
.30 s before a plugin views child (the measured planning values, section 2); a child starts only if the balance covers the
reservation. On exit, settle against actual CPU (the whole child, discovery included)
and let the balance go negative. A negative balance blocks new starts until the rate
refills it. A child that dies without a usage report, or is killed by the supervisor,
is charged the **supervisor ceiling plus one second**: 11 CPU-s. No reset on restart,
rotation, plan change or transient failure; a policy revision clamps stored credit to
the new burst and keeps debt; settlement is idempotent by lease id.

One outstanding lease per account. Built-in children are not on any account ledger;
they are bounded by the global pool only.

Envelopes, so the implementation has something to test against: in any hour an
account can settle at most rate x 3,600 + 10 (stored) + 11 (one boundary-crossing
kill) = **57 CPU-s free, 381 CPU-s paid**. Taps: a plugin tap costs .90 measured, so a
free account that does nothing else sustains one tap per **90 s** and bursts 11 from a
full store; paid, one per 9 s. With one 900 s plugin refreshing, free sustains one tap
per ~99 s; with eight 900 s plugins, paid sustains one per ~9.7 s. After R1 a plugin
tap is one render, .55, and those become 55 s / 5.5 s idle. These are consumption
averages, not latencies.

**Budget stops are typed, not transient.** A supervisor kill (`SIGXCPU`, or `SIGKILL`
with `cpu.stat` at the limit, or `memory.events` `oom_kill` incremented) becomes
`FaceRenderError::Budget(Cpu | Memory)`, mapped to `RefreshOutcome::NeedsAttention` so
it waits the **full cadence** and never enters the 60 s retry doubling. `face_status`
says "This face exceeded its processing allowance and was stopped." The last-good frame
stays. The same applies to built-ins, which have no ledger but can still be killed.

**Staged taps stay out of the ledger** because they cost no child (.001 CPU-s per
exchange, measured). The one bound they need is the fix in section 3: skip the
exchange for a kind without a selector, so a plugin tap cannot hold the shared worker
for its measured 91-191 ms.

## 6. Operator safety settings, separate from entitlements

Proposed environment settings, owned by public code, **off by default in SelfHosted**,
mandatory in the hosted unit. They are not allowances and no account class changes
them.

| Setting | Hosted | SelfHosted default |
|---|---|---|
| `DESKMATE_FACES_CHILD_CPU_SECONDS` | 10 (soft) / 11 (hard) | 0 = off |
| `DESKMATE_FACES_CHILD_MEMORY_MIB` | 256 for render/views/describe, 128 for the selector worker | 0 = off; a positive value needs delegated cgroup v2 or the server warns once and makes no memory claim |
| `DESKMATE_FACES_POOL_CPU_PERCENT` | 50 | 0 = no quota |
| `DESKMATE_FACES_WORKERS` | 1 background child at a time | 1 once the dispatcher exists |
| `DESKMATE_IMAGE_ATTEMPTS_PER_SECOND` | 25, burst 1, no queue | 25 |
| Service `MemoryMax` (unit file, not env) | 1.5 GiB | unset |

The recipe, with `unsafe_code = "forbid"` kept:

1. Launch Bun through a fixed `/bin/sh` wrapper that takes positional arguments only
   (cgroup path, CPU limit, program, args): `ulimit -Ht $((cpu + 1)) && ulimit -St $cpu
   || exit 70`, write `$$` to `cgroup.procs` when a path is given, `exec "$@"`. Exit 70
   is "supervisor failed", reported loudly; the server then stops hosted dispatch rather
   than run Bun unbounded.
2. Hosted Linux: `Delegate=yes` on the unit (a deploy-file change; confirmed absent
   today), the server in its own leaf, a pool parent with `cpu.max 50000 100000`, one
   empty leaf per child with `memory.max`, `memory.swap.max=0`, `memory.oom.group=1`.
   Read `cpu.stat` before removing the leaf; that is the settlement figure.
3. macOS self-host: the QuickJS heap, the wall deadlines and optional `ulimit -t`. No
   hard memory claim is made there, and the docs must not imply one.
4. Do not use `RLIMIT_AS`: Bun reserves address space far above its RSS.
5. Admission before `spawn_blocking`, never after, so a semaphore cannot hide an
   unbounded queue of waiting threads.

The 10 CPU-s guard is **deliberately stricter than the sum of today's legal guards**
(up to 8 s of sandbox wall time plus native raster plus discovery). A plugin that is
legal today can be stopped by it; it then shows as a budget stop, not a crash.

The `MemoryMax` figure is informed by the observed 555 MiB peak with one account's
eight plugin sources, not derived from it. Measured children sit at 65-107 MiB and the
retained selector at 73-88 MiB, so the 256/128 MiB child limits leave room for legal
work seen so far; it must be re-read after the pilot. Revision 5's 1 GiB
split (512 server + 256 + 128 + 128) is withdrawn.

## 7. The pilot, re-derived

Five free accounts and one paid: **8 panels, 28 sources, 13 plugins, 64 cards**, the
remaining 15 slots built-ins or external producers. An operator admission ceiling, not a
service level.

| Load | CPU, at 900 s | CPU, everything at the 60 s floor |
|---|---:|---:|
| 13 plugins (.85) | .0123 | .184, ledger-capped at .15 (5 x .01 + .10) |
| 15 paging built-ins (1.05) | .0175 | .263 |
| `describe` once a minute (.30, fifteen faces) | .0050 | .0050 |
| **Automatic total** | **.035** | **.418** |
| Stress: 13 plugins at the 5.8 s render | .084 | ledger-capped at .15 |

At 900 s the pilot is about 3.5% of one CPU, consistent with the live .0082 for one
account's eight plugins, six of them at 21,600 s. The floor case fits the .5 pool with
.08 to spare for taps and the selector, and only because the ledger caps plugins;
fifteen paging built-ins at the floor are the largest unbudgeted load, which is why
the global pool and fair scheduling matter more than any account cap. The .5 CPU pool is sized for the floor case and for taps. Memory: 28
sources' frames at the 15-per-account ceiling are 6 x 4,946,100 = 29.7 MB; the 8
panels about 32 MiB active; the children as in section 2. External `POST` at 1 per 5 s
on all 28 slots would write 159.5 GB a day of frame bytes, which no one has promised
the disk can sustain; it is the ceiling of the pacing, not a target.

Unmeasured and therefore not claimed: multi-account saturation, the tunnel's throughput
for staged frames, the fifteen-face `describe`, disk pressure, and anything after a
single account's eighth source.

## 8. AccountPolicy and the enforcement seams

Public `entitlements.rs` grows one method and one type and loses one:

```rust
pub trait Entitlements: Send + Sync + 'static {
    fn policy(&self, account: &AccountId) -> AccountPolicy;
}

pub struct AccountPolicy {
    pub revision: u64,
    pub max_panels: Option<usize>,
    pub max_cards: usize,                       // per panel; min(policy, MAX_CONFIG_CARDS)
    pub max_image_sources: usize,               // min(policy, MAX_IMAGE_SOURCES)
    pub max_plugin_sources: Option<usize>,
    pub min_plugin_cadence: Duration,
    pub render_fallback_spacing: Option<Duration>,
    pub plugin_cpu: Option<CpuBucket>,          // refill_micros_per_second, burst_micros
    pub post_account: Option<AttemptBucket>,    // refill_count, refill_period, burst
}
```

`SelfHosted` returns revision 0, `None`, 8, 8, `None`, 60 s, `None`, `None`, `None`.
Free: `Some(1)`, 8, 4, `Some(1)`, 900 s, `Some(5 s)`, `Some((10_000, 10_000_000))`,
`Some((4, 5 s, 4))`. Paid differs in `Some(3)`, 8, `Some(8)`, 60 s, refill 100,000,
`Some((8, 5 s, 8))`. `feature_enabled` goes: it has no production caller
(`tests/accounts.rs` only). The snapshot is immutable, answered without I/O, and
re-read at every serialised admission; a policy change and a mutation share the
account's admission lock so a stale snapshot cannot mint through a reduction.

| Boundary | Seam | Size |
|---|---|---|
| Panels, cards | `claim.rs:94`, `app_api/mod.rs:562`: read the snapshot; say "This panel can have at most N cards" | small |
| Image sources | Pass the effective cap into `ImageSourceStore::mint` and check under its mutex; delete the pre-check at `images.rs:262`. Concurrent last-slot callers yield exactly one success | small |
| Plugin sources | Origin from `describe` (R1); count under the same serialised source admission; a catalog replacement that changes a kind's origin triggers re-count | medium; Track B owns the catalog half |
| Plugin cadence floor | `worker::clamped_refresh` takes the policy floor; applied at schedule time, after edit and on restart | small |
| CPU ledger | New module beside `worker.rs`; lease before every plugin child, settle after; persisted | medium |
| Fallback spacing | `refresh_loop`'s tap branch | small |
| External `POST` | Authenticate and admit before `PngBody` buffers; per-source and per-account buckets; keep the write pacing as is | medium |
| Child supervision | `faces_package::run` and `Selector::start`: the wrapper, cgroups, settlement, typed budget stops | larger |
| Staging before drawing | `stage_other_views`: ask the store for a slot before rendering the page | small, C1's file |

Three faces-package changes carry most of the saving and none of them touch Rust
types in an incompatible way (`CatalogFace` has no `deny_unknown_fields`):

- **R1 -- capability flags in `describe`**: `origin: "builtin" | "plugin"`, `views:
  bool`, `selector: bool`. The server skips the views child when `views` is false
  (every plugin today: refresh drops from .85 to one render, .40-.55) and skips the selector exchange
  when `selector` is false (every plugin today: the shared worker is never held for a
  plugin tap). Missing flags from an older package mean "true", preserving dispatch.
- **R2 -- dispatch one folder**: the server passes the resolved plugin folder from its
  own catalog; a render, views or tap child verifies and loads that folder only.
  Removes the eleven-folder discovery from every job child, about .08 CPU-s each as
  measured; `describe` alone keeps it.
  The folder must come from the server's catalog, never from a request, and be checked
  for containment in the plugin root.
- **Describe cadence**: the once-a-minute catalog re-read is the largest live cost today.
  Re-reading on a directory mtime change, or every five minutes, is a one-line
  operator setting; it changes how fast a `--faces-only` deploy is noticed and nothing
  else. Owner's call on the interval; the default stays at a minute until then.

## 9. The VM re-measurement, run 2026-10-04

M1-M3 ran on the live VM at about 18:05 UTC; their results are in section 2. M4 is a
24 h window and is still open. Method, which mirrors Vault's 2026-10-01 bundle:
`git archive HEAD companion/faces` (identical to the live `5c44245` faces tree, archive
sha256 `643ee7e6...47eb8e`) extracted to a temporary directory; the installed
`node_modules` copied from `/var/lib/private/deskmate/faces`; only the two HTTP adapter
exports in `kit/http.ts` replaced with offline fixtures, the Hacker News one serving
`test/hn-front-page.captured.json`; the real `main.ts` driven in fresh processes and as
one `tap-worker`, under `systemd-run` as `nobody` with `PrivateNetwork`,
`ProtectSystem=strict`, `ProtectHome`, `NoNewPrivileges`, `CPUQuota=50%`,
`MemoryMax=768M`. Children received a readable empty denylist through
`DESKMATE_PLUGIN_DENYLIST`, as the server passes its own: the first attempt used the
live path, which `nobody` cannot read, and an unreadable denylist correctly withdraws
every plugin. Temporary files and the unit were removed; production was not touched.
Harness: `assets/2026-10-04-limits-measurement/measure6.py`; raw JSON lines beside it.

What each item covered:

- **M1 -- per-verb children at fifteen faces.** Done: `describe`, weather render and
  views, `github-stats` render and views, `ink-landscape` render, views and one-shot tap.
  Plugin children rose about .08 CPU-s against the five-face catalog; that is what R2
  saves at eleven folders, and it grows with every plugin added.
- **M2 -- the retained worker.** Done: start-up, ten built-in taps, ten plugin taps,
  five built-in taps after. The plugin taps held the worker 91 ms median and 191 ms
  worst, which makes R1 a prerequisite for a second hosted account, not merely
  desirable.
- **M3 -- a paging built-in refresh end to end.** Done for Hacker News: render, views
  and the three staged pages, .98-1.04 CPU-s against the .95 extrapolation.
- **M4 -- 24 h of the service's own accounting.** Open. The window started at the
  17:57:27 UTC restart; read it on or after 2026-10-05 17:57 UTC with
  `systemctl show deskmate-server -p MemoryPeak -p CPUUsageNSec -p ActiveEnterTimestamp`,
  and only if `ActiveEnterTimestamp` has not moved.

Not in this session: OS kill behaviour, overshoot, cgroup delegation, OOM. Those are
implementation gates (M2/M3 in revision 5's sense) and run when the supervision code
exists.

## 10. Public and private

The public repository is the complete server: accounts, claiming, the window, previews,
all three card kinds, the four built-in faces, external PNG, local plugin loading and
the sandbox, storage, and every operator safety setting in section 6. `ServerOptions`
already composes `edition` and `entitlements`; the hosted executable is a private crate
that depends on the public `server` library at a reviewed commit and supplies the
hosted `Entitlements` with its grant store. No public manifest, lockfile or CI job
names that crate. No licence key, activation call or credential is needed to build or
start the public repo.

A self-hoster who reaches a structural limit gets the structural message ("This panel
can have at most 8 cards", "This account can have at most 8 picture sources") and never
an upgrade prompt, because `SelfHosted` has no cap below the structure.

`docs/self-host.md:14` is corrected in the first implementation change to: "External
producers keep one frame per source. Server-rendered sources stage up to four views
each, and an account's staged set is bounded at fifteen frames in RAM and on disk. That
is a server bound, not a promise about what is resident on the panel."

## 11. What Track B needs from this

Two things, both in section 8: the plugin-source count and the per-account CPU ledger
are the "hosted allowances" its row waits on, and R1's `origin` flag is how the server
tells a plugin from a built-in without trusting `refresh_seconds`. B owns `describe`
and the plugin folder; A owns the policy, the ledger and the admission seams.

## 12. First implementation task, and the gates

After acceptance, the first task is the one every review has named: `AccountPolicy` in
`entitlements.rs` with `SelfHosted` returning exactly today's values; route the panel,
card and source checks through it; move the source cap inside `mint`'s lock; fix the
card error wording and the self-host paragraph. It installs no hosted table, no
supervision and no rate limit, changes nothing for the live server or a self-hoster,
and gives Track B its hook. Then, as separately reviewable changes: R1 and R2, the
ledger, fallback spacing, `POST` admission, supervision.

Gates, beyond the workspace suite: policy equivalence for `SelfHosted` (byte-identical
behaviour, proven by the existing tests passing unchanged); a concurrent last-slot mint
test; R1 flag parsing against an older package; R2 containment and a 1/4/11-folder
cost comparison; a plugin tap that never touches the selector; budget stops waiting the
full cadence and showing in `face_status`; ledger settlement across a restart; and the
C1 bench (`latency_bench.rs`) run with the pool quota on, to show the staged median has
not moved. Hosted supervision cannot launch until a kill, an OOM and a delegation
failure have each been observed under the real unit.

## Verification of this revision

Source read at `70a0639`: `entitlements.rs`, `claim.rs`, `images.rs`, `image_sources.rs`,
`data_cards.rs`, `data_cards/worker.rs`, `data_cards/faces_package.rs`,
`faces_package/selector.rs`, `device_link.rs`, `admin.rs`, `app_api/mod.rs`, the deploy
unit, `faces/src/main.ts`, `registry.ts`, `tap-worker.ts`, `plugins/discovery.ts`,
`plugins/run.ts`, and every `plugin.json`. Live VM inventory and the service journal read
over `ssh`; `deploy.sh --status`. M1-M3 measured on the VM as section 9 describes, in
a transient unit, with production untouched; the 17:57 UTC service restart was not
caused by this session. Arithmetic recomputed from the measured inputs. No application
code changed, nothing deployed.
