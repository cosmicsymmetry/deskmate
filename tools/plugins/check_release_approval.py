#!/usr/bin/env python3
"""Read Git objects only. In approval CI this file comes from the trusted base.

Never check out or execute PR code here, including its plugin tests or dependencies.
GitHub PR identity (not the rerunning actor or a commit's author text) grants approval.
"""
import json
import os
from pathlib import PurePosixPath
import re
import subprocess

INDEX = "companion/faces/plugins/releases.json"
# This policy comes from the trusted base, never a path list in the PR. Protect
# directories where an added .js sibling or Python module could shadow an import.
PROTECTED_PATHS = frozenset({
    INDEX,
    ".github/CODEOWNERS",
    "companion/crates/server/deploy/deploy.sh",
})
PROTECTED_TREES = (
    "companion/faces",
    "tools/plugins",
    ".github/workflows",
    ".github/actions",
)
RUNTIME_ROOTS = (".", "companion", "companion/faces")
RUNTIME_CONFIGS = frozenset({
    "package.json", "bun.lock", "bun.lockb", "bunfig.toml", ".bunfig.toml",
    "package-lock.json", "npm-shrinkwrap.json", "pnpm-lock.yaml", "yarn.lock",
    ".npmrc", ".yarnrc", ".yarnrc.yml",
})


def protected(path):
    file = PurePosixPath(path)
    # Checkout attributes and submodules can change the bytes being executed.
    if file.name in {".gitattributes", ".gitmodules"}:
        return True
    # Only content inside a named plugin is an outside-author contribution to
    # the faces package. Host tests/preloads also execute during deployment, so
    # they cannot be allowed to rewrite the verifier/index between checks.
    plugin_prefix = "companion/faces/plugins/"
    if path.startswith(plugin_prefix):
        parts = path[len(plugin_prefix):].split("/")
        if len(parts) > 1 and re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", parts[0]):
            return False
    if path in PROTECTED_PATHS or any(
        path == root or path.startswith(root + "/") for root in PROTECTED_TREES
    ):
        return True
    for root in RUNTIME_ROOTS:
        prefix = "" if root == "." else root + "/"
        if path == prefix + "node_modules" or path.startswith(prefix + "node_modules/"):
            return True
        if str(file.parent) == root and (
            file.name in RUNTIME_CONFIGS
            or file.name.startswith(".env")
            or (file.name.startswith("tsconfig") and file.suffix == ".json")
        ):
            return True
    return False


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def entries_at(revision):
    if not git("ls-tree", revision, "--", INDEX):
        return []
    entries = json.loads(git("show", f"{revision}:{INDEX}"))
    if not isinstance(entries, list):
        raise ValueError("The release index must be an array")
    return entries


def check_approval(base, head, author, owner):
    if not all(re.fullmatch(r"[a-f0-9]{40,64}", sha) for sha in (base, head)):
        raise ValueError("Approval needs exact base and head commit IDs")
    common = git("merge-base", base, head)
    paths = subprocess.check_output([
        "git", "diff", "--no-renames", "--name-only", "-z", common, head, "--",
    ]).decode("utf-8", errors="surrogateescape").split("\0")
    changed = sorted(path for path in paths if path and protected(path))
    if not changed:
        return "Release approval paths unchanged; authors may submit an unapproved plugin for review."
    if not owner or not author or author.casefold() != owner.casefold():
        raise ValueError(
            "Only the repository owner may change release approval files: "
            + ", ".join(json.dumps(path, ensure_ascii=True) for path in changed)
            + ". Leave these files unchanged in an outside submission; ask the owner "
            "to make approval or verification changes in an owner-authored PR."
        )
    before, after = entries_at(common), entries_at(head)
    for entry in before:
        if entry not in after:
            raise ValueError("Keep every previous reviewed release unchanged for rollback history")
    return "Release approval files changed by the repository owner; previous releases retained."


if __name__ == "__main__":
    try:
        print(check_approval(*(os.environ[key] for key in ("BASE_SHA", "HEAD_SHA", "PR_AUTHOR", "REPO_OWNER"))))
    except (KeyError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f"Release approval refused: {error}") from error
