# GitHub stats

Shows a GitHub user's public repository, follower and following counts on one
448×368 picture card. This is the bundled plugin-contract example, version 1.0.1.
The manifest author, Deskmate, is attribution text rather than a verified identity.

## Setup and permissions

Choose **GitHub stats** in the add menu and set **Username** (default: `octocat`).
The plugin declares only `api.github.com` and requests the public user endpoint.
The optional `github_token` credential is supplied by the operator through the
account's `plugin-secrets.json`; it is substituted as a bearer credential by the
host and never passed into the sandbox. See [contract v1](../../../../docs/plugins/contract-v1.md).

## Rendering and limitations

The card refreshes every 900 seconds. Missing credentials, network failures or
missing response fields display `--` for the affected values. It has no tap action
and does not prove that a username belongs to the person configuring the card.
API availability and rate limits depend on GitHub. Operator withdrawal retains the
last stored frame, which becomes stale.

## Check and evidence

From `companion/faces`, run `bun run plugin:check github-stats` and `bun test`.
The offline checker reports the expected request-failure fallback and writes its
full-size and desk-scale PNGs under `out/plugins/github-stats/`. The discovery tests
also exercise recorded success and credential isolation. These are sandbox and PNG
checks, not physical-panel verification.

## Release 1.0.1

Fixes the duplicated `Bearer` prefix when an operator configures `github_token`.
The plugin now supplies only the secret placeholder; the host adds the prefix.
The regression test runs the shipped source through the sandbox and asserts the
outgoing header using a synthetic token and an offline response. Layout, requested
permissions, settings and refresh cadence are unchanged.

The source follows the repository licence and contains no third-party image assets.
