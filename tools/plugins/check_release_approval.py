#!/usr/bin/env python3
"""Read Git objects only. In approval CI this file comes from the trusted base.

Never check out or execute PR code here, including its plugin tests or dependencies.
GitHub PR identity (not the rerunning actor or a commit's author text) grants approval.
"""
import json
import os
import re
import subprocess

INDEX = "companion/faces/plugins/releases.json"


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
    changed = subprocess.run(
        ["git", "diff", "--quiet", common, head, "--", INDEX], check=False
    ).returncode
    if changed not in (0, 1):
        raise ValueError("Could not compare the release index; refusing approval")
    if not changed:
        return "Release index unchanged; authors may submit an unapproved plugin for review."
    if not owner or not author or author.casefold() != owner.casefold():
        raise ValueError(
            "Only the repository owner may change the release index. "
            "An outside author cannot approve their own plugin; leave the index unchanged "
            "and the owner will add the reviewed release when accepting it."
        )
    before, after = entries_at(common), entries_at(head)
    for entry in before:
        if entry not in after:
            raise ValueError("Keep every previous reviewed release unchanged for rollback history")
    return "Release index change submitted by the repository owner; previous releases retained."


if __name__ == "__main__":
    try:
        print(check_approval(*(os.environ[key] for key in ("BASE_SHA", "HEAD_SHA", "PR_AUTHOR", "REPO_OWNER"))))
    except (KeyError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f"Release approval refused: {error}") from error
