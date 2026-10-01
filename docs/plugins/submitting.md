# Submitting a plugin

Submit a hosted plugin as a pull request to
[cosmicsymmetry/deskmate](https://github.com/cosmicsymmetry/deskmate), targeting `main`.
The first submission trial uses this manual process. Review and merge do not deploy
your plugin; hosted release is a separate operator action.

## Prepare the plugin

Read [contract v1](contract-v1.md) first. It describes the actual sandbox, manifest,
request and rendering behavior, including known limitations.

Add a folder at `companion/faces/plugins/<id>/` with:

- `plugin.json`: unique stable id matching the folder, version, author attribution,
  description, settings, requested refresh cadence, hosts and secrets.
- `index.js`: sandbox-compatible `plan` and `render` exports. No imports, filesystem
  access or direct network calls. `plan({})` must work during discovery.
- `README.md`: what it displays, settings, data sources, date/time conventions,
  credentials if required, requested cadence and known limitations.

Use `companion/faces/plugins/github-stats/` as a working example. Check for id
collisions with other plugins and built-in faces. A plugin supplies a picture; it
does not add a card kind or change the device schema or protocol.

Include meaningful tests under `companion/faces/test/plugins/`. Exercise the real
sandbox with fixed inputs and recorded responses. Cover the behavior a reviewer
could otherwise misunderstand: date boundaries, missing data, request failure,
credential absence, and declared interactions, as applicable. Never use live
credentials or personal API responses in a test or preview.

## Verify and show the result

From `companion/faces/`, install the pinned dependencies and run:

```sh
bun install --frozen-lockfile
bun test
bun run check
bun run lint
bun run format:check
bun run src/main.ts describe
```

Render through the real entrypoint, substituting your id and settings:

```sh
echo '{"kind":"your-plugin-id","settings":{}}' \
  | bun run src/main.ts render > /tmp/deskmate-plugin-preview.json
```

The response is a JSON envelope, not raw PNG. Decode its `png` field as base64
and inspect the resulting image at 448×368 and at roughly 40% scale. See the
[contract's local testing instructions](contract-v1.md#testing-a-plugin-locally).
Provide a preview image and the command/fixtures that reproduce it in the PR;
a small committed sample under the plugin folder is acceptable. A local image
path alone is not accessible evidence for a GitHub reviewer.

State exactly what was checked: sandbox render, PNG inspection, server frame store,
browser flow or physical panel. Those are different observations. A render succeeding
does not establish that a panel received it.

The current plugin runtime uses the host's timezone unless a caller explicitly
supplies one; the server does not currently forward the owner's configured timezone.
Date-sensitive plugins must disclose this. Requested refresh cadence is clamped to
the scheduler's current range, and a raster frame does not tick between refreshes.
Do not promise exact midnight updates.

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
affected behavior. New executable versions or changed permissions require another
PR and review; increment the manifest version when behavior changes. The runtime
does not itself enforce reviewed hashes or version increments.

Merging accepts code into the repository. An operator separately chooses a reviewed
revision for release. Until that deployment, there is no promise that the plugin is
available on the hosted service. After deployment, the existing catalog can expose
the plugin in the add-card menu; a separate public directory is not yet defined.
