#!/usr/bin/env python3
"""Owned protocol regressions; these do not attest arbitrary agent OS behavior."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parent.parent
HELPER = ROOT / "skills/codeclew/scripts/declaration_source.py"
Reader = runpy.run_path(str(HELPER))["Reader"]
SNAPSHOT = "sha256:owned-snapshot/1"
SYMBOL = "owned:Invoice.submit"


def source(text="fun submit() = 42"):
    return {"kind": "SOURCE", "id": "body", "record": {
        "id": "body", "service": "owned", "file": "Owned.kt", "revision": "revision-a",
        "startLine": 4, "endLine": 4, "authority": "EXACT_RETAINED_SOURCE",
        "text": text, "textDigest": "sha256:" + hashlib.sha256(text.encode()).hexdigest(),
    }}


def declaration(ids=None):
    return {"kind": "DEPENDENCY", "id": "symbol-record", "record": {
        "id": "symbol-record", "kind": "SYMBOL", "service": "owned", "symbol": SYMBOL,
        "sourceIds": ["body"] if ids is None else ids, "fullRecordDigest": "sha256:descriptor",
        "normalized": {"documentation": {"bodyStatus": "RETAINED_BODY"}},
    }}


def page(*items, cursor=None, omitted=None):
    return {"schema": "codeclew-docs-page/1.0", "subject": "service:owned",
            "snapshot": SNAPSHOT, "items": list(items), "nextCursor": cursor,
            "omitted": omitted or []}


class DeclarationSourceTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.output = Path(self.temporary.name)
        self.args = argparse.Namespace(clew="/supported/bin/clew", root="/owned/docs",
            service="owned", snapshot=SNAPSHOT, symbol=SYMBOL, entrypoint=None,
            output_dir=self.output, page_size=6)
        self.commands = []

    def execute(self, packets):
        responses = iter(packets)
        def native(command, **kwargs):
            self.commands.append(command)
            self.assertEqual(command[:3], [self.args.clew, "docs", "context"])
            self.assertEqual(kwargs["stdin"], subprocess.DEVNULL)
            result = next(responses)
            if isinstance(result, subprocess.CompletedProcess):
                return result
            return subprocess.CompletedProcess(command, 0, json.dumps(result).encode(), b"diagnostic\xff\n")
        # Any attempted direct source read in the helper fails this regression.
        # Local receipt writes remain permitted. This is scoped instrumentation,
        # not an agent sandbox or enforcement beyond this tested invocation.
        with patch("subprocess.run", side_effect=native), \
             patch.object(Path, "read_bytes", side_effect=AssertionError("raw read forbidden")), \
             patch.object(Path, "read_text", side_effect=AssertionError("raw read forbidden")), \
             patch("builtins.open", side_effect=AssertionError("raw read forbidden")):
            value, code = Reader(self.args).result()
        self.assertFalse(value["sourceFallbackAllowed"])
        self.assertFalse(value["completeDeclarationClaimed"])
        self.assertFalse(value["callFlowClosureClaimed"])
        self.assertEqual(json.loads((self.output / "result.json").read_text()), value)
        return value, code

    def test_mixed_entrypoint_ids_are_not_body_order(self):
        self.args.entrypoint, self.args.symbol = "entry", None
        entry = {"kind": "ENTRYPOINT", "id": "entry", "record": {
            "service": "owned", "symbol": SYMBOL, "sourceIds": ["receiver", "import", "body"]}}
        value, code = self.execute([page(entry), page(declaration()), page(source())])
        self.assertEqual(code, 0)
        self.assertEqual(self.commands[-1][-2:], ["--source", "body"])
        self.assertEqual(value["sourceItems"], [source()])
        self.assertEqual(value["declaration"]["bodyStatus"], "RETAINED_BODY")

    def test_required_continuation_stops_at_binding_and_reuses_source(self):
        value, code = self.execute([page(cursor="next"),
            page(declaration(), source(), cursor="unrelated-flow")])
        self.assertEqual(code, 0)
        self.assertEqual(len(self.commands), 2)
        self.assertEqual(self.commands[1][-2:], ["--cursor", "next"])
        self.assertEqual(value["unreadNavigation"][0]["nextCursor"], "unrelated-flow")
        self.assertEqual(value["sourceRaw"]["pointer"], "/items/1")

    def test_missing_source_binding_never_falls_back(self):
        value, code = self.execute([page(declaration([]))])
        self.assertEqual((code, value["status"]), (3, "MISSING_DECLARATION_SOURCE"))
        self.assertIn("explicit user permission", value["nextAction"])
        self.assertEqual(len(self.commands), 1)

    def test_ambiguous_source_binding_never_guesses(self):
        value, code = self.execute([page(declaration(["one", "two"]))])
        self.assertEqual((code, value["status"]), (3, "AMBIGUOUS_DECLARATION_SOURCE"))
        self.assertEqual(len(self.commands), 1)

    def test_native_ambiguity_preserves_all_streams_and_exit(self):
        stdout = b'{"error":{"code":"AMBIGUOUS_SYMBOL","candidates":["one","two"]}}\n'
        stderr = b"diagnostic\xff\n" * 18000
        value, code = self.execute([subprocess.CompletedProcess([], 2, stdout, stderr)])
        self.assertEqual((code, value["status"]), (2, "NATIVE_ERROR"))
        receipt = value["rawReceipt"]
        self.assertEqual((self.output / receipt["stdout"]).read_bytes(), stdout)
        self.assertEqual((self.output / receipt["stderr"]).read_bytes(), stderr)
        self.assertEqual(receipt["stderrSha256"], hashlib.sha256(stderr).hexdigest())

    def test_embedded_error_with_zero_exit_is_still_failure(self):
        value, code = self.execute([{"error": {"code": "INVALID_INPUT"}}])
        self.assertEqual((code, value["status"]), (2, "NATIVE_ERROR"))

    def test_missing_source_and_omissions_remain_explicit(self):
        value, code = self.execute([page(declaration()), page(omitted=[{"reason": "not retained"}])])
        self.assertEqual(code, 3)
        self.assertEqual(value["status"], "SELECTED_BINDING_NOT_RETURNED")
        self.assertEqual(value["omitted"][0]["omitted"][0]["reason"], "not retained")

    def test_large_source_and_unspecified_body_status_are_preserved(self):
        item = declaration()
        item["record"]["normalized"]["documentation"] = None
        body = source("x" * 100000)
        value, code = self.execute([page(item, body)])
        self.assertEqual(code, 0)
        self.assertIsNone(value["declaration"]["bodyStatus"])
        self.assertEqual(value["sourceItems"], [body])
        self.assertEqual(value["sourceTextBytes"], 100000)

    def test_malformed_page_and_nonprogressing_cursor_are_rejected(self):
        value, code = self.execute([page(cursor="same"), page(cursor="same")])
        self.assertEqual((code, value["status"]), (2, "INVALID_NATIVE_CONTINUATION"))

    def test_duplicate_declarations_are_not_silently_selected(self):
        value, code = self.execute([page(declaration(), declaration())])
        self.assertEqual((code, value["status"]), (3, "AMBIGUOUS_NATIVE_BINDING"))

    def test_mismatched_scope_is_rejected(self):
        packet = page(declaration(), source()); packet["snapshot"] = "other"
        value, code = self.execute([packet])
        self.assertEqual((code, value["status"]), (2, "NATIVE_SCOPE_MISMATCH"))

    def test_source_digest_and_revision_are_checked(self):
        body = source(); body["record"]["textDigest"] = "sha256:wrong"
        value, code = self.execute([page(declaration(), body)])
        self.assertEqual((code, value["status"]), (2, "SOURCE_TEXT_DIGEST_MISMATCH"))
        body = source()
        value, code = self.execute([page({"kind": "COVERAGE", "revision": "other"}, declaration(), body)])
        self.assertEqual((code, value["status"]), (2, "SOURCE_REVISION_MISMATCH"))

    def test_invalid_json_is_kept_without_raw_fallback(self):
        raw = b"not-json\xff"
        value, code = self.execute([subprocess.CompletedProcess([], 0, raw, b"error")])
        self.assertEqual((code, value["status"]), (2, "INVALID_NATIVE_OUTPUT"))
        self.assertEqual((self.output / value["rawReceipt"]["stdout"]).read_bytes(), raw)

    def test_invalid_span_does_not_claim_provenance(self):
        body = source(); body["record"]["endLine"] = 1
        value, code = self.execute([page(declaration(), body)])
        self.assertEqual((code, value["status"]), (2, "INVALID_SOURCE_PROVENANCE"))

    def test_default_authoring_instruction_requires_permission_for_source_bypass(self):
        skill = (ROOT / "skills/codeclew/SKILL.md").read_text()
        section = (ROOT / "skills/codeclew/references/compose-original.md").read_text()
        words = " ".join(section.split())
        # User-required instruction contract. Behavior is separately exercised
        # above; this check does not establish that a model obeys the instruction.
        for obligation in ["source facts MUST come through supported", "shell/file tools",
                           "GitHub/web/raw URLs", "private-store/cache access",
                           "user explicitly authorizes", "not permission to bypass clew",
                           "not OS/filesystem sandbox enforcement", "Ordinary product implementation"]:
            self.assertIn(obligation.casefold(), words.casefold())
        self.assertIn('scripts/evidence_packet.py" discover', section)
        self.assertIn('scripts/evidence_packet.py" read', section)
        self.assertIn('do not clone it again', words)
        self.assertIn('Prepared SOURCE: start here', skill)
        self.assertTrue(HELPER.is_file())


if __name__ == "__main__":
    unittest.main()
