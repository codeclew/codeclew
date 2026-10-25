#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import random
import re
import subprocess
import tempfile
import unittest
from urllib.parse import quote, urlsplit, urlunsplit


MODULE_PATH = Path(__file__).resolve().with_name("check_repository_privacy.py")
SPEC = importlib.util.spec_from_file_location("codeclew_repository_privacy", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
privacy = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = privacy
SPEC.loader.exec_module(privacy)


class RepositoryPrivacyTest(unittest.TestCase):
    def test_password_urls_are_rejected_even_for_explicit_synthetic_fixtures(self) -> None:
        fixture_username = "fixture-user"
        fixture_password = "fixture-password"
        for endpoint in ["https://example.invalid/docs", "https://service.example.org/docs"]:
            parts = urlsplit(endpoint)
            authority = quote(fixture_username, safe="") + ":" + quote(fixture_password, safe="") + "@" + parts.netloc
            location = urlunsplit(parts._replace(netloc=authority))
            self.assertEqual(urlsplit(location).password, fixture_password)
            self.assertIn("credential-url", privacy.blob_rules(location.encode(), "negative-fixture.rs"))
        self.assertNotIn("credential-url", privacy.blob_rules(b"https://example.invalid/docs"))

    def test_personal_and_bot_identities_pass_without_weakening_content_checks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def git(*args: str) -> None:
                subprocess.run(["git", "-C", str(root), *args], check=True,
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            def check(*args: str) -> subprocess.CompletedProcess:
                return subprocess.run([sys.executable, "-I", "-S", str(MODULE_PATH), *args],
                                      cwd=root, capture_output=True, text=True, check=False)
            git("init", "-q")
            git("config", "user.name", "Example Contributor")
            git("config", "user.email", "contributor@example.invalid")
            public = root / "public.txt"
            public.write_text("Public source")
            git("add", ".")
            git("commit", "-qm", "Personal contribution")
            self.assertEqual(check("--history").returncode, 0)
            self.assertEqual(check("--worktree", "--pre-commit").returncode, 0)
            git("config", "user.name", "Example Bot")
            git("config", "user.email", "bot@example.invalid")
            public.write_text("Public bot contribution")
            git("add", ".")
            git("commit", "-qm", "Bot contribution")
            self.assertEqual(check("--history").returncode, 0)
            self.assertEqual(check("--pre-commit").returncode, 0)
            private = b"/Users/" + b"private-example/source"
            public.write_bytes(private)
            git("add", ".")
            rejected = check("--pre-commit")
            self.assertEqual(rejected.returncode, 1)
            self.assertIn("public.txt: personal-home-path", rejected.stderr)
            git("commit", "-qm", "Private content control")
            public.write_text("Public source restored")
            git("add", ".")
            git("commit", "-qm", "Restore public content")
            self.assertEqual(check("--worktree").returncode, 0)
            rejected = check("--history")
            self.assertEqual(rejected.returncode, 1)
            self.assertIn("personal-home-path", rejected.stderr)
            self.assertNotIn("private-example", rejected.stderr)

    def test_large_inline_assets_do_not_require_an_email_to_make_progress(self) -> None:
        asset = b"A" * 65536
        first = next(privacy.EMAIL_RE.finditer(asset))
        self.assertEqual(first.span(), (0, len(asset)))
        self.assertIsNone(first.group(1))
        self.assertNotIn("non-placeholder-email", privacy.blob_rules(asset))
        for domain, forbidden in [(b"example.invalid", False), (b"example.com", True)]:
            data = asset + b"@" + domain
            self.assertEqual("non-placeholder-email" in privacy.blob_rules(data), forbidden)

    def test_email_findings_preserve_previous_detector_semantics(self) -> None:
        previous = re.compile(br"[A-Z0-9._%+-]+@([A-Z0-9.-]+\.[A-Z]{2,})", re.IGNORECASE)
        at = b"@"
        cases = [b"a" + at + b"example.invalid+next" + at + b"example.com",
                 b"a" + at + b"b" + at + b"example.com",
                 b"person" + at + b"example.INVALID", b"not an address"]
        generator = random.Random(2718)
        alphabet = b"abCD09._%+-@ /:"
        for _ in range(2000):
            cases.append(bytes(generator.choice(alphabet) for _ in range(80)))
        for data in cases:
            expected = any(not match.group(1).lower().endswith(b".invalid")
                           for match in previous.finditer(data))
            observed = any(match.group(1) is not None
                           and not match.group(1).lower().endswith(b".invalid")
                           for match in privacy.EMAIL_RE.finditer(data))
            self.assertEqual(observed, expected, repr(data))

    def test_worktree_checks_pending_content_without_changing_index(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            tracked = root / "tracked.txt"
            tracked.write_text("public placeholder")
            (root / ".gitignore").write_text("ignored.txt\n")
            subprocess.run(["git", "-C", str(root), "add", "."], check=True)
            index = (root / ".git/index").read_bytes()
            private = b"/Users/" + b"private-example/source"
            tracked.write_bytes(private)
            (root / "pending.txt").write_bytes(private)
            (root / "ignored.txt").write_bytes(private)
            def check(*args: str) -> subprocess.CompletedProcess:
                return subprocess.run(
                    [sys.executable, "-I", "-S", str(MODULE_PATH), *args],
                    cwd=root, capture_output=True, text=True, check=False,
                )
            self.assertEqual(check().returncode, 0, "default still checks staged blobs")
            pending = check("--worktree")
            self.assertEqual(pending.returncode, 1)
            self.assertIn("tracked.txt: personal-home-path", pending.stderr)
            self.assertIn("pending.txt: personal-home-path", pending.stderr)
            self.assertNotIn("ignored.txt", pending.stderr)
            self.assertNotIn("private-example", pending.stderr)
            tracked.unlink()
            (root / "pending.txt").unlink()
            (root / "link.txt").symlink_to("ignored.txt")
            self.assertEqual(check("--worktree").returncode, 0, "do not follow symlinks or scan deleted content")
            self.assertEqual((root / ".git/index").read_bytes(), index)

    def test_pilot_results_are_forbidden_even_when_force_added(self) -> None:
        self.assertEqual(
            privacy.path_rules("docs/pilot/results/case-001.json"),
            ["private-generated-path"],
        )
        self.assertEqual(privacy.path_rules("docs/pilot/case-template.json"), [])

    def test_pilot_case_schema_is_forbidden_outside_exact_template(self) -> None:
        template = (MODULE_PATH.parent.parent / privacy.PILOT_CASE_TEMPLATE).read_bytes()
        self.assertNotIn(
            "filled-pilot-case",
            privacy.blob_rules(template, privacy.PILOT_CASE_TEMPLATE),
        )
        self.assertIn(
            "filled-pilot-case",
            privacy.blob_rules(template, "private-case.json"),
        )
        pretty = template.replace(b'"schema":"', b'"schema": "')
        self.assertIn(
            "filled-pilot-case",
            privacy.blob_rules(pretty, "pretty-private-case.json"),
        )
        changed = template.replace(
            b'"outcome":"RECORDER_OUTPUT_REQUIRED"', b'"outcome":"FAILED"'
        )
        self.assertIn(
            "filled-pilot-case",
            privacy.blob_rules(changed, privacy.PILOT_CASE_TEMPLATE),
        )
        for schema in [
            "codeclew-pilot-attestation-key/1.0",
            "codeclew-pilot-case-set/1.0",
            "codeclew-pilot-release-decision/1.0",
            "codeclew-pilot-source-snapshot/1.0",
        ]:
            evidence = (f'{{"schema":"{schema}"}}\n').encode()
            self.assertIn(
                "filled-pilot-case",
                privacy.blob_rules(evidence, "arbitrary.json"),
            )


if __name__ == "__main__":
    unittest.main()
