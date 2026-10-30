#!/usr/bin/env python3
"""Owned protocol checks for supported retained declaration discovery."""
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
Catalogue = runpy.run_path(str(ROOT / "skills/codeclew/scripts/declaration_catalogue.py"))["Catalogue"]
SNAPSHOT = "sha256:owned/1"


def work_page(*items, omitted=None, cursor=None):
    return {"schema": "codeclew-documentation-work-page/1.0", "work": "work-a",
            "subject": "service:owned", "snapshot": SNAPSHOT, "items": list(items),
            "omitted": omitted or [], "nextCursor": cursor, "total": len(items),
            "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED"}


def coverage_page():
    return {"schema": "codeclew-docs-page/1.0", "subject": "service:owned", "snapshot": SNAPSHOT,
            "authority": "PINNED_SNAPSHOT_NOT_REVERIFIED", "omitted": [], "nextCursor": "unread-entrypoints",
            "items": [{"kind": "COVERAGE", "id": "owned", "revision": "revision-a",
                       "coverage": "SYNTAX", "boundaries": ["CALL_TARGETS_UNRESOLVED"]}]}


def declaration(name="tick", sources=None, body_status="RETAINED_BODY"):
    return {"kind": "DEPENDENCY", "id": "declaration:" + name, "reference": "d1", "record": {
        "id": "declaration:" + name, "service": "owned", "kind": "SYMBOL", "symbol": "owned.Sweep." + name,
        "sourceIds": ["retained-body"] if sources is None else sources,
        "digest": "sha256:observation", "normalized": {"kind": "METHOD", "name": name,
        "ownerIdentity": "owned.Sweep", "documentation": {"bodyStatus": body_status, "parameterTypes": ["int"]}}}}


class DeclarationCatalogueTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(); self.addCleanup(self.temporary.cleanup)
        self.output = Path(self.temporary.name)
        self.args = argparse.Namespace(clew="/supported/bin/clew", root="/owned/docs", service="owned",
            snapshot=SNAPSHOT, prepare_work=False, work="work-a", contains="", cursor=None,
            page_items=None, output_dir=self.output)
        self.calls, self.inputs = [], []

    def execute(self, packets):
        sequence = iter(packets)
        read_input = Path.read_text
        def native(command, **kwargs):
            self.calls.append(command)
            self.assertEqual(command[0], self.args.clew)
            self.assertIn(command[1:4], [["docs", "context", "--root"],
                ["docs", "work", "read"], ["docs", "work", "prepare"]])
            if "--input" in command:
                # Test-owned input JSON, not target source or private evidence.
                path = Path(command[command.index("--input") + 1])
                self.assertEqual(path.parent, self.output)
                self.inputs.append(json.loads(read_input(path)))
            packet = next(sequence)
            return packet if isinstance(packet, subprocess.CompletedProcess) else subprocess.CompletedProcess(
                command, 0, json.dumps(packet).encode(), b"diagnostic\xff\n")
        with patch("subprocess.run", side_effect=native), \
             patch.object(Path, "read_text", side_effect=AssertionError("target read forbidden")), \
             patch.object(Path, "read_bytes", side_effect=AssertionError("target read forbidden")), \
             patch("builtins.open", side_effect=AssertionError("target read forbidden")):
            value, code = Catalogue(self.args).result()
        self.assertFalse(value["sourceFallbackAllowed"])
        self.assertFalse(value["sourceBodyReadAttested"])
        self.assertFalse(value["activeEntrypointsClaimed"])
        self.assertFalse(value["selectionDeliveryCompleteClaimed"])
        self.assertEqual(json.loads((self.output / "result.json").read_text()), value)
        return value, code

    def test_discovers_exact_declaration_not_active_entrypoint(self):
        value, code = self.execute([coverage_page(), work_page(declaration())])
        self.assertEqual(code, 0)
        row = value["items"][0]
        self.assertEqual((row["symbol"], row["revision"], row["sourceIds"]),
                         ("owned.Sweep.tick", "revision-a", ["retained-body"]))
        self.assertEqual(row["observationKind"], "SYMBOL")
        self.assertEqual(row["descriptorKind"], "METHOD")
        self.assertEqual(row["bodyStatus"], "RETAINED_BODY")
        self.assertEqual(row["parameterTypes"], ["int"])
        self.assertEqual(row["authority"], "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED")
        self.assertEqual(value["coverage"]["record"]["coverage"], "SYNTAX")
        self.assertTrue(value["pageHasNoOmissions"])

    def test_preparation_explicit_same_snapshot_supported_commands_only(self):
        self.args.prepare_work, self.args.work, self.args.page_items = True, None, 2
        value, code = self.execute([work_page(), coverage_page(), work_page(declaration())])
        self.assertEqual(code, 0)
        self.assertEqual(value["work"], "work-a")
        self.assertEqual(self.calls[0][1:4], ["docs", "work", "prepare"])
        self.assertEqual(self.calls[0][self.calls[0].index("--snapshot") + 1], SNAPSHOT)
        self.assertEqual(self.inputs[0]["maxItems"], 2)
        self.assertNotIn("entrypoint", self.inputs[0])

    def test_continuation_keeps_work_and_literal_filter(self):
        self.args.cursor, self.args.contains = "native:1", "Sweep"
        value, code = self.execute([coverage_page(), work_page(declaration(), cursor="native:2")])
        self.assertEqual(code, 0)
        self.assertEqual(self.inputs[-1], {"query": {"kind": "SYMBOL", "symbolContains": "Sweep"}, "cursor": "native:1"})
        self.assertEqual(value["nextCursor"], "native:2")
        self.assertFalse(value["selectionExhausted"])
        self.assertTrue(value["pageHasNoOmissions"])
        self.assertFalse(value["selectionDeliveryCompleteClaimed"])
        self.assertEqual(len(self.calls), 2)

    def test_multiple_matches_are_returned_without_guessing_a_winner(self):
        value, code = self.execute([coverage_page(), work_page(declaration("tick(int)"), declaration("tick(string)"))])
        self.assertEqual(code, 0)
        self.assertEqual(len(value["items"]), 2)
        self.assertNotEqual(value["items"][0]["symbol"], value["items"][1]["symbol"])

    def test_empty_source_and_declaration_only_remain_explicit(self):
        value, code = self.execute([coverage_page(), work_page(declaration(sources=[], body_status="DECLARATION_ONLY"))])
        self.assertEqual(code, 0)
        self.assertEqual(value["items"][0]["sourceBindingStatus"], "MISSING")
        self.assertEqual(value["items"][0]["bodyStatus"], "DECLARATION_ONLY")
        self.assertEqual(len(self.calls), 2)

    def test_multiple_source_ids_and_absent_body_status_are_not_completeness(self):
        item = declaration(sources=["one", "two"]); item["record"]["normalized"]["documentation"] = None
        value, code = self.execute([coverage_page(), work_page(item)])
        self.assertEqual(code, 0)
        self.assertEqual(value["items"][0]["sourceBindingStatus"], "MULTIPLE_UNREAD")
        self.assertIsNone(value["items"][0]["bodyStatus"])

    def test_oversized_omission_preserves_native_part_recovery(self):
        omission = {"id": "large", "kind": "DEPENDENCY", "reason": "ITEM_EXCEEDS_WORK_BYTE_BUDGET",
                    "recovery": {"command": "docs work read-dependency-part"}}
        value, code = self.execute([coverage_page(), work_page(omitted=[omission])])
        self.assertEqual(code, 0)
        self.assertEqual(value["omitted"], [omission])
        self.assertFalse(value["pageHasNoOmissions"])
        self.assertTrue(value["selectionExhausted"])

    def test_native_failure_preserves_entire_streams_and_exit(self):
        stdout, stderr = b'{"error":{"code":"STALE_REQUIRES_RESLICE"}}\n', b"diagnostic\xff\n" * 12000
        value, code = self.execute([subprocess.CompletedProcess([], 4, stdout, stderr)])
        self.assertEqual((code, value["status"]), (4, "NATIVE_ERROR"))
        receipt = value["rawReceipt"]
        self.assertEqual((self.output / receipt["stdout"]).read_bytes(), stdout)
        self.assertEqual((self.output / receipt["stderr"]).read_bytes(), stderr)
        self.assertEqual(receipt["stderrSha256"], hashlib.sha256(stderr).hexdigest())
        self.assertIn("explicit user permission", value["nextAction"])

    def test_wrong_snapshot_and_foreign_work_are_rejected(self):
        page = work_page(declaration()); page["work"] = "foreign"
        value, code = self.execute([coverage_page(), page])
        self.assertEqual((code, value["status"]), (2, "NATIVE_WORK_MISMATCH"))
        page = coverage_page(); page["snapshot"] = "different"
        value, code = self.execute([page])
        self.assertEqual((code, value["status"]), (2, "NATIVE_SCOPE_MISMATCH"))

    def test_repeated_cursor_is_a_failure(self):
        self.args.cursor = "native:1"
        value, code = self.execute([coverage_page(), work_page(cursor="native:1")])
        self.assertEqual((code, value["status"]), (2, "NON_PROGRESSING_NATIVE_CONTINUATION"))

    def test_foreign_service_row_and_malformed_output_are_rejected(self):
        item = declaration(); item["record"]["service"] = "foreign"
        value, code = self.execute([coverage_page(), work_page(item)])
        self.assertEqual((code, value["status"]), (2, "INVALID_DECLARATION_ROW"))
        value, code = self.execute([subprocess.CompletedProcess([], 0, b"bad json", b"error")])
        self.assertEqual((code, value["status"]), (2, "INVALID_NATIVE_OUTPUT"))

    def test_empty_filtered_result_does_not_claim_behavior_absent(self):
        value, code = self.execute([coverage_page(), work_page()])
        self.assertEqual((code, value["status"]), (0, "NO_DECLARATIONS_ON_PAGE"))
        self.assertIn("not proof", " ".join(value["limitations"]))


if __name__ == "__main__":
    unittest.main()
