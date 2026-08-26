#!/usr/bin/env python3
"""Platform-independent contract tests for macOS release version binding."""

from __future__ import annotations

from pathlib import Path
import sys
import tempfile
import unittest


sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_macos_release as release  # noqa: E402


class ReleaseVersionTest(unittest.TestCase):
    def test_cli_version_must_match_the_release_tag(self) -> None:
        with tempfile.TemporaryDirectory() as value:
            temporary = Path(value)
            launcher = temporary / "clew"
            launcher.write_text(
                "#!/bin/sh\nprintf '%s\\n' 'clew 0.1.5'\n", encoding="ascii"
            )
            launcher.chmod(0o500)

            release.verify_cli_version(
                launcher, "v0.1.5", temporary / "state", temporary
            )
            with self.assertRaisesRegex(release.ReleaseError, "does not match"):
                release.verify_cli_version(
                    launcher, "v0.1.4", temporary / "other-state", temporary
                )


if __name__ == "__main__":
    unittest.main()
