# Stage 5 — Public plugin uploads and sandboxing: the risk review

> **This is a review plan, not an implementation plan.** Stage 4's exit criterion states
> it plainly: stage 5 "does not begin from this exit criterion. It begins only after its
> separate risk review and explicit owner approval." Nothing in this document authorizes
> an upload endpoint, billing, arbitrary fonts, or user content. Its deliverable is a
> decision record the owner can approve, reject, or scope down — with every question
> named before any code exists to bias the answer.

**Spec:** `docs/superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md`
(§5 server pipeline and security names stage 5's boundary; the headless-browser rejection
is permanent and not revisited here).

**Predecessor state (what stage 5 would inherit, all delivered in stages 3b-4):** a
curated filesystem registry loaded at startup; the frozen manifest v1 and additive v2
contracts; the restricted expression language with fuel/depth/output/repeat caps; the
two-layer asset containment (single normal path component + canonical containment after
join); the SSRF egress guard; `resvg` as the only rasterizer behind deny-all resolver
callbacks, a curated-fonts-only fontdb, source/expansion/node/dimension/wall-clock caps,
and a hostile corpus; per-device render negotiation with typed refusals; volatile PSRAM
frames with the two-slot bound.

## Why this needs a review at all

Everything shipped so far assumes **curated** content: the operator putting a manifest on
the server's filesystem is trusted not to be adversarial, and every bound exists as
defence in depth behind that assumption. Public uploads invert it: the manifest author
becomes the attacker in the threat model, the upload path becomes an unauthenticated (or
weakly authenticated) write primitive into a homelab that hosts unrelated services, and
every "bounded but curated" decision must be re-derived under "bounded and hostile".
The repository's own history argues for humility here: the arbitrary-file-read via
`[[assets]] file` was found *in curated content handling*, and two of its fourteen attack
vectors were caught only by the second containment layer.

## The questions the review must answer, each with a written verdict

### R1 — Identity, authorization, and blast radius

- Who may upload: accounts (V3 dependency), signed invites, or owner-only? An upload
  path without V3's identity work is effectively owner-curated with extra steps — is
  stage 5 even meaningful before V3?
- Per-uploader quotas and global ceilings (count, bytes, refresh cadence) and what
  happens at each ceiling.
- Whether a hostile *paying* user is in scope (billing is named for V4): if yes, abuse
  economics change — refunds, takedowns, and identity recycling need answers.

### R2 — The upload payload itself

- Manifest parsing already bounds sizes and shapes, but the parser has only ever seen
  curated and test-authored input. What fuzzing coverage exists/is needed for
  `parse_manifest_bytes`, the expression grammar, and TOML itself (the TOML crate's own
  DoS surface: deeply nested tables, huge keys)?
- Assets: fonts are the sharp edge. A hostile TTF reaches two consumers — the server's
  fontdb (usvg text) and the DEVICE's TTF rasterizer for runtime glyphs. The device-side
  font parser was never designed against hostile fonts; today it only ever receives
  curated bytes. **The review must decide whether user fonts are excluded outright in
  any first public iteration** (recommendation to evaluate: yes, excluded; curated font
  whitelist only).
- Images: the canonical RGB565 form is validated on-device, but user-supplied image
  assets travel as arbitrary bytes to the device store. Decide the allowed kinds and
  whether server-side re-encoding (decode -> re-encode through one trusted path) is
  mandatory so original attacker bytes never reach a device.

### R3 — Execution and resource isolation on the server

- The expression evaluator is fueled and the rasterizer wall-clocked, but both run
  in-process in the server. The review must decide whether public content requires an
  OS-level sandbox (separate process, seccomp/jail, cgroup memory/CPU caps, no network
  namespace) around parse/compile/rasterize, and what the crash-isolation story is (one
  hostile manifest must not take down the device link).
- Concurrency: N uploads x 250 ms render budgets is still a CPU DoS without admission
  control. Name the queueing/limits model.
- The egress guard (SSRF) was built for curated URLs. Public mode means every fetch URL
  is attacker-chosen: re-review DNS rebinding, redirect-chasing, IP-literal and
  IPv6/zone-id corners, response-size/time budgets under adversarial servers.

### R4 — What reaches the fleet

- A public plugin's compiled scene is bounded by the wire validator, but bounded ≠
  harmless: decide whether public content is quarantined to the uploader's OWN devices
  (strong recommendation to evaluate) versus any shared catalog, and what review/signing
  a shared catalog would require.
- Volatile raster frames are transient; durable assets are not. Decide whether public
  plugins may install durable assets at all, given `AssetRelease`'s keep-set semantics
  and the 31-slot ceiling shared with curated content.

### R5 — Operations

- Takedown/kill-switch: how fast can a hostile plugin be removed from the registry and
  from devices, and does that path work with the owner asleep (automatic tripwires:
  render-budget violations, egress anomalies, crash counts)?
- Audit trail: uploads, fetches, and renders attributable to an uploader.
- The homelab question, stated honestly: the server shares a VM with unrelated services.
  The review must state the residual risk of public content on shared infrastructure
  even WITH sandboxing, and whether stage 5 requires dedicated isolation (separate VM or
  host) as a precondition rather than a nice-to-have.

## Review mechanics

- [x] **Step 1:** For each of R1-R5, write the verdict section: threat, existing
      mitigation, gap, decision options with a recommendation, and the evidence that
      would close it. No code. — **DONE 2026-09-09**, in
      `docs/security/stage5-plugin-upload-risk-review.md`.
- [x] **Step 2:** Produce the go/no-go summary: the minimal safe first iteration (likely:
      owner-approved uploads only, no user fonts, re-encoded images, per-uploader device
      quarantine, process-isolated rendering, dedicated isolation decision), and what is
      deferred with reasons. — **DONE 2026-09-09.** The verdict is **NO-GO for public
      uploads as scoped**, and it disagrees with two of the six proposals this step
      anticipated: the upload endpoint itself is judged net-negative while no principal
      exists, and process isolation plus dedicated isolation are raised from
      "decide whether" to preconditions. It adds one constraint this plan did not
      contain — no durable assets from public plugins at all.
- [ ] **Step 3:** Present to the owner for explicit approval. Implementation planning
      starts only from the approved scope, as its own plan. — **The review is written and
      awaiting the owner; nothing here is approved.**

**This plan's premise was corrected by the review it commissioned.** R1 asks "is stage 5
even meaningful before V3?" and assumes V3 supplies identity. It does not: the V3 design
being executed (`2026-09-06-deskmate-v3-server-host-design.md:31-32`, and its §7
"Where multi-tenancy slots in — deferred, not built") explicitly excludes user accounts
and per-user isolation, and the delivered operator cookie is HMAC'd with the single
`DESKMATE_ADMIN_TOKEN`. Stage 5 therefore depends on the multi-tenancy step V3 defers,
not on V3.

## What this review does not do

- No upload endpoint, no billing, no sandbox implementation, no new dependencies.
- No revisiting of settled negatives: the headless browser stays rejected; `resvg` stays
  the only rasterizer; provisioning stays a cable operation.
