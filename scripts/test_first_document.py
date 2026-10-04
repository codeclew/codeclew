#!/usr/bin/env python3
"""Exercise first-document preparation without providers or private state access."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
ASSET = ROOT / "crates/clew/assets/documentation/examples/first-document.py"
SPEC = importlib.util.spec_from_file_location("first_document", ASSET)
starter = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(starter)


def source(reference, start, end, blob="blob"):
    return {"reference": reference, "kind": "SOURCE", "record": {
        "file": "worker.py", "revision": "commit", "authority": "EXACT_SNAPSHOT_TEXT",
        "occurrence": {"startByte": start, "endByte": end, "snapshot": "saved", "blob": blob},
    }}


class FirstDocumentTest(unittest.TestCase):
    def test_nested_source_delivery_is_complete_without_repeating_children(self):
        rows = [source("body", 0, 100), source("call", 10, 20),
                source("duplicate", 0, 100), source("crossing", 90, 120),
                source("other-authority", 10, 20, "other"),
                {"reference": "omitted", "kind": "SOURCE"},
                {"reference": "whole-file", "record": {"occurrence": None}}]
        self.assertEqual(starter.select_sources(rows),
                         {"body", "crossing", "other-authority", "omitted", "whole-file"})

    def test_native_cursor_is_preserved_and_loop_fails(self):
        requests = []
        def read(selection):
            requests.append(selection)
            return {"nextCursor": "second" if len(requests) == 1 else None}
        self.assertEqual(len(list(starter.pages(read, {"references": ["s9"]}))), 2)
        self.assertEqual(requests[-1], {"references": ["s9"], "cursor": "second"})
        with self.assertRaisesRegex(ValueError, "Repeated native cursor"):
            list(starter.pages(lambda _: {"nextCursor": "same"}, {}))

    def test_saved_snapshot_read_never_captures_and_preserves_previous_work(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "codeclew-docs.yaml").write_text("initialized")
            (root / "notes").mkdir()
            note = root / "notes/owner.md"
            note.write_text("Retain this original guidance")
            calls = []
            def cli(executable, selected_root, *args, **kwargs):
                calls.append(args)
                if args[:2] == ("work", "prepare"):
                    self.assertEqual(args[args.index("--snapshot") + 1], "exact-saved")
                    return {"work": "real-work", "items": [{"kind": "SECTION",
                        "id": "section-overview", "reference": "section7"}], "nextCursor": None}
                request = json.loads(Path(args[args.index("--input") + 1]).read_text())
                if args[:2] == ("work", "read"):
                    return {"items": [source("s9", 0, 100), source("s10", 10, 20)],
                            "omitted": [], "nextCursor": None}
                if args[:2] == ("work", "read-part"):
                    self.assertEqual(request["reference"], "s9")
                    return {"reference": "s9", "source": {"file": "worker.py"},
                            "text": "def run():\n    return 1\n", "nextCursor": None}
                self.fail(f"Unexpected mutable/provider command: {args}")
            args = SimpleNamespace(root=str(root), clew="clew", mode="read", service="worker",
                                   snapshot="exact-saved", audience="Maintainers", question="What returns?")
            with patch.object(starter, "cli", cli), contextlib.redirect_stdout(io.StringIO()), \
                    contextlib.redirect_stderr(io.StringIO()):
                starter.prepare(args)
                first = next((root / "authoring").iterdir())
                (first / "proposal.json").write_text("My edited proposal")
                starter.prepare(args)
            self.assertEqual(note.read_text(), "Retain this original guidance")
            self.assertEqual((first / "proposal.json").read_text(), "My edited proposal")
            second = next(path for path in (root / "authoring").iterdir() if path != first)
            proposal = json.loads((second / "proposal.json").read_text())
            self.assertEqual(proposal["operations"][0]["entrypoint"], "section7")
            self.assertEqual(proposal["operations"][0]["summary"]["evidence"], [])
            packet = (second / "packet.md").read_text()
            self.assertIn("What returns?", packet)
            self.assertIn("return 1", packet)
            self.assertEqual(sum(call[:2] == ("work", "read-part") for call in calls), 2)

    def test_different_existing_registration_stops_before_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "codeclew-docs.yaml").write_text("initialized")
            calls = []
            def cli(executable, selected_root, *args, **kwargs):
                calls.append(args)
                return {"inputDigest": "sha256:catalogue", "nextCursor": None,
                        "items": [{"id": "worker", "record": {"title": "Existing registration"}}]}
            args = SimpleNamespace(root=str(root), clew="clew", mode="capture", service="worker",
                                   repo=str(root), title=None, repository="https://example.org/worker",
                                   language="python", dialect="3.11", source_root=["worker.py"])
            with patch.object(starter, "cli", cli), \
                    patch.object(starter.subprocess, "check_output", return_value="commit\n"):
                with self.assertRaisesRegex(ValueError, "different registration"):
                    starter.prepare(args)
            self.assertEqual(calls, [("service", "list")])

    def test_public_starter_is_the_shipped_asset(self):
        self.assertEqual(ASSET.read_bytes(),
                         (ROOT / "site/examples/codeclew-source/reproduce/first-document.py").read_bytes())


if __name__ == "__main__":
    unittest.main()
