#!/usr/bin/env python3
"""Owned protocol checks for exact Work declaration-to-SOURCE binding reads."""
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
M = runpy.run_path(str(ROOT / "skills/codeclew/scripts/bound_source.py"))
BoundSource, digest = M["BoundSource"], M["digest"]
SNAPSHOT = "sha256:owned/1"


def source(text="public class Ledger { /* π🙂 */ }"):
    return {"id": "source-b", "service": "owned", "revision": "rev-a", "file": "Ledger.cs",
            "startLine": 3, "endLine": 3, "authority": "EXACT_SNAPSHOT_TEXT", "text": text,
            "textDigest": "sha256:" + hashlib.sha256(text.encode()).hexdigest()}


def declaration(ids=None, references=None):
    return {"kind": "DEPENDENCY", "id": "decl-b", "reference": "d9",
            "sourceReferences": ["s9"] if references is None else references,
            "record": {"id": "decl-b", "kind": "SYMBOL", "service": "owned", "symbol": "class:Ledger",
                "sourceIds": ["source-b"] if ids is None else ids, "digest": "sha256:descriptor",
                "normalized": {"documentation": {"bodyStatus": "DECLARATION_ONLY"}}}}


def page(*items, cursor=None, omitted=None):
    return {"schema": "codeclew-documentation-work-page/1.0", "work": "work-a",
            "subject": "service:owned", "snapshot": SNAPSHOT, "items": list(items),
            "omitted": omitted or [], "nextCursor": cursor}


def part(record=None, start=0, end=None, cursor=None, reference="s9"):
    record = record or source()
    text = record["text"].encode(); end = len(text) if end is None else end
    fragment = text[start:end].decode()
    value = {"schema": "codeclew-documentation-source-part/1.0", "work": "work-a",
        "snapshot": SNAPSHOT, "reference": reference, "sourceId": record["id"],
        "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED", "recordDigest": digest(record),
        "source": {k: v for k, v in record.items() if k != "text"}, "startByte": start, "endByte": end,
        "totalTextBytes": len(text), "text": fragment, "fragmentDigest": "sha256:" + hashlib.sha256(fragment.encode()).hexdigest(),
        "nextCursor": cursor, "receiptType": "SOURCE_PART"}
    return {**value, "receiptDigest": digest(value)}


class WorkBoundSourceTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(); self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.output = self.directory / "receipts"; self.output.mkdir()
        self.input = self.directory / "catalogue.json"
        self.row = {"kind": "DECLARATION", "id": "decl-b", "reference": "d9", "symbol": "class:Ledger",
                    "service": "owned", "revision": "rev-a", "sourceIds": ["source-b"],
                    "observationDigest": "sha256:descriptor"}
        self.packet = {"schema": "codeclew-declaration-catalogue/1.0", "work": "work-a",
                       "selection": {"root": "/original/docs", "service": "owned", "snapshot": SNAPSHOT},
                       "items": [{**self.row, "id": "decl-a", "reference": "d8", "sourceIds": ["source-a"]}, self.row]}
        self.args = argparse.Namespace(clew="/supported/bin/clew", root="/disposable/docs", service="owned",
            snapshot=SNAPSHOT, catalogue_result=self.input, declaration_id="decl-b", source_id=None,
            source_reference=None, output_dir=self.output)
        self.commands, self.inputs = [], []

    def execute(self, packets):
        self.input.write_text(json.dumps(self.packet))
        sequence = iter(packets); read_bytes, read_text = Path.read_bytes, Path.read_text
        def allowed_artifact(path):
            self.assertEqual(path, self.input)
            return read_bytes(path)
        def native(command, **kwargs):
            self.commands.append(command)
            self.assertIn(command[1:4], [["docs", "work", "read"], ["docs", "work", "read-part"]])
            self.assertNotIn("--symbol", command)
            self.inputs.append(json.loads(read_text(Path(command[command.index("--input") + 1]))))
            value = next(sequence)
            return value if isinstance(value, subprocess.CompletedProcess) else subprocess.CompletedProcess(
                command, 0, json.dumps(value, ensure_ascii=False).encode(), b"diagnostic\xff\n")
        with patch("subprocess.run", side_effect=native), \
             patch.object(Path, "read_bytes", allowed_artifact), \
             patch.object(Path, "read_text", side_effect=AssertionError("raw target read forbidden")), \
             patch("builtins.open", side_effect=AssertionError("raw target read forbidden")):
            value, code = BoundSource(self.args).result()
        self.assertFalse(value["sourceFallbackAllowed"])
        self.assertFalse(value["completeDeclarationClaimed"])
        self.assertFalse(value["callFlowClosureClaimed"])
        self.assertEqual(json.loads((self.output / "result.json").read_text()), value)
        return value, code

    def test_explicit_record_avoids_ambiguous_class_selector_and_revalidates_scope(self):
        coverage = {"kind": "COVERAGE", "id": "owned", "revision": "rev-a", "boundaries": ["SCOPE_AMBIGUOUS:class:Ledger"]}
        value, code = self.execute([page(coverage, declaration(), cursor="unread-source"), part()])
        self.assertEqual(code, 0)
        self.assertEqual(self.inputs[0], {"references": ["d9"]})
        self.assertEqual(value["sourceItems"][0]["record"], source())
        self.assertEqual(value["declaration"]["id"], "decl-b")
        self.assertEqual(value["declaration"]["bodyStatus"], "DECLARATION_ONLY")
        self.assertEqual(value["catalogueInput"]["catalogueRoot"], "/original/docs")
        self.assertEqual(value["nativePages"][0]["coverage"], [coverage])

    def test_multiple_bindings_never_choose_first(self):
        self.row["sourceIds"] = ["source-a", "source-b"]
        value, code = self.execute([page(declaration(self.row["sourceIds"], ["s8", "s9"]))])
        self.assertEqual((code, value["status"]), (3, "EXPLICIT_SOURCE_CHOICE_REQUIRED"))
        self.assertEqual(len(self.commands), 1)

    def test_explicit_source_still_requires_reference_choice_if_unmapped(self):
        self.row["sourceIds"] = ["source-a", "source-b"]; self.args.source_id = "source-b"
        value, code = self.execute([page(declaration(self.row["sourceIds"], ["s8", "s9"]))])
        self.assertEqual((code, value["status"]), (3, "EXPLICIT_SOURCE_REFERENCE_CHOICE_REQUIRED"))
        self.assertEqual(len(self.commands), 1)

    def test_explicit_pair_is_verified_by_native_part_id(self):
        self.row["sourceIds"] = ["source-a", "source-b"]
        self.args.source_id, self.args.source_reference = "source-b", "s9"
        value, code = self.execute([page(declaration(self.row["sourceIds"], ["s8", "s9"])), part()])
        self.assertEqual(code, 0)
        self.assertEqual(value["sourceItems"][0]["id"], "source-b")

    def test_reference_to_different_source_is_rejected(self):
        wrong = source(); wrong["id"] = "source-a"
        value, code = self.execute([page(declaration()), part(wrong)])
        self.assertEqual((code, value["status"]), (2, "SOURCE_PART_IDENTITY_MISMATCH"))

    def test_large_unicode_source_is_assembled_without_gaps_or_clipping(self):
        record = source("π🙂" * 15000)
        cut = len(("π🙂" * 7000).encode())
        value, code = self.execute([page(declaration()), part(record, end=cut, cursor="next"), part(record, start=cut)])
        self.assertEqual(code, 0)
        self.assertEqual(value["sourceItems"][0]["record"], record)
        self.assertEqual(self.inputs[-1]["cursor"], "next")
        self.assertEqual(len(value["sourceParts"]), 2)

    def test_missing_binding_is_explicit(self):
        self.row["sourceIds"] = []
        value, code = self.execute([page(declaration([], []))])
        self.assertEqual((code, value["status"]), (3, "MISSING_DECLARATION_SOURCE"))

    def test_missing_reference_has_supported_exact_source_recovery(self):
        value, code = self.execute([page(declaration(references=[]))])
        self.assertEqual((code, value["status"]), (3, "MISSING_NATIVE_SOURCE_REFERENCES"))
        self.assertEqual(value["recovery"]["source"], "source-b")

    def test_omitted_declaration_retains_native_dependency_part_recovery(self):
        value, code = self.execute([page(omitted=[{"id": "decl-b", "reason": "ITEM_EXCEEDS_WORK_BYTE_BUDGET"}])])
        self.assertEqual((code, value["status"]), (3, "DECLARATION_BINDING_OMITTED"))
        self.assertEqual(value["recovery"]["command"], "docs work read-dependency-part")
        self.assertEqual(len(self.commands), 1)

    def test_changed_native_declaration_not_trusted_from_saved_catalogue(self):
        row = declaration(); row["record"]["sourceIds"] = ["source-other"]
        value, code = self.execute([page(row)])
        self.assertEqual((code, value["status"]), (2, "NATIVE_DECLARATION_BINDING_CHANGED"))

    def test_wrong_catalogue_scope_is_rejected_before_native_calls(self):
        self.packet["selection"]["snapshot"] = "other"
        value, code = self.execute([])
        self.assertEqual((code, value["status"]), (2, "CATALOGUE_SCOPE_MISMATCH"))
        self.assertEqual(self.commands, [])

    def test_native_error_keeps_stdout_stderr_and_exit(self):
        stdout, stderr = b'{"error":{"code":"AMBIGUOUS_SYMBOL"}}\n', b"failure\xff\n" * 16000
        value, code = self.execute([subprocess.CompletedProcess([], 5, stdout, stderr)])
        self.assertEqual((code, value["status"]), (5, "NATIVE_ERROR"))
        receipt = value["rawReceipt"]
        self.assertEqual((self.output / receipt["stdout"]).read_bytes(), stdout)
        self.assertEqual((self.output / receipt["stderr"]).read_bytes(), stderr)

    def test_part_range_digest_and_revision_changes_are_failures(self):
        for field, replacement, expected in [("startByte", 1, "SOURCE_PART_RANGE_OR_DIGEST_MISMATCH"),
                                               ("fragmentDigest", "bad", "SOURCE_PART_RANGE_OR_DIGEST_MISMATCH")]:
            p = part(); p[field] = replacement
            value, code = self.execute([page(declaration()), p])
            self.assertEqual((code, value["status"]), (2, expected))
        p = part(); p["source"]["revision"] = "other"
        value, code = self.execute([page(declaration()), p])
        self.assertEqual((code, value["status"]), (2, "SOURCE_BINDING_SCOPE_MISMATCH"))

    def test_required_declaration_pagination_is_preserved(self):
        value, code = self.execute([page(cursor="decl:1"), page(declaration()), part()])
        self.assertEqual(code, 0)
        self.assertEqual(self.inputs[1], {"references": ["d9"], "cursor": "decl:1"})


if __name__ == "__main__":
    unittest.main()
