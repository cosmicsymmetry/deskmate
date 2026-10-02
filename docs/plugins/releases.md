# Reviewed plugin releases

The operator installs only plugin bytes recorded in
[`releases.json`](../../companion/faces/plugins/releases.json). Each entry contains
`id`, manifest `version`, and lowercase `sha256`. Previous entries stay unchanged
for rollback history. The initial entries cover the reviewed `days-left-this-year`
and `github-stats` plugins. A hash identifies content; **an owner commit to this
index is the approval**.

## Submissions and approval

An outside contributor leaves the index unchanged. `plugin:check` prints the exact
entry an owner could add and includes it in `report.json`. A new version that is
not yet indexed is information in CI, so a submission can pass before acceptance.
Different bytes using an already indexed id/version fail CI and deployment.
Every code or permission change needs a new manifest version and review.

The owner reviews the source, permissions, tests and previews, then adds the entry
in an **owner-authored PR**. Pushing an index commit onto an outside author's PR
does not change that PR's author and will still fail approval CI. The owner can
merge reviewed source first and approve it in a follow-up owner PR, or carry the
reviewed source and index entry together in an owner-authored release PR. No faces
deployment from that revision is allowed until every included plugin is approved.

The `owner-release-approval` workflow uses `pull_request_target`: its workflow and
checker come from the trusted base, and it only reads fetched Git objects. It never
checks out or runs PR code, installs PR dependencies, consumes PR artifacts, or
receives write permissions. It checks the **PR author**, not a rerunning actor or
Git commit attribution. This follows GitHub's
[trusted workflow guidance](https://docs.github.com/en/actions/reference/security/securely-using-pull_request_target).
Changing the checker in a submission cannot approve that submission. The ordinary
CI job also exercises the policy and its tests. On this workflow's initial owner
PR, the ordinary job bootstraps verification; the independent trusted workflow
starts enforcing future PRs after it exists on `main`.

`CODEOWNERS` documents index ownership. Required code-owner reviews and repository
settings are unchanged; the owner can submit and approve their own release PR.
This gate is a review signal, not a claim that repository administrators cannot
bypass it. Operators still choose the reviewed revision to deploy.

## Canonical SHA-256 format

There is one implementation:
[`src/plugins/releases.ts`](../../companion/faces/src/plugins/releases.ts), used by
the author checker, CI, deployment and tests.

1. Enumerate **every regular file recursively** inside `plugins/<id>/`. The root
   manifest and `index.js` are required. Reject symlinks and special files, including
   symlinked plugin folders or release indexes.
2. Sort relative paths by their UTF-8 bytes, using `/` between directory components.
   Do not use locale sorting or normalize file contents, line endings or names.
3. Start the SHA-256 stream with the UTF-8 bytes `deskmate-plugin-release-v1` and one
   NUL byte. For each sorted file append: its path's byte length in ASCII decimal,
   `:`, the UTF-8 path bytes, its content's byte length in ASCII decimal, `:`, then
   its exact content bytes. Length framing prevents ambiguous path/content joins.
4. Emit the lowercase hexadecimal digest. Timestamps, ownership, mode bits and
   empty directories do not contribute. They do not select executable content.

**There are no file exclusions.** Manifests, code, assets, dotfiles, README files,
tests, fixture JSON and preview images all contribute. The deploy runs plugin tests
as well as the sandboxed runtime, and covering all files avoids an extension-based
loophole. Consequently, even documentation/test-only edits inside a reviewed plugin
folder require a version bump and a new index entry. Keep generated checker output
in `out/`, outside plugin folders.

## Verify and deploy

From `companion/faces/`:

```sh
bun run src/author/releases.ts check    # CI: unindexed new versions are informational
bun run src/author/releases.ts verify   # install: every folder must match an entry
```

An optional final argument selects a plugin directory for isolated checks. Neither
command contacts the network, reads credentials, or runs plugin source.

Every `deploy.sh` mode that ships faces uses `verify` before any faces-related VM
contact and again on the staged target bytes after its tests, before installation.
A missing entry or mismatch names the plugin and version and refuses the entire
faces install. **`deploy.sh --dry-run` still synchronizes to the VM.** To check only
approval, run the isolated verification command above.

To roll back, restore the exact previously reviewed folder bytes from the recorded
revision and run verification against the retained index. Release and rollback are
operator actions, not an automatic update service. For urgent runtime withdrawal,
use the [operator denylist](../self-host.md#operator-plugin-withdrawal); deploy-time
verification alone cannot stop an already installed plugin.
