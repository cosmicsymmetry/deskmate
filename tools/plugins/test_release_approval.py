import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from check_release_approval import INDEX, check_approval


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
        Path("check_release_approval.py").write_text("raise RuntimeError('PR code must never execute')")
        with self.assertRaisesRegex(ValueError, "Only the repository owner"):
            check_approval(self.base, self.append(), "outside", "owner")

    def test_stale_submission_is_compared_to_its_merge_base(self):
        base = self.append()
        self.git("checkout", "-q", self.base)
        Path("submission.js").write_text("new content")
        self.assertIn("unchanged", check_approval(base, self.commit(), "outside", "owner"))

    def test_invalid_revisions_fail_closed(self):
        with self.assertRaisesRegex(ValueError, "exact base and head"):
            check_approval("--help", self.base, "owner", "owner")


if __name__ == "__main__":
    unittest.main()
