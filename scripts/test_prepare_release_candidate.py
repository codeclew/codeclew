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

    def prepare(self, version="v1.2.3", commit=None, expected_existing_tag_commit=None):
        arguments = ["python3", "-I", "-S", str(SCRIPT), "--version", version,
                     "--commit", commit or self.commit]
        if expected_existing_tag_commit is not None:
            arguments += ["--expected-existing-tag-commit", expected_existing_tag_commit]
        return subprocess.run(
            arguments,
            cwd=self.root, text=True, capture_output=True,
        )

    def existing_origin_tag(self):
        origin = tempfile.TemporaryDirectory(prefix="clew-candidate-origin-")
        self.addCleanup(origin.cleanup)
        self.git("init", "--bare", "-q", origin.name)
        self.git("remote", "add", "origin", origin.name)
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "tag", "-a", "v1.2.3", "-m", "Prior unqualified tag")
        self.git("push", "origin", "refs/tags/v1.2.3")
        return self.git("ls-remote", "--tags", "origin")

    def correction_commit(self):
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "--allow-empty", "-qm", "Correction")
        return self.git("rev-parse", "HEAD")

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

    def test_explicit_correction_rebinds_only_disposable_checkout(self):
        before = self.existing_origin_tag()
        successor = self.correction_commit()
        result = self.prepare(commit=successor, expected_existing_tag_commit=self.commit)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.git("rev-parse", "v1.2.3^{}"), successor)
        self.assertEqual(self.git("ls-remote", "--tags", "origin"), before)
        self.assertEqual(self.git("status", "--porcelain"), "")

    def test_correction_rejects_wrong_base_and_changed_origin_without_local_mutation(self):
        self.existing_origin_tag()
        tag = self.git("rev-parse", "v1.2.3")
        successor = self.correction_commit()
        wrong = self.prepare(commit=successor, expected_existing_tag_commit="a" * 40)
        self.assertNotEqual(wrong.returncode, 0)
        self.assertEqual(self.git("rev-parse", "v1.2.3"), tag)
        self.git("push", "--force", "origin", successor + ":refs/tags/v1.2.3")
        changed = self.prepare(commit=successor, expected_existing_tag_commit=self.commit)
        self.assertNotEqual(changed.returncode, 0)
        self.assertEqual(self.git("rev-parse", "v1.2.3"), tag)

    def test_correction_requires_existing_tag_clean_tree_and_full_sha(self):
        self.assertNotEqual(self.prepare(expected_existing_tag_commit=self.commit).returncode, 0)
        self.assertEqual(self.git("tag"), "")
        before = self.existing_origin_tag()
        tag = self.git("rev-parse", "v1.2.3")
        for expected in ["HEAD", "-f"]:
            self.assertNotEqual(self.prepare(expected_existing_tag_commit=expected).returncode, 0)
        (self.root / "uncommitted").write_text("dirty")
        self.assertNotEqual(self.prepare(expected_existing_tag_commit=self.commit).returncode, 0)
        self.assertEqual(self.git("rev-parse", "v1.2.3"), tag)
        self.assertEqual(self.git("ls-remote", "--tags", "origin"), before)

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
