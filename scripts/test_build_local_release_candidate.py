#!/usr/bin/env python3

from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_local_release_candidate as candidate
import build_macos_release as release


class LocalReleaseCandidateTest(unittest.TestCase):
    def test_workspace_version_is_exact_semver(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            root = Path(value)
            (root / "Cargo.toml").write_text(
                '[workspace]\n[workspace.package]\nversion = "1.2.3"\n',
                encoding="utf-8",
            )
            self.assertEqual(candidate.workspace_version(root), "v1.2.3")
            (root / "Cargo.toml").write_text(
                '[workspace]\n[workspace.package]\nversion = "1.2.3-dev"\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(release.ReleaseError, "invalid"):
                candidate.workspace_version(root)

    def test_dirty_source_is_rejected_before_candidate_build(self) -> None:
        with mock.patch.object(release, "git_text", return_value=" M source"):
            with self.assertRaisesRegex(release.ReleaseError, "clean source"):
                candidate.clean_source_authority(Path("."))

    def test_metadata_binds_runtime_source_and_profile(self) -> None:
        value = candidate.candidate_metadata(
            "v1.2.3",
            "a" * 40,
            "b" * 40,
            "core",
            "sha256:" + "c" * 64,
            "sha256:" + "d" * 64,
            "sha256:" + "e" * 64,
        )
        digest = value["candidateDigest"]
        unsigned = dict(value)
        unsigned["candidateDigest"] = ""
        self.assertEqual(digest, release.sha256(release.canonical(unsigned)))
        self.assertEqual(value["status"], "LOCAL_ONLY")
        self.assertNotIn("path", json.dumps(value).lower())

    def test_existing_output_is_never_overwritten(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            output = Path(value) / "candidate"
            output.mkdir()
            with self.assertRaisesRegex(release.ReleaseError, "must not already exist"):
                candidate.validate_output(output)


if __name__ == "__main__":
    unittest.main()
