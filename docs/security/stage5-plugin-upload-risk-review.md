# Stage 5 Risk Review — Public Plugin Uploads and Sandboxing

Reviewed: 2026-09-08, branch `chore/debt-cleanup` at `5ca7a64`, against the shipped
stage 3b/4 plugin pipeline.

Assignment: `docs/superpowers/plans/2026-09-01-deskmate-plugin-upload-risk-review.md`.
Architecture: `docs/superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md`
§5. Contracts: `docs/plugins/manifest-v1.md` (frozen), `docs/plugins/manifest-v2.md`.

## What this document is, and what it does not authorize

**This is a decision record. It requires explicit owner approval before any stage-5
implementation planning begins, and it is not itself that planning.** Stage 4's exit
criterion states the gate; the review plan restates it: stage 5 "does not begin from this
exit criterion. It begins only after its separate risk review and explicit owner
approval."

Nothing here authorizes: an upload endpoint, an uploader account system, billing, user
fonts, user-supplied image bytes reaching a device, a shared plugin catalog, a sandbox
implementation, a new dependency, a new capability bit, a schema or protocol change, or
any deployment change to `deskmate.rodi.one`. No code was written for this review and no
code should be written from it. Implementation planning starts only from an approved
scope, as its own plan.

It also does not reopen settled negatives: the headless browser stays rejected
permanently (spec §5), `resvg` stays the only rasterizer, and provisioning stays a cable
operation.

## Threat model

Everything shipped assumes **curated** content. The operator who places a manifest under
`DESKMATE_PLUGINS_DIR` is trusted not to be adversarial, and every bound in the pipeline
is defence in depth behind that assumption — `companion/crates/server/src/plugin_provider.rs:66`
says so in as many words: "Spec §5 also notes v1 ships only 'a curated set, not public
uploads,' which is the actual mitigation for a hostile-CPU plugin at this stage: every
manifest is first-party."

Public uploads invert that. The manifest author becomes the attacker. The review's
question in every section below is therefore not "is there a bound?" but "does this bound
hold when the author is trying to break it, and what is behind it when it fails?"

Two facts from this repository's own history set the humility level, and both are cited
rather than asserted:

1. **The arbitrary-file-read via `[[assets]] file` was found in curated content
   handling.** The fix is two independent layers — `validate_asset_file_path`
   (`companion/crates/plugin/src/manifest.rs:696`, a single `Component::Normal` and
   nothing else, via `path_is_single_normal_component` at `:716`) and
   `ensure_within_base_dir` (`companion/crates/plugin/src/assets.rs:258`, a
   `fs::canonicalize` containment check at `:263`, deliberately not a lexical
   `starts_with` because a symlink inside the plugin directory resolves before the
   check). Two of the fourteen tested attack vectors are caught **only** by the second
   layer; the surviving regression test for that case is
   `assets.rs:611`. A bound that looks complete can be incomplete in exactly the way
   nobody modelled.
2. **A hostile TTF would reach two consumers, not one.** Server-side, every plugin-declared
   asset is loaded into usvg's `fontdb` at
   `companion/crates/server/src/rasterizer.rs:575-577`, and hostile bytes are additionally
   parsed once *before* that, by `font_family` at `rasterizer.rs:782-790`, which builds a
   throwaway `fontdb::Database` purely to read the family name. Device-side,
   `firmware/main/ui/font_registry.c:149` hands raw asset bytes to
   `lv_tiny_ttf_create_data_ex` — LVGL's `stb_truetype`-derived parser, in C, on an
   ESP32-S3 with no MMU isolation, no crash containment, and a rollback window that treats
   a boot fault as a firmware problem. That parser was never designed against hostile
   fonts and today only ever receives curated bytes.

---

## R1 — Identity, authorization, and blast radius

### The threat

An upload endpoint is a write primitive into the render host. Whoever holds the
credential that reaches it can place bytes on the server's filesystem (or in its registry)
that the server will then parse, evaluate, fetch on behalf of, rasterize, and push to
devices. "Who may upload" therefore decides the entire blast radius of every other
section: without a principal, there is nobody to quota, nobody to attribute a fetch to,
nobody to revoke, and nobody to quarantine content to.

### The existing mitigation

There is exactly one human-facing credential in the server, and no notion of an uploader
at all.

- `DESKMATE_ADMIN_TOKEN` is mandatory at startup (`companion/crates/server/src/main.rs:62`,
  which `expect`s it) and is compared in constant time through the single sanctioned path
  `ServerState::verify_admin_token` (`companion/crates/server/src/lib.rs:199`, whose own
  doc calls it "the only sanctioned way to check the admin token, preserving the
  constant-time-comparison guarantee for the one secret that protects every write").
- Every admin route sits behind the `AdminAuthenticated` extractor
  (`companion/crates/server/src/admin.rs:122-142`), and the rejection is deliberately
  featureless — bare 401, no body, no logging of the presented credential
  (`companion/crates/server/src/auth.rs:57-61`).
- The registry is **loaded once at process start** from a filesystem directory
  (`PluginRegistry::load`, `companion/crates/server/src/plugin_registry.rs:125`; sole
  production caller `main.rs:121`). There is no upload route, no reload route, and no
  runtime mutation of the plugin set. Putting a manifest on the server is an SSH-and-restart
  operation today.
- Device identities exist and persist as SHA-256 digests, never plaintext tokens — but
  they identify **devices**, not people, and carry no owner attribute.
- Registry-wide ceilings exist: `MAX_PLUGINS = 32` (`plugin_registry.rs:23`) and
  `MAX_DURABLE_REGISTRY_ASSETS = MAX_ASSET_DIGESTS - 1 = 31` (`:31`, enforced at `:189`,
  reserving one slot for the volatile raster frame).

**There is no per-uploader quota, no per-uploader accounting, no attribution, and no
mechanism to revoke one uploader without rotating the single admin token** — which would
simultaneously de-authenticate the owner, every operator script, and (post-V3) the
operator session cookie, since that cookie is HMAC-signed with the same admin token.

### The gap under a hostile author

The gap is categorical rather than incremental: **the concepts stage 5 needs do not
exist.** "Per-uploader quotas", "per-uploader device quarantine", "identity recycling",
and "takedown of one uploader" are all statements about a principal the system cannot
name. Every one of them is unimplementable as a policy tweak; each requires an identity
model first.

The plan asks whether V3 supplies it. **It does not, and this is the review's single most
consequential finding.** The roadmap row for V3 reads "Accounts, OAuth-held integration
credentials …, config storage, and multi-tenancy"
(`docs/superpowers/plans/2026-08-03-deskmate-roadmap.md:37`), but the V3 design that is
actually being executed narrowed that scope. On branch `feat/v3-server-host`,
`docs/superpowers/specs/2026-09-06-deskmate-v3-server-host-design.md:29` describes the
surface as "Admin-token gated, single operator", and `:31-34` lists what V3 explicitly
**does not deliver**: "User accounts and signup; per-user multi-tenancy or data isolation;
billing; the plugin-upload sandbox". §7 of that spec (`:288-297`) is titled "Where
multi-tenancy slots in (deferred, not built)" and reserves an `owner_id` seam defaulted to
a single constant. The delivered work matches: the operator session cookie in the OAuth
sub-project is HMAC-SHA256 keyed by `DESKMATE_ADMIN_TOKEN` — one operator, the owner.

So the premise "after V3 delivers identity" is false as written. Identity for *uploaders*
is deferred past V3, to an unplanned multi-tenant step.

### Decision options

| | Option | Assessment |
|---|---|---|
| A | **Owner-only uploads**, admin-token authenticated. | Sound, and the only option available today — but see the recommendation: it is functionally the status quo with a new attack surface bolted on. |
| B | **Signed invites** — the owner mints a per-uploader bearer capability, revocable independently of the admin token. | The minimum honest unit of attribution and revocation. Roughly a day of work: a capability record, a mint route, a verify extractor, a revoke route. It does *not* give data isolation, only naming. |
| C | **Full accounts.** | Requires the multi-tenant step V3 §7 defers. Not available, and not schedulable from here. |

**RECOMMENDATION: Option A, and do not build the endpoint.** The reasoning is the point
of the section. Under owner-only authentication, an "upload" is the owner choosing to
place third-party content in the registry — which is precisely what the owner can do today
with `scp` and a restart. The endpoint adds an unauthenticated-adjacent network write path,
a new parse surface (multipart, archive extraction, path handling — the exact class that
produced the `[[assets]] file` finding), and a runtime registry-mutation path that does not
exist today, in exchange for saving one `scp`. That is a strictly negative security trade.

If third-party manifests are wanted **now**, the correct mechanism is a documented
submission process: an author sends a manifest, the owner reads it, and the owner installs
it — with every R2/R3/R4 hardening below applied, because owner *approval* is not owner
*audit*, and a human reading a 64 KiB TOML file will not catch a hostile font.

Adopt Option B's shape without its plumbing: **thread a single-valued `uploader_id` through
the registry and any future asset accounting now**, exactly as V3 §7 reserves `owner_id`,
so that the multi-tenant step is additive rather than a rewrite.

**Hostile paying users are out of scope and should be declared so.** Billing is V4; there
are no accounts to bill; refunds, takedowns and identity recycling are questions about a
subsystem that does not exist. Answering them here would be speculative design.

### Evidence that would close it

- A written owner decision on the submission process (mechanism, not code).
- A multi-tenancy design (V3 §7's deferred step) reaching approved status, with an
  `uploader_id` present end to end: registry record, asset desired-set, egress log line,
  render accounting.
- A test asserting that revoking one uploader capability leaves the admin token and every
  other uploader working — the property that distinguishes B from A.

---

## R2 — The upload payload itself

### The threat

Hostile bytes reaching four parsers of increasing consequence: the TOML parser and the
hand-rolled pre-scanner in front of it; the expression grammar; the SVG/XML stack
(`roxmltree`, `usvg`, `resvg`); and — the sharp edge — the **font** and **image** parsers,
one pair on the server and one pair on the device.

### The existing mitigation

**Manifest shape and size** (`companion/crates/plugin/src/manifest.rs`) — every table is
`#[serde(deny_unknown_fields)]` (`:326`, `:356`, `:367`, `:378`, `:405`, `:435`, `:442`,
`:458`), and the bounds are named constants checked in `validate()`:

| Bound | Value | Line |
|---|---|---|
| `MAX_MANIFEST_BYTES` | 64 KiB, checked before parsing | `:38`, enforced `:973` |
| `MAX_TOML_NESTING_DEPTH` | 16, checked by a byte scanner **before** the TOML parser runs | `:107`, `check_nesting_depth` `:543`, `:564` |
| `MAX_NODES` | `protocol::MAX_SCENE_NODES` = 24 | `:43` |
| `MAX_ASSETS` | 16 | `:46` |
| `MAX_GLYPHS_PER_ICON_FONT` | 256 | `:49` |
| `MAX_EXPR_SOURCE_LEN` | 256 | `:57` |
| `MIN`/`MAX_REFRESH_MINUTES` | 1 / 1440 | `:62`, `:64` |
| `MAX_URL_LEN` / `ALLOWED_URL_SCHEME` | 512 / `https` | `:73`, `:87` |
| `MAX_REPEAT_GROUPS` | 2 | `:114` |
| `MAX_SVG_SOURCE_BYTES` / `MAX_EXPANDED_SVG_BYTES` | 256 KiB / 512 KiB | `:127`, `:131` |

`parse_manifest_bytes` (`:1069`) rejects non-UTF-8 before the size check, deliberately, so
the error names the right reason.

**Expression language** (`companion/crates/plugin/src/expr.rs`) — `MAX_SOURCE_LEN` 512
(`:72`), `MAX_DEPTH` 16 enforced at *parse* time in the recursive-descent parser
(`:79`, `:431-435`), `MAX_PATH_SEGMENTS` 16 (`:84`, `:561`), a shared `FUEL_BUDGET` of
10,000 threaded across every node of one compile (`:93`, consumed per node at `:688` and
per path segment at `:714`), `MAX_OUTPUT_LEN` 4096 (`:98`), `MAX_ROUND_PLACES` 12 (`:103`).
`compile::MAX_REPEAT_ITEMS` is 5 (`companion/crates/plugin/src/compile.rs:62`, applied
`:819`). There is no code in a manifest; it is a closed expression grammar over fetched
data.

**Assets** (`companion/crates/plugin/src/assets.rs`) — `MAX_ASSET_BYTES` is
`protocol::MAX_ASSET_TOTAL_LENGTH` = 1 MiB (`:75`), read through `read_bounded` (`:234`)
which reads cap+1 so an oversized *or sparse* file is rejected without reading it whole
(test `:561`). Containment is the two-layer arrangement described in the threat model
above.

**SVG** (`companion/crates/server/src/rasterizer.rs`) — DTD/entity expansion is forbidden
(`:58`), and this is load-bearing rather than belt-and-braces: usvg 0.45.1 parses with
`allow_dtd: true`, so entity expansion is genuinely reachable. `MAX_XML_DEPTH` 64 (`:30`),
`MAX_XML_ELEMENT_NODES` 4096 (`:32`), `MAX_EXPANDED_SVG_NODES` 8192 (`:34`),
`MAX_SVG_DIMENSION` 4096 (`:36`), all enforced in `preflight` (`:599`, `:633`, `:639`) and
`validate_dimensions` (`:653`). `validate_reference` (`:708`) rejects `http:`/`https:`/`//`
(`:715`), `file:` (`:720`), every `data:` from plugin-authored SVG (`:723` — only
module-generated documents may embed module-generated image data), and treats a bare
relative URL as a filesystem request (`:729`). `renderer_options` (`:542`) replaces both
usvg `ImageHrefResolver` callbacks with deny-all closures (`:554-564`) — necessary because
usvg's default string resolver reads local files even with `resources_dir: None` — and
never calls `load_system_fonts`, so the fontdb has no ambient host access (`:569-571`).

**Testing that exists:** `companion/crates/plugin/tests/hostile_manifest.rs` — 10 MB
manifest, deeply nested tables, duplicate assets, surrogate-range and above-ceiling glyph
codepoints, zero refresh, `file:` scheme URL, non-UTF-8 bytes. That is **eight tests**,
each a hand-picked case.

### The gap under a hostile author

**1. There is no fuzzing anywhere in this repository.** A `find` for any `fuzz` target,
directory, or corpus across the whole tree returns nothing. Every parser above is
protected by hand-written bounds and hand-picked hostile cases, which is exactly the
posture that produced the `[[assets]] file` finding. Three surfaces are unfuzzed and
should not face hostile input as they are:

- `parse_manifest_bytes` end to end, including `serde`/`toml` interaction.
- `check_nesting_depth` and its helper `skip_string` (`manifest.rs:584`) — a **hand-rolled
  lexer standing in front of the real parser**, whose whole job is to be conservative
  about depth. Its own doc admits it is "deliberately not a full TOML lexer". A
  disagreement between this scanner's idea of "inside a string" and the TOML crate's is a
  bypass of the only pre-parse depth guard, and nothing tests for that disagreement
  systematically.
- `Expr::parse` + `eval`, and the `preflight`/`evaluate_svg_template` path.

The `toml` crate's own DoS surface (huge keys, wide tables, pathological escapes) is
bounded only indirectly, by the 64 KiB size cap. That cap is probably sufficient, but
"probably" is the word doing the work, and no measurement exists.

**2. Fonts are the sharp edge, and the exposure is worse than "the server parses a
font".** Under public uploads a hostile TTF is parsed *three* times on paths that already
exist:

- `rasterizer.rs:782-790` (`font_family`) builds a fontdb and reads the first face —
  before any policy decision, purely to learn the family name for `validate_font_family`.
- `rasterizer.rs:575-577` loads every plugin asset into the render fontdb.
- `firmware/main/ui/font_registry.c:149` calls `lv_tiny_ttf_create_data_ex` on the raw
  asset bytes on the device.

The server-side consumers are Rust (`ttf-parser`/`fontdb`), so the realistic failure is a
panic or a resource blow-up rather than memory corruption — but see R3: a panic in-process
is not contained, and the process holds device tokens and, post-V3, the decrypted secrets
master key. The device-side consumer is C, on hardware with no MMU isolation, where a
fault is a panic-and-reboot at best. `font_registry.c` bounds the *bookkeeping* (open
faces, digests, pixel sizes); it does not and cannot validate the font's internal tables.

**3. Images: the device validates less than it appears to.** `build_image`
(`firmware/main/ui/scene_view.c:780`) checks three things — the asset kind is
`ASSET_KIND_IMAGE`, the length exceeds `sizeof(lv_image_header_t)` (`:792-793`), and the
header magic equals `LV_IMAGE_HEADER_MAGIC` (`:812`). It then points an `lv_image_dsc_t`
directly at the mapped bytes (`:807-810`) and hands it to LVGL. It sets the object's size
from the **scene node** rather than the header (`:826-829`, with a comment saying a
mismatched blob "is clipped to the declared box instead of silently overflowing its
neighbours"). That comment is about *layout*. It bounds the drawn rectangle, not the
source walk: LVGL's blit reads pixels using the header's own `w`/`h`/`stride`/`cf`, and
**nothing checks that `w * h * bytes_per_pixel` fits inside `data_size`.** A crafted header
with a large declared width over a short payload is an out-of-bounds read inside the LVGL
draw path. That is a live gap for *curated* content too — today it is unreachable only
because every image blob is produced by the server's own rasterizer, whose output is the
canonical 12-byte LE header plus exactly 448×368 RGB565 (329,740 bytes).

**4. A redirect can downgrade an https-only source.** `manifest.rs:87` pins
`ALLOWED_URL_SCHEME = "https"` for a manifest's declared source URL, but every redirect
hop is re-validated by `egress_guard`, which accepts `http` as well
(`companion/crates/server/src/egress.rs:472-475`). Under curated content the fetched
payload is untrusted anyway, so this is minor; under public uploads it means an author can
route the server's own fetches over plaintext and hand any on-path party control of the
data feeding the expression evaluator. Worth closing because it is a one-line policy, not
because it is severe.

### Decision options

| | Option | Assessment |
|---|---|---|
| A | **Accept user fonts with server-side validation** (parse, re-emit through a trusted subsetter). | Reject. Re-emitting still requires parsing the hostile input, and the device consumer remains a C parser fed bytes an attacker chose the *shape* of. A font subsetter is itself a large parser. |
| B | **Curated font whitelist only; user fonts excluded outright.** | The plan's own suggested recommendation. |
| C | **Images: accept user bytes with header validation on the device.** | Necessary but insufficient alone — it makes the device correct, and leaves the server pushing attacker-chosen blobs. |
| D | **Images: mandatory server-side re-encode** — decode through one trusted path, re-emit canonical RGB565, attacker bytes never reach a device. | The plan's suggestion. |

**RECOMMENDATION: B and D together, plus C anyway, plus fuzzing as a precondition.**

- **User fonts: excluded outright.** Agree with the plan, and the reason is stronger than
  the plan states: the exclusion is not about the server's fontdb (Rust, panics, survivable
  with R3's subprocess) but about `lv_tiny_ttf` on the board. There is no sandbox available
  on an ESP32-S3, no crash isolation, and a bricked panel is not recoverable by restarting
  a service. A plugin gets `tools/fonts/Inter-Regular.ttf` and `Inter-SemiBold.ttf`
  (`rasterizer.rs:23-24`, SHA-256 pinned) and the curated icon font, or it gets no glyphs.
  Revisit only if a memory-safe device-side font parser ever exists, which is not planned.
- **Images: server-side re-encode is mandatory, and should apply to curated content too.**
  Agree with the plan and extend it. The pipeline already produces exactly one canonical
  image form; a public plugin's image should be decoded by the server and re-emitted
  through that same path so the digest the device receives is over server-generated bytes.
  Note the honest limitation: this moves the hostile decode into the server, so it must sit
  inside R3's subprocess and be restricted to one format with one decoder.
- **C anyway, independent of D.** Add the `w * h * bpp <= data_size` invariant to
  `build_image` and host-test it in `firmware/host_tests`. Re-encoding is a *server-side*
  control, and the device must not depend on the server being correct — that is this
  project's stated posture everywhere else ("treat all bytes received from the host as
  untrusted"). This one is worth doing regardless of whether stage 5 ever happens, and it
  carries the usual firmware cost: `.bss` measurement and an on-board OTA download
  re-verification.
- **Fuzzing is a precondition, not evidence-after-the-fact.** No hostile manifest should
  reach these parsers before they have been fuzzed.

Also close the redirect-scheme downgrade: require `https` on every hop for plugin fetches,
or state in `manifest-v2.md` that the https pin is advisory. A one-line policy either way.

### Evidence that would close it

- `cargo-fuzz` targets on `parse_manifest_bytes`, `check_nesting_depth` **differentially
  against the real `toml` parser's own depth behaviour**, `Expr::parse` + `eval` (asserting
  fuel/depth/output invariants hold, not merely that it does not crash), and
  `rasterizer::preflight`. Corpus seeded from `companion/plugins/*/manifest.toml`, the
  `crates/plugin/tests/fixtures/` bodies, and every case in `hostile_manifest.rs`. Run to a
  stated exec count under ASan, with the corpus committed and the targets in CI as a
  short-budget smoke run.
- A host test in `firmware/host_tests` proving `build_image` rejects a header whose
  declared geometry exceeds its payload, and an ASan run over it — the same argument that
  made `make -C firmware/host_tests sanitize` non-optional for the scene decoder.
- A recorded decision in `docs/plugins/manifest-v2.md` (or v3) that `kind = "font"` and
  `kind = "icon-font"` assets are registry-curated only, machine-checked the way
  `crates/plugin/tests/manifest_v2_doc.rs` already checks that document's bounds section.
- A measurement, not an assumption, of the `toml` crate's worst case within 64 KiB.

---

## R3 — Execution and resource isolation on the server

### The threat

CPU and memory exhaustion; a crash or panic in a parser taking down the device link along
with it; and — the one that matters most — an in-process memory-safety or logic
compromise reached through `resvg`/`usvg`/`roxmltree`/`ttf-parser`/`toml`, in a process
that holds device bearer material and, as of V3, the decrypted OAuth secrets master key.

### The existing mitigation

**Refresh rate** is enforced twice: at parse time by `manifest.rs:851` and again where a
`Duration` is actually built, `refresh_interval_from_minutes`
(`companion/crates/server/src/plugin_provider.rs:495-510`).

**Response memory** is bounded by `egress::MAX_RESPONSE_BODY_BYTES` = 2 MiB
(`companion/crates/server/src/egress.rs:108`), and a body over that cap classifies as a
*permanent* card fault rather than a retry loop.

**Render wall-clock** is `MAX_RENDER_WALL_CLOCK = 250 ms` (`plugin_provider.rs:525`),
checked by `within_wall_clock_budget` (`:530`) and threaded through the rasterizer as
`RenderDeadline` (`rasterizer.rs:581-597`), which is polled at every font load, every
escape, and every base64 chunk.

**CPU is not enforced, and the code says so.** `plugin_provider.rs:55-68`, verbatim:

> **CPU is not enforced here, and cannot honestly be claimed as enforced.**
> `compile_scene` is a plain synchronous function call with no subprocess or OS-level
> resource limit around it; nothing in this process can preempt or cap the CPU time of a
> `FnOnce` it calls, short of a sandboxed subprocess … What actually keeps a compile's CPU
> cost small in practice is `plugin::compile_scene`'s own structural bounds … Spec §5 also
> notes v1 ships only "a curated set, not public uploads," which is the actual mitigation
> for a hostile-CPU plugin at this stage: every manifest is first-party.

`MAX_RENDER_WALL_CLOCK`'s own doc (`:513-524`) is equally explicit: the budget "measures
after the fact" — a compile that finishes over budget is refused rather than pushed, but a
compile already running past budget cannot be interrupted.

**Egress** is the strongest mitigation in the codebase and deserves saying so plainly.
`companion/crates/server/src/egress.rs` implements resolve-then-pin: exactly one DNS
resolution per hop (`resolve_once` `:526`, `resolve_and_pin` `:560`), the result handed to
`reqwest::ClientBuilder::resolve` so reqwest never re-resolves; `select_pinned_address`
(`:505`) **fails closed if any returned address is denied**, rather than picking a
permitted one from a mixed answer; every redirect hop is re-validated and re-pinned from
scratch, capped at `MAX_REDIRECTS = 5` (`:105`); IP-literal hosts are denied directly
(`deny_literal` `:485`). The deny list is thorough: loopback, link-local, private,
broadcast, multicast, unspecified, `169.254.169.254` by name (`:89`), plus
`EXTRA_SPECIAL_USE_V4` (`:299`) covering `0.0.0.0/8`, **CGNAT `100.64/10` — explicitly
noted as Tailscale's range and the render host's own Tailscale address** — `192.0.0.0/24`,
all three TEST-NETs, 6to4 relay anycast, benchmarking, and Class E; and for v6 (`:373`)
loopback, unspecified, link-local, ULA, deprecated site-local, multicast, documentation,
v4-mapped via `to_ipv4_mapped` (`:277`), and the NAT64 well-known prefix. Budgets:
`RESOLUTION_TIMEOUT` 5 s, `REQUEST_TIMEOUT` 10 s per hop, `TOTAL_FETCH_BUDGET` 20 s for the
whole chain (`:92`, `:95`, `:101`).

### The gap under a hostile author

**1. There is no process isolation, and V3 raised the stakes.** Parsing, evaluation and
rasterization all run in the server process. That process holds device authentication
material and — after V3 sub-project 1, delivered in `994a422..e472113` — a 32-byte
XChaCha20-Poly1305 master key held in memory and the refresh tokens it decrypts. The V3
plan's own threat statement is that encryption at rest "does not defend against a
live-server compromise — a process that can read the keyfile and ciphertext can mint
tokens, because it must, to use the calendar." **Putting a hostile-content parser in that
address space converts every parser bug in `resvg`, `usvg`, `roxmltree`, `ttf-parser` and
`toml` into a path to the owner's Google account.** This single fact changes process
isolation from a hardening measure into a precondition.

**2. There is no memory cap and no real CPU preemption.** The wall-clock budget measures;
it cannot kill. A compile that allocates unboundedly or spins is bounded only by the
structural caps, which is precisely the thing fuzzing exists to test and which nothing has
tested.

**3. There is no admission control.** No concurrency limit, work queue, or per-uploader
fair share was found anywhere in the plugin path. N plugins × a 1-minute floor
(`MIN_REFRESH_MINUTES = 1`, `manifest.rs:62`) × a 250 ms nominal render is a CPU DoS with
no ceiling, and the 250 ms figure is *nominal* — the budget does not enforce it.

**4. There is no crash isolation.** A panic in compile or raster inside a runtime worker
has no documented containment story. "One hostile manifest must not take down the device
link" is not currently a property the architecture provides; it is a property that
currently holds because manifests are first-party.

**5. Egress: three residual items under attacker-chosen URLs.** The guard is strong, and
none of these is a rebinding hole — they are abuse-economics gaps that only appear when the
URL author is hostile.

- **No port restriction.** `egress_guard` (`:470`) checks scheme and host, not port. A
  hostile author can point the server at any port on any *public* address. Private space
  is denied, so this is not SSRF into the homelab; it is the server acting as a connector
  to arbitrary public ports with the owner's IP and Cloudflare-adjacent reputation.
- **No per-uploader egress budget.** At `MIN_REFRESH_MINUTES = 1` and `MAX_PLUGINS = 32`,
  a hostile set of manifests is a sustained request generator attributable to the owner.
- **Repeated fetches re-resolve.** Each fetch is correctly pinned, but a rebinding attacker
  gets a fresh attempt every refresh. The fail-closed mixed-answer rule at `:505` is what
  makes this acceptable; it is worth recording as a deliberate acceptance rather than an
  oversight.

### Decision options

| | Option | Assessment |
|---|---|---|
| A | **In-process, tighter caps.** | Reject. The code itself says CPU "cannot honestly be claimed as enforced" in-process, and V3's secrets make in-process parsing of hostile content indefensible regardless of caps. |
| B | **Thread-based isolation** (rayon pool, panic catching). | Reject. `catch_unwind` does not contain memory corruption, does not cap memory, and does not preempt. It buys crash tidiness and calls it a sandbox. |
| C | **Separate short-lived process** per parse/compile/rasterize, with OS limits. | Recommended. |
| D | **Full container/VM per render.** | Over-scoped for the throughput here (one render per plugin per refresh interval); revisit only if C proves insufficient. |

**RECOMMENDATION: Option C, as a precondition rather than a requirement to be scheduled
later.** Concretely, and stated as a shape rather than an implementation:

- A `deskmate-render` helper binary invoked per job, receiving manifest + fetched payload +
  curated asset bytes on stdin and returning a compiled `Scene` or a raster frame on
  stdout. **Never network.** The parent already owns fetching; the child must not.
- OS limits on the child: `RLIMIT_AS` (a real memory cap, which nothing provides today),
  `RLIMIT_CPU`, `RLIMIT_NOFILE`, `RLIMIT_FSIZE = 0`. Under systemd, additionally
  `PrivateNetwork=yes`, `NoNewPrivileges=yes`, `ProtectSystem=strict`,
  `PrivateTmp=yes`, `MemoryMax=`, `CPUQuota=`, and a read-only bind of exactly one plugin
  directory. seccomp if it can be done without an escape-hatch dependency; the systemd
  properties are the load-bearing part and are configuration, not code.
- **Kill on deadline.** This is the property that converts `MAX_RENDER_WALL_CLOCK` from a
  measurement into an enforcement, and it is worth having for *curated* content too — it
  retires the honest caveat at `plugin_provider.rs:55`.
- **Admission control**: a bounded work queue with a global concurrency cap (cores − 1, so
  the device link always has a scheduler slot) and per-uploader fair share. Shed with the
  existing typed refusal vocabulary; never queue unboundedly.
- **Raise the public refresh floor.** `MIN_REFRESH_MINUTES = 1` is right for a curated
  pomodoro-adjacent card and wrong for public content. Recommend a separate public floor of
  15 minutes, plus a per-uploader daily fetch budget.
- **Add a port allowlist** (`80`/`443` only) to `egress_guard`, and record the
  repeated-resolution acceptance in the module doc alongside the existing rebinding note.

### Evidence that would close it

- A test that runs a deliberately pathological manifest (infinite-loop-shaped expression,
  or a rasterizer input engineered past the budget) and asserts the **parent** returns a
  typed refusal within a bounded time while the device link stays up — i.e. the crash- and
  hang-isolation property asserted directly, not inferred.
- `RLIMIT_AS` proven by a job that allocates past it and is killed, with the parent
  classifying it correctly.
- A load measurement: N concurrent renders at the admission cap, showing the device link's
  scheduler latency unchanged.
- A `systemd` unit diff in `companion/crates/server/deploy/` reviewed against the property
  list above.

---

## R4 — What reaches the fleet

### The threat

A hostile plugin's bytes reaching a device that does not belong to its author; and durable
assets displacing curated ones on devices generally.

### The existing mitigation

The wire validator bounds a compiled scene: `MAX_SCENE_NODES = 24`
(`companion/crates/protocol/src/scene.rs:35`), `MAX_ASSET_TOTAL_LENGTH = 1 MiB` and
`MAX_ASSET_DIGESTS = 32` (`companion/crates/protocol/src/message.rs:65-67`), and
`protocol::validate_message` is the single place the wire's bounds live. The registry
reserves the volatile slot via `MAX_DURABLE_REGISTRY_ASSETS = MAX_ASSET_DIGESTS - 1`
(`plugin_registry.rs:31`, enforced `:189`). Volatile raster frames live in a two-slot
PSRAM store and are transient by construction.

### The gap under a hostile author

**1. `desired_assets()` is registry-wide, not per-device.** In `app-core`,
`synchronize_full` (`companion/crates/app-core/src/runtime.rs:2515`) reconciles assets by
calling `host.desired_assets()` (`:2546`, trait at `:179`, composition at `:3313` and
`:3339`). Every device that connects while the registry is loaded receives **every**
curated plugin's assets. Under public uploads that is fleet-wide distribution of every
uploader's bytes to every device, which is the exact opposite of quarantine. **This means
"per-uploader device quarantine" is an architecture change, not a policy flag** — it
requires `desired_assets()` to become per-device, and that is a real piece of work with a
real regression surface.

**2. `AssetRelease` is a device-wide keep-set, so registry composition is destructive by
design.** `firmware/main/core/asset_store.c`'s compaction marks every committed record
whose digest is *absent* from the list DEAD. `runtime.rs:2536-2546` documents the
consequence and carries the guard that an empty desired set must skip the pass entirely,
because `AssetRelease { digests: [] }` means "wipe every asset you hold" — a defect that
shipped and was caught only by `server/tests/hostile_device.rs` asserting the exact request
sequence. Under public uploads, a plugin appearing or disappearing from the registry
directly rewrites what every device retains, and the 31 durable slots are a **shared**
resource between curated and public content. An uploader publishing 16 assets
(`MAX_ASSETS`, `manifest.rs:46`) across two plugins consumes the entire durable budget.

**3. A shared catalog has no review, signing, or takedown mechanism whatsoever.** There is
no signature over a manifest, no provenance record, no revocation list, and no runtime
removal path (see R5).

### Decision options

| | Option | Assessment |
|---|---|---|
| A | **Quarantine public content to the uploader's own devices.** | The plan's strong recommendation. Requires the `desired_assets()` change above. |
| B | **Shared public catalog.** | Reject for the first iteration — and for any iteration until review, signing, provenance and takedown all exist. |
| C | **Public plugins may install durable assets** under the 31-slot ceiling. | Reject. |
| D | **Public plugins render through the raster/volatile path only; no durable assets at all.** | Recommended. |

**RECOMMENDATION: A and D.**

- **Quarantine — agree strongly with the plan, with a cost correction.** It is not a
  configuration option; it is `desired_assets()` moving from registry-wide to per-device,
  plus an `uploader_id → device_id` mapping that R1 says does not exist. Both are
  prerequisites, and both should be costed before stage 5 is scheduled.
- **No durable public assets — this is the recommendation that makes the rest cheap.** A
  public plugin renders through the existing rasterization fallback into the **volatile**
  two-slot PSRAM store: transient, bounded, replaced on every push, and entirely outside
  the `AssetRelease` keep-set arithmetic. This composes with R2 exactly: if a public plugin
  can install no durable asset, then there is no hostile TTF in device flash, no hostile
  image blob in device flash, the 31 durable slots stay 100% curated, and the destructive
  keep-set semantics never see attacker-influenced membership. It costs public plugins the
  native render path — their faces come from `resvg` with the accepted fidelity deviation
  already documented in `PRODUCT.md` — which is the right trade for content nobody
  reviewed.
- **Shared catalog: defer explicitly**, and record what it would require, so it is not
  re-litigated: a manifest signing scheme, a provenance record per plugin, a human review
  step with a written standard, a revocation list the server checks at load, and R5's
  takedown path. That is a project, not a stage-5 task.

### Evidence that would close it

- A test asserting that a device receives assets only for plugins whose `uploader_id`
  matches its owner — with a second device present in the same test that must receive
  nothing, since the current registry-wide behaviour would pass a single-device test.
- A test asserting that a public plugin's compile/render path emits **no**
  `AssetBegin { volatile: false }` at all, so the durable ceiling is structurally
  unreachable from public content.
- A regression on the existing empty-keep-set guard, run with a public plugin appearing and
  disappearing, proving no curated asset is ever marked DEAD by a public registry change.

---

## R5 — Operations

### The threat

A hostile plugin live on the fleet at 03:00 with the owner asleep; no attribution when
something is noticed; and — the one that cannot be engineered away — a compromise reached
through a plugin parser landing on a host that runs the owner's unrelated personal
services.

### The existing mitigation

- Plugin load failures are surfaced: `PluginLoadFailure` (`plugin_registry.rs:51`),
  logged at startup (`main.rs:132-139`), and exposed through `GET /v1/plugins`.
- Egress denials, body-cap hits and render-budget overruns are typed and classified
  (`classify_plugin_failure` in `plugin_provider.rs`), permanent vs transient.
- Device-facing auth failures are rate-limited in their logging
  (`auth.rs`, `UnknownAuthWarningLimiter`).

**There is no kill switch.** The registry is loaded exactly once, at process start
(`plugin_registry.rs:125`, sole caller `main.rs:121`). Removing a plugin means SSH to
docker-vm, delete or move the directory, restart the systemd unit. That drops every device
WebSocket (they reconnect; runtimes are retained per device and replay state, so it is not
data loss — but it is a service interruption and a manual action).

**There is no per-uploader audit trail.** Nothing attributes a fetch, a render, or a push
to a content author. Everything is per-plugin-id at best, and plugin ids are
author-supplied strings validated only for length and shape
(`validate_plugin_id`, `plugin_registry.rs:265`).

**There are no automatic tripwires.** Repeated render-budget violations, repeated egress
denials, and crash counts are all *classified* but none of them *acts*.

### The gap under a hostile author

The operational gap is that the only response to hostile content is a manual one requiring
the owner to be awake, at a computer, on Tailscale (the VM's LAN address is unreachable
from any other network), with `sudo`. Mean time to takedown is therefore measured in hours
and gated on the owner's sleep schedule. For public content that is not an acceptable
posture, and no amount of pre-publication bounding substitutes for it — the whole point of
R2 and R3 being uncertain is that something will get through.

### The homelab question, stated honestly

**The render host is not dedicated, and this is the residual risk that no sandbox
removes.** `deskmate.rodi.one` runs on docker-vm — the owner's homelab VM — alongside
unrelated self-hosted personal services, behind Cloudflare → cloudflared → Caddy. The
same VM also carries the `trmnl-claude-sync` timer and `/opt/deskmate-feeds`.

The honest statement of residual risk, **even with R3's process isolation fully
implemented**:

- A sandboxed child still shares a **kernel** with every other service on that VM. A
  kernel LPE reached from inside the sandbox lands on a host holding the owner's photos,
  files, and read-later history, plus the Deskmate device registry and the V3 secrets
  keyfile and its ciphertext.
- Container/service boundaries on that VM are Docker-level, not hypervisor-level. They are
  not a security boundary against a local root.
- `PrivateNetwork=yes` on the render child removes the child's network, not the parent's
  reach; the parent still fetches, and the parent is still on the same host as everything
  else.
- The blast radius therefore includes services with **no relationship to Deskmate** and no
  say in this decision.

### Decision options

| | Option | Assessment |
|---|---|---|
| A | **Keep public content on docker-vm with process isolation.** | Reject. The residual risk above is borne by the co-hosted personal services' data that has nothing to do with this project. |
| B | **Dedicated VM or host for the render path** (or a managed sandbox provider). | Recommended, as a precondition. |
| C | **No public content at all.** | The status quo, and the correct answer until B is paid for. |

**RECOMMENDATION: B, decided now as a precondition rather than left as a nice-to-have** —
and C until B exists. The plan asks whether stage 5 "requires dedicated isolation … as a
precondition rather than a nice-to-have". It does. Accepting hostile content on a host that
carries the owner's photo library is not a trade this project is entitled to make on that
data's behalf, and "we sandboxed it" is a claim about a boundary that has been defeated
before.

Alongside it, three operational capabilities that are cheap and useful **even for curated
content**:

1. **A runtime per-plugin `enabled` flag**, flippable through an admin route with no
   restart and no link drop. This is the kill switch, and its absence is a gap today, not
   only under stage 5.
2. **Automatic tripwires that flip that flag**: N consecutive render-budget overruns, N
   subprocess kills or crashes, N egress denials, or a body-cap hit rate over threshold →
   disable the plugin, record the reason, notify. Fail safe: a disabled plugin's cards
   show the existing typed refusal, which the UI already renders.
3. **Structured audit events** carrying `uploader_id`, `plugin_id`, `device_id`, outcome
   and timing for every fetch, compile, render and push — with the existing discipline that
   untrusted strings never reach logs (`docs/security/v1-review.md`'s "Logs" section
   converted every dynamic error to a closed category label; the same rule applies here,
   so audit events carry ids and closed labels, never manifest-derived text).

### Evidence that would close it

- A timed drill: hostile plugin identified → disabled → confirmed no longer rendering on a
  device, with the device link never dropping, in under 60 seconds and without SSH.
- A tripwire test that drives N synthetic budget overruns and asserts the plugin
  auto-disables with the reason recorded.
- A deployment diagram and a written acceptance for whichever host runs public content,
  naming what else runs there — which, for a dedicated host, is "nothing".
- An audit-log sample showing a full fetch → compile → render → push chain attributable to
  one uploader, and a grep proving no manifest-derived string appears in it.

---

## Is stage 5 meaningful before V3 delivers identity?

**No — and the question's premise needs correcting first: V3 does not deliver the identity
stage 5 needs.**

The roadmap row promises V3 "Accounts … and multi-tenancy"
(`docs/superpowers/plans/2026-08-03-deskmate-roadmap.md:37`). The V3 design that is
actually being built narrowed that. On branch `feat/v3-server-host`,
`docs/superpowers/specs/2026-09-06-deskmate-v3-server-host-design.md:29` describes the
admin surface as "Admin-token gated, single operator", and `:31-34` names what V3
explicitly does not deliver: "User accounts and signup; per-user multi-tenancy or data
isolation; billing; the plugin-upload sandbox (that is the … stage 5)". §7 (`:288-297`),
"Where multi-tenancy slots in (deferred, not built)", reserves an `owner_id` seam defaulted
to a single constant and notes that this is the point where SQLite would earn its place.
The delivered code agrees: the OAuth sub-project's operator session cookie is HMAC-SHA256
keyed by `DESKMATE_ADMIN_TOKEN` — one operator, who is the owner.

So the honest sequence is: **stage 5 needs the multi-tenant step that V3 defers, not V3
itself.** Waiting for V3 would not produce an uploader principal.

The consequence for stage 5 as scoped:

- Without a principal, "per-uploader quota", "per-uploader device quarantine", "identity
  recycling" and "takedown of one uploader" are not policies that can be written. They are
  statements about a thing the system cannot name (R1).
- An upload endpoint authenticated by the single admin token is **owner curation with
  extra steps**: it adds a network write path, a multipart/archive parse surface, and a
  runtime registry-mutation path, replacing an `scp` and a `systemctl restart`. The
  security balance is negative.
- The plan's own framing anticipates this — "An upload path without V3's identity work is
  effectively owner-curated with extra steps" — and the answer is yes, it is, and V3 does
  not fix it.

**But the hardening stage 5 would need is meaningful right now, and most of it improves
curated content too.** The five items below are prerequisites under any identity model, are
independent of when uploads happen, and each closes a gap that exists today:

1. Fuzz targets and a committed corpus for the manifest, expression and SVG parsers (R2).
   Nothing in this repository has ever been fuzzed.
2. The `build_image` header-vs-payload invariant on the device (R2). A real missing bounds
   check today, unreachable only because the server happens to be the sole producer.
3. The render subprocess with OS limits and kill-on-deadline (R3). It retires the standing
   honest caveat at `plugin_provider.rs:55` and gives curated renders real preemption.
4. Per-device `desired_assets()` (R4). Quarantine needs it; it also stops broadcasting
   every plugin's assets to every device, which is wasteful today.
5. A runtime plugin enable/disable with tripwires (R5). The kill switch is missing for
   curated content too.

**That is the recommendation: spend stage 5 on the hardening, not on the endpoint.**

---

## Go / no-go

### Verdict: NO-GO for public plugin uploads as scoped.

The blocking reasons, in order of decisiveness:

1. **No uploader identity exists and V3 will not create one** (R1). Every per-uploader
   control the plan names is unimplementable without it.
2. **Hostile content would be parsed in a process holding the V3 secrets master key and
   decrypted OAuth refresh tokens** (R3). This is disqualifying on its own.
3. **The render host is shared with the owner's unrelated personal services** (R5). The
   residual risk falls on the co-hosted personal services' data that has no stake in this feature.
4. **No parser in the pipeline has ever been fuzzed** (R2), in a codebase whose one known
   content-handling vulnerability was found in curated input and needed two layers to close.
5. **Quarantine is an architecture change, not a policy** (R4), because `desired_assets()`
   is registry-wide.

### The minimal safe first iteration

If stage-5 effort is to be spent now, spend it here. **None of these requires an upload
endpoint, an identity system, or owner approval of a new risk** — each closes a gap that is
already present with curated content, and together they are the precondition set for any
future upload work.

| # | Item | Why now |
|---|---|---|
| 1 | Fuzz targets + committed corpus for `parse_manifest_bytes`, `check_nesting_depth` (differentially against `toml`), `Expr::parse`/`eval`, `rasterizer::preflight`; short-budget smoke run in CI. | The largest unknown in the pipeline, and the cheapest to reduce. |
| 2 | `build_image` header-vs-`data_size` invariant + host test + ASan; `.bss` measured; on-board OTA download re-verified. | A missing bounds check on untrusted-by-contract bytes, today. |
| 3 | `deskmate-render` subprocess: no network, `RLIMIT_AS`/`RLIMIT_CPU`, kill-on-deadline; systemd hardening properties; bounded work queue with a global concurrency cap. | Converts a measured budget into an enforced one and contains a panic. |
| 4 | Per-device `desired_assets()`, and an `uploader_id` (single-valued) threaded through the registry per V3 §7's seam. | Prerequisite for quarantine; also stops fleet-wide asset broadcast. |
| 5 | Runtime per-plugin enable/disable admin route + auto-disable tripwires + structured audit events with ids and closed labels only. | The kill switch is missing for curated content too. |
| 6 | Policy fixes: `https`-only on every redirect hop; port allowlist (80/443) in `egress_guard`; record the repeated-resolution acceptance. | One-line changes with no architectural cost. |

**Only after all six, plus a dedicated host and an approved multi-tenant identity design,
should an upload endpoint be planned** — and then with: owner-approved uploads only, no
user fonts ever, mandatory server-side image re-encode, no durable public assets, and
quarantine to the uploader's own devices.

### The plan's six proposals, judged on their merits

| Proposal | Verdict |
|---|---|
| **Owner-approved uploads only** | **Agree in principle, disagree with building it.** With no uploader identity, this collapses to owner curation plus a new network write path and a new parse surface, replacing an `scp`. Use a documented submission process instead, and reserve a single-valued `uploader_id` in the data model now. |
| **No user fonts** | **Agree, strongly, and for a stronger reason than stated.** Two consumers, and the second — `lv_tiny_ttf` at `firmware/main/ui/font_registry.c:149` — is a C parser on hardware with no isolation and no crash containment. Server-side exposure is survivable behind a subprocess; the device's is not. Curated whitelist only; revisit only if a memory-safe device font parser ever exists. |
| **Mandatory server-side image re-encoding** | **Agree, and extend it two ways.** Apply it to curated content too (one canonical producer), and add the device-side header invariant *anyway* — re-encoding is a server-side control, and this project's stated posture is that device-side code treats host bytes as untrusted. Note honestly that re-encoding moves a hostile decode into the server, so it must live inside the subprocess with one format and one decoder. |
| **Per-uploader device quarantine** | **Agree, with a cost correction the plan does not state.** `desired_assets()` is registry-wide (`runtime.rs:3313`), so every device currently receives every plugin's assets. This is an architecture change with a real regression surface, not a policy flag, and it must be costed before stage 5 is scheduled. |
| **Process-isolated rendering** | **Agree, and upgrade it from "requires" to "precondition".** The plan treats it as a decision to make; V3 settled it. A parser bug in `resvg`/`usvg`/`roxmltree`/`ttf-parser`/`toml` in the server process is a path to the owner's Google refresh tokens. Also: kill-on-deadline is the only thing that turns `MAX_RENDER_WALL_CLOCK` into enforcement, which benefits curated content today. |
| **Dedicated-isolation decision** | **Decide it now: required.** Not a nice-to-have and not a later optimisation. The shared VM carries the co-hosted personal services and the V3 secrets keyfile; a sandbox does not remove a shared kernel. Public content runs on a dedicated host or it does not run. |

### Deferred, with reasons

| Deferred | Reason |
|---|---|
| **Accounts and multi-tenancy** | V3 explicitly does not deliver them (`…v3-server-host-design.md:31-34`, §7). Needed before any per-uploader control is expressible. |
| **Hostile paying users / abuse economics** | Billing is V4 and there is no account system to bill or ban. Refunds, takedowns and identity recycling are questions about a subsystem that does not exist; answering them now would be speculative. |
| **Shared public catalog** | Requires manifest signing, provenance, a written human review standard, a revocation list checked at load, and R5's takedown path. None exist. That is a project, not a stage-5 task. |
| **Durable assets from public plugins** | `AssetRelease` is a device-wide keep-set with destructive compaction, and the 31 durable slots are shared with curated content. Public plugins render through the volatile raster path instead — transient, two-slot bounded, outside the keep-set arithmetic. |
| **User fonts** | Deferred indefinitely, not scheduled — the same disposition spec §5 gives the headless browser, and for the same reason: the risk is structural, not a matter of better bounds. |
| **Full container/VM-per-render** | Over-scoped for one render per plugin per refresh interval. Revisit only if the subprocess model proves insufficient under measurement. |

---

## Summary of recommendations

| ID | Recommendation |
|---|---|
| R1 | Owner-only authorization; **do not build an upload endpoint**; use a documented submission process; reserve a single-valued `uploader_id` now per V3 §7's seam. Declare hostile paying users out of scope. |
| R2 | **Exclude user fonts outright.** Mandate server-side image re-encoding *and* add the device `build_image` header invariant regardless. Fuzz all four parser surfaces before any hostile input reaches them. Require `https` on every redirect hop. |
| R3 | **Process-isolated rendering is a precondition**, with OS memory/CPU limits, no network in the child, and kill-on-deadline. Add admission control and a 15-minute public refresh floor. Add a port allowlist to the egress guard. |
| R4 | **Quarantine to the uploader's own devices** (requires per-device `desired_assets()`), and **no durable assets from public plugins at all** — volatile raster path only. Defer any shared catalog. |
| R5 | **Dedicated host required**, not optional; state the shared-VM residual risk in any approval. Add a runtime kill switch, auto-disable tripwires, and per-uploader audit events with ids and closed labels only. |
| Gate | **NO-GO for public uploads.** GO for the six-item hardening set, all of which improves curated content and none of which requires this decision to be revisited. |
