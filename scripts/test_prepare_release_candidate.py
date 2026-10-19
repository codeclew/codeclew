#!/usr/bin/env python3
"""Exercise candidate tag binding against real isolated Git repositories."""

from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("prepare_release_candidate.py").resolve()


class CandidateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="clew-release-candidate-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git("init", "-q")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "--allow-empty", "-qm", "Fixture")
        self.commit = self.git("rev-parse", "HEAD")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True).strip()

    def prepare(self, version="v1.2.3", commit=None):
        return subprocess.run(
            ["python3", "-I", "-S", str(SCRIPT), "--version", version,
             "--commit", commit or self.commit],
            cwd=self.root, text=True, capture_output=True,
        )

    def test_build_tag_is_local_and_bound_to_exact_commit(self):
        result = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.git("rev-parse", "v1.2.3"), self.commit)
        self.assertEqual(self.git("remote"), "")
        self.assertEqual(self.git("status", "--porcelain"), "")

    def test_existing_version_is_never_replaced_even_at_same_commit(self):
        self.git("tag", "v1.2.3")
        result = self.prepare()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("never replace", result.stderr)
        self.assertEqual(self.git("rev-parse", "v1.2.3"), self.commit)

    def test_wrong_commit_dirty_tree_and_invalid_version_are_rejected(self):
        for kwargs in [{"commit": "a" * 40}, {"commit": "HEAD"},
                       {"version": "--help"}, {"version": "v1.2.3-rc1"}]:
            with self.subTest(kwargs=kwargs):
                self.assertNotEqual(self.prepare(**kwargs).returncode, 0)
                self.assertEqual(self.git("tag"), "")
        (self.root / "unexpected").write_text("uncommitted")
        self.assertNotEqual(self.prepare().returncode, 0)
        self.assertEqual(self.git("tag"), "")


if __name__ == "__main__":
    unittest.main()
