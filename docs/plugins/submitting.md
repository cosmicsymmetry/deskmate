# Submitting a plugin

Submit a hosted plugin as a pull request to
[cosmicsymmetry/deskmate](https://github.com/cosmicsymmetry/deskmate), targeting `main`.
The first submission trial uses this manual process. Review and merge do not deploy
your plugin; hosted release is a separate operator action.

Your publishing identity is your GitHub account and the PR record. Manifest `author`
text is unverified attribution; no panel or hosted Deskmate account is needed to contribute.
Browse the [plugin directory](index.md) for the existing plugins and their READMEs.

## Prepare the plugin

Read [contract v1](contract-v1.md) first. It describes the actual sandbox, manifest,
request and rendering behavior, including known limitations.

From `companion/faces/`, install dependencies and scaffold a plugin:

```sh
bun install --frozen-lockfile
bun run plugin:new your-plugin-id
bun run plugin:check your-plugin-id
```

Use lowercase letters, digits and hyphens for the id. The generator refuses existing
folders and built-in names. It creates `plugins/<id>/plugin.json`, `index.js`,
`index.test.js` and a README with the submission template's sections. The sample
already renders a calendar date. Replace its display, description and attribution;
keep `plan({})` safe for discovery. A plugin supplies a picture, never a card kind.

The executable `index.js` runs in QuickJS: no imports, direct networking, filesystem,
environment or `Intl`. Only `plan` declares requests. Tests and preview fixtures run
on the author side and must contain public, synthetic or sanitized data only.
Add meaningful sandbox tests for date boundaries, missing data, API failures,
credential absence and taps, as applicable. Tests may live beside the plugin or in
`test/plugins/`; the generated test demonstrates the real runtime.

## Verify and show the result

```sh
bun run plugin:check your-plugin-id
bun test
bun run check
bun run lint
bun run format:check
```

The checker uses the real manifest parser, discovery's QuickJS `plan({})` probe,
server `renderRequest` entrypoint, and rasterizer. It writes to
`out/plugins/<id>/` (or `<directory>/<id>/` with `--out <directory>`):

- A **448×368 PNG** and a **0.4× desk-scale PNG**, scaled from that same rendered frame.
- `report.json` listing errors, plugin logs and runtime limit notices for each case.
- The release hash and exact `{id, version, sha256}` an owner could approve. A new
  version not yet in the release index is informational, not a failed submission.

It exits nonzero for manifest, discovery or render failures and still attempts every
case. It reports limits reached during those cases, including dropped oversized
state, shortened logs and exhausted planning/request budgets. A passing check says
nothing about paths the fixtures did not exercise. Inspect both PNG sizes yourself.

**Network is always disabled; stored secrets are never read**, even when your shell
has a server config directory. A request without a recorded reply becomes the normal
failed-request answer and is listed in the report. This exercises your fallback.
For useful success-path evidence, add `plugins/<id>/check.json`, for example:

```json
{
  "cases": [
    {
      "name": "New Year ahead of UTC",
      "now": "2027-12-31T15:00:00Z",
      "timezone": "Asia/Tokyo",
      "settings": {},
      "responses": [
        {
          "url": "https://api.example.com/value",
          "status": 200,
          "body": "{\"value\":42}"
        }
      ]
    }
  ]
}
```

Omit `responses` for a plugin without requests. Replies match the exact URL and
method (`GET` by default); repeated requests receive the same recorded reply.
Use `base64` instead of `body` for a binary response. The real host response decoder
still enforces its body limit. `settings` overlays manifest defaults; optional
`state` and `event` exercise saved state and taps. Without a fixture file, the
checker renders once at `2026-10-01T12:00:00Z` in UTC with default settings and no
recorded replies. Case names are labels, never output paths.

For every PR changing `companion/faces/plugins/**`, the **plugin-previews** CI job
checks every changed plugin and uploads `plugin-previews-<head-sha>` with the PNGs
and reports, even when a case fails. Open the PR's **ci** run and download that
artifact. Fully removed folders are listed in `removed.json`; no render is possible
for deleted code. This job needs no secrets and works for fork PRs. GitHub may still
require a maintainer to approve a first-time contributor's workflow run.

Include an accessible preview (a PR attachment or committed sanitized sample) and
link the CI run/artifact. Artifacts expire after 14 days, so use a committed sample
when evidence must survive that. State exactly what was checked: sandbox render,
PNG inspection, server frame store, browser flow or physical panel. Those are
different observations; a rendered PNG does not establish delivery to a panel.

Date-sensitive faces receive the owner's configured timezone. Since image sources
are shared per account and preferences are per device, the first active consuming
device in device-id order supplies it; before attachment, the first active saved
config supplies it, otherwise UTC. Successful faces get a date/zone-change check
once a minute; allow roughly 60 seconds plus render/delivery time after midnight.
Failures can stay stale longer. Ordinary cadence remains clamped to 60 seconds
through six hours. A raster frame never ticks by itself.

## Open the pull request

Use the [plugin submission template](https://github.com/cosmicsymmetry/deskmate/compare/main...HEAD?expand=1&template=plugin.md)
when opening your PR, or copy
[the template](../../.github/PULL_REQUEST_TEMPLATE/plugin.md) into the PR body.
Choose your feature branch as the comparison branch.

Explain what a person gets, how to configure it, what external services it reaches,
and what permissions or credentials it needs. Include version, preview, exact
validation results and known limitations. Keep tokens, personal emails, private
addresses and account data out of commits, examples, screenshots and PR text.

GitHub identifies the contributor; the manifest's `author` is attribution text, not
a verified Deskmate publishing identity. Follow the repository licence and identify
any third-party code or assets and their licences.

## Review and updates

The reviewer records the reviewed head commit and checks:

- Manifest and source agree; ids do not collide and declared hosts/secrets are the
  minimum the plugin needs. Inspect the code as well as the manifest.
- Input handling, network failures and credential absence behave as documented;
  errors retain the previous frame and reach the user through the existing contract.
- Tests run through the sandbox, resource usage fits the existing contract, and no
  networking escapes the declared request path.
- Output is legible at panel scale and honest about freshness and timezone.
- Applicable local checks and CI pass. Skipped jobs are not passing jobs.
- The submission contains no private data and includes attribution for reused work.

For the first trial, the project owner makes the acceptance decision after review.
Requested changes are commits on the same PR, followed by another review of the
affected behavior. **Bump the manifest version for every code or permission change.**
Each needs another PR and review. The [release index](releases.md) covers every file
inside the plugin folder, so documentation, test and asset edits there also need a
new version. Leave `plugins/releases.json` unchanged in an outside author's PR;
only the owner can approve its entry. The [protected approval paths](releases.md#submissions-and-approval)
also require an owner-authored PR: keep plugin behavior tests and assets inside
`plugins/<id>/`, alongside the generated test. Host runtime, package-wide tests,
verification and deploy changes need a separate owner PR. CI refuses reused versions with changed bytes,
while a new unindexed version can be green. Deployment refuses unreviewed content.

After adding a plugin or changing its label/version/description, regenerate the
documentation directory from `companion/faces` with
`bun run src/author/directory.ts`. Repository tests in CI check that it matches all manifests
and that each plugin has a README.

Merging accepts code into the repository. An operator separately chooses a reviewed
revision for release. Until that deployment, there is no promise that the plugin is
available on the hosted service. After deployment, the existing catalog can expose
the plugin in the add-card menu unless the operator has withdrawn it. The
[documentation directory](index.md) links the available source and setup guides;
it is not an installation service or a hosted availability promise.

The next outside author is invited to follow this guide unaided and open an issue
at the step where they got stuck.
