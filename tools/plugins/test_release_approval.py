import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from check_release_approval import INDEX, check_approval

REPO = Path(__file__).resolve().parents[2]


class ApprovalTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.cwd = os.getcwd()
        os.chdir(self.directory.name)
        self.addCleanup(self.directory.cleanup)
        self.addCleanup(os.chdir, self.cwd)
        self.git("init", "-q")
        self.release = {"id": "sample", "version": "1.0.0", "sha256": "a" * 64}
        Path(INDEX).parent.mkdir(parents=True)
        Path(INDEX).write_text(json.dumps([self.release]))
        self.base = self.commit()

    def git(self, *args):
        return subprocess.check_output(["git", *args], text=True, stderr=subprocess.PIPE).strip()

    def commit(self):
        self.git("add", ".")
        self.git("-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "-qm", "fixture")
        return self.git("rev-parse", "HEAD")

    def append(self):
        Path(INDEX).write_text(json.dumps([self.release, {**self.release, "version": "2.0.0"}]))
        return self.commit()

    def test_outside_author_cannot_approve_even_when_owner_reruns_ci(self):
        head = self.append()
        with patch.dict(os.environ, {"GITHUB_ACTOR": "owner"}):
            with self.assertRaisesRegex(ValueError, "Only the repository owner"):
                check_approval(self.base, head, "outside-author", "owner")

    def test_owner_can_append_reviewed_release(self):
        self.assertIn("previous releases retained", check_approval(self.base, self.append(), "Owner", "owner"))

    def test_outside_code_submission_can_be_green_without_approval(self):
        Path("new-plugin.js").write_text("unreviewed plugin content")
        self.assertIn("unchanged", check_approval(self.base, self.commit(), "outside-author", "owner"))

    def test_owner_cannot_rewrite_or_delete_rollback_history(self):
        for entries in ([], [{**self.release, "sha256": "b" * 64}]):
            with self.subTest(entries=entries):
                Path(INDEX).write_text(json.dumps(entries))
                with self.assertRaisesRegex(ValueError, "previous reviewed release"):
                    check_approval(self.base, self.commit(), "owner", "owner")

    def test_changing_checker_in_pr_does_not_execute_it(self):
        script = Path("tools/plugins/check_release_approval.py")
        script.parent.mkdir(parents=True)
        script.write_text("PROTECTED_PATHS = set()\nraise RuntimeError('PR code must never execute')")
        with self.assertRaisesRegex(ValueError, "Only the repository owner"):
            check_approval(self.base, self.commit(), "outside", "owner")

    def refuse_edit(self, path, contents):
        file = Path(path)
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(contents)
        head = self.commit()
        with self.assertRaises(ValueError) as error:
            check_approval(self.base, head, "outside", "owner")
        self.assertIn("Only the repository owner", str(error.exception))
        self.assertIn(path, str(error.exception))
        self.assertIn("previous releases retained", check_approval(self.base, head, "owner", "owner"))
        self.git("reset", "--hard", self.base)

    def test_machinery_cannot_enable_pending_installation_with_index_unchanged(self):
        self.refuse_edit("companion/faces/src/author/releases.ts", "verifyReleases(root, true);\n")

    def test_index_path_cannot_be_redirected_with_index_unchanged(self):
        self.refuse_edit("companion/faces/src/plugins/releases.ts", 'export const RELEASE_INDEX = "self-approved.json";\n')

    def test_approval_workflow_cannot_be_disabled_with_index_unchanged(self):
        self.refuse_edit(".github/workflows/plugin-release-approval.yml", "on: workflow_dispatch\n")

    def test_deployment_and_verifier_dependencies_are_owner_only(self):
        for path in [
            ".github/CODEOWNERS", ".github/workflows/ci.yml", ".github/actions/run/action.yml",
            "companion/crates/server/deploy/deploy.sh",
            "companion/faces/src/plugins/manifest.ts", "companion/faces/src/plugins/releases.js",
            "companion/faces/test/setup.ts", "companion/faces/test/plugins/example.test.ts",
            "companion/faces/new-directory/added.test.ts",
            "tools/plugins/json.py", "tools/plugins/test_release_approval.py",
            "package.json", "companion/package.json", "companion/faces/package.json",
            "bunfig.toml", "companion/bunfig.toml", "companion/faces/bunfig.toml",
            "companion/faces/tsconfig.json", "companion/faces/bun.lock",
            "companion/faces/.env.production", ".env", "companion/.env.local",
            ".gitattributes", "companion/faces/.gitattributes", ".gitmodules",
            "node_modules/preload.js", "companion/node_modules/preload.js",
            "companion/faces/node_modules/preload.js",
        ]:
            with self.subTest(path=path):
                self.refuse_edit(path, "changed verification dependency\n")

    def test_deleting_or_renaming_protected_machinery_is_also_refused(self):
        file = Path("companion/faces/src/author/releases.ts")
        file.parent.mkdir(parents=True)
        file.write_text("trusted verifier\n")
        self.base = self.commit()
        file.rename("unprotected.txt")
        with self.assertRaisesRegex(ValueError, "companion/faces/src/author/releases.ts"):
            check_approval(self.base, self.commit(), "outside", "owner")

    def test_normal_plugin_docs_and_behavior_tests_remain_open_to_contributors(self):
        for path in ["companion/faces/plugins/new-card/index.js", "companion/faces/plugins/new-card/index.test.js", "docs/plugins/submitting.md"]:
            file = Path(path)
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text("contribution\n")
        self.assertIn("unchanged", check_approval(self.base, self.commit(), "outside", "owner"))

    def test_stale_submission_is_compared_to_its_merge_base(self):
        base = self.append()
        self.git("checkout", "-q", self.base)
        Path("submission.js").write_text("new content")
        self.assertIn("unchanged", check_approval(base, self.commit(), "outside", "owner"))

    def test_invalid_revisions_fail_closed(self):
        with self.assertRaisesRegex(ValueError, "exact base and head"):
            check_approval("--help", self.base, "owner", "owner")


class TrustedWorkflowTests(unittest.TestCase):
    def test_approval_workflow_executes_only_base_code_with_read_permissions(self):
        # Pin the executable security boundary, ignoring prose comments. Any new
        # action, command, permission, checkout or trigger needs explicit review;
        # a denylist of known bad shell strings would miss new execution paths.
        expected = '''name: plugin release approval
on:
  pull_request_target:
    branches: [main]
permissions:
  contents: read
jobs:
  owner-release-approval:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ github.event.pull_request.base.sha }}
          fetch-depth: 0
          persist-credentials: false
      - name: require owner identity for release approval changes
        env:
          BASE_SHA: ${{ github.event.pull_request.base.sha }}
          HEAD_SHA: ${{ github.event.pull_request.head.sha }}
          PR_AUTHOR: ${{ github.event.pull_request.user.login }}
          REPO_OWNER: ${{ github.repository_owner }}
        run: |
          git fetch --no-tags origin "$HEAD_SHA"
          python3 tools/plugins/check_release_approval.py
'''
        actual = (REPO / ".github/workflows/plugin-release-approval.yml").read_text()
        executable = lambda text: [line.rstrip() for line in text.splitlines() if line.strip() and not line.lstrip().startswith("#")]
        self.assertEqual(executable(actual), executable(expected))


if __name__ == "__main__":
    unittest.main()
