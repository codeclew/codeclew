#!/usr/bin/env python3
"""Negative controls for the retained READ closure, including rehashed edits."""
import copy
import hashlib
import json
from pathlib import Path
import runpy
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
F = runpy.run_path(str(ROOT / "scripts/test_author_citations.py"))
A, P = F["A"], F["P"]
encode, digest = A["encode"], A["digest"]


def refresh(directory, packet, discovery=None):
    """Coedit wrapper checksums so tests exercise binding checks underneath."""
    if discovery is None:
        discovery = json.loads((directory / "discovery.input.json").read_bytes())
    raw = encode(discovery).encode()
    (directory / "discovery.input.json").write_bytes(raw)
    packet["inputPacketDigest"] = "sha256:" + hashlib.sha256(raw).hexdigest()
    receipts = json.loads((directory / "receipts.json").read_bytes())
    for receipt in receipts:
        for stream in ("stdout", "stderr"):
            receipt[stream + "Sha256"] = hashlib.sha256((directory / receipt[stream]).read_bytes()).hexdigest()
    (directory / "receipts.json").write_text(encode(receipts))
    (directory / "packet.json").write_text(encode(packet))
    (directory / "read-summary.json").write_text(encode({"packetDigest": digest(packet)}))
    (directory / "citations.json").write_text(encode(A["manifest_value"](packet, discovery)))


def part_proof(name, value):
    return {"stdout": name, **{k: value[k] for k in (
        "startByte", "endByte", "recordDigest", "receiptDigest", "authority", "nextCursor"
    ) if k in value}}


class CitationEvidenceTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def fixture(self, label="read"):
        manifest, _, _ = F["fixture"](self.root, label)
        packet = json.loads((manifest.parent / "packet.json").read_bytes())
        return manifest, packet

    def refused(self, manifest):
        with patch("subprocess.run", side_effect=AssertionError("Validation cannot acquire source")):
            with self.assertRaises(A["Stop"]):
                A["load"](manifest)

    def test_rehashed_native_parts_still_require_exact_identity_ranges_and_digests(self):
        changes = {
            "sourceId": "foreign", "reference": "s-other", "startByte": 1,
            "endByte": 1, "totalTextBytes": 999, "fragmentDigest": "wrong",
            "recordDigest": "wrong", "nextCursor": "not-terminal", "snapshot": "other",
        }
        for field, value in changes.items():
            with self.subTest(field=field):
                manifest, packet = self.fixture(field)
                path = manifest.parent / "part.stdout.json"
                part = json.loads(path.read_bytes()); part[field] = value
                part["receiptDigest"] = digest({k: v for k, v in part.items() if k != "receiptDigest"})
                path.write_text(encode(part))
                packet["outcomes"][0]["sourceParts"] = [part_proof(path.name, part)]
                refresh(manifest.parent, packet)
                self.refused(manifest)

    def test_rehashed_pages_cannot_change_declaration_or_scope(self):
        for field in ("work", "snapshot", "subject", "id", "symbol", "sourceIds", "bodyStatus", "scope"):
            with self.subTest(field=field):
                manifest, packet = self.fixture(field)
                path = manifest.parent / "page.stdout.json"
                page = json.loads(path.read_bytes())
                if field in ("work", "snapshot", "subject"):
                    page[field] = "foreign"
                elif field == "id":
                    page["items"][0]["id"] = "foreign"
                elif field in ("bodyStatus", "scope"):
                    norm = page["items"][0]["record"]["normalized"]
                    (norm["documentation"] if field == "bodyStatus" else norm)[field] = "foreign"
                else:
                    page["items"][0]["record"][field] = [] if field == "sourceIds" else "foreign"
                path.write_text(encode(page))
                packet["outcomes"][0]["nativePages"][0]["metadata"] = {k: v for k, v in page.items() if k != "items"}
                refresh(manifest.parent, packet)
                self.refused(manifest)

    def test_coedited_discovery_and_packet_do_not_rebind_native_work_service_or_snapshot(self):
        for field in ("work", "service", "snapshot"):
            with self.subTest(field=field):
                manifest, packet = self.fixture(field)
                discovery = json.loads((manifest.parent / "discovery.input.json").read_bytes())
                if field == "work":
                    discovery[field] = packet[field] = "foreign"
                else:
                    discovery["scope"][field] = packet["scope"][field] = "foreign"
                refresh(manifest.parent, packet, discovery)
                self.refused(manifest)

    def test_complete_multipart_utf8_proof_and_incomplete_overlap_reordering_refusals(self):
        manifest, packet = self.fixture()
        directory = manifest.parent
        record = {**packet["outcomes"][0]["source"], "text": next(iter(packet["texts"].values()))}
        split = len(record["text"][:12].encode())
        total = len(record["text"].encode())
        receipts = json.loads((directory / "receipts.json").read_bytes())
        names = []
        for index, (start, end, cursor) in enumerate([(0, split, "next"), (split, total, None)]):
            name = f"multipart-{index}.json"; names.append(name)
            part = F["part"](record, start, end, cursor)
            (directory / name).write_text(encode(part))
            receipts.append({**receipts[-1], "stdout": name})
        (directory / "receipts.json").write_text(encode(receipts))
        packet["outcomes"][0]["sourceParts"] = [part_proof(name, json.loads((directory / name).read_bytes())) for name in names]
        refresh(directory, packet)
        with patch("subprocess.run", side_effect=AssertionError("No source acquisition")):
            _, _, _, entries = A["load"](manifest)
        self.assertEqual(next(iter(entries.values()))["source"], packet["outcomes"][0]["source"])
        complete = copy.deepcopy(packet)
        for indices in ([1, 0], [0], [0, 0, 1]):
            packet = copy.deepcopy(complete)
            packet["outcomes"][0]["sourceParts"] = [complete["outcomes"][0]["sourceParts"][i] for i in indices]
            refresh(directory, packet)
            self.refused(manifest)

    def test_source_navigation_binding_uses_the_same_complete_native_source_proof(self):
        manifest, packet = self.fixture()
        directory = manifest.parent
        outcome = packet["outcomes"][0]
        binding = {"id": outcome["source"]["id"], "reference": "s9", "source": outcome["source"],
                   "recordDigest": outcome["sourceRecordDigest"], "raw": {"stdout": "page.stdout.json", "pointer": "/items/0"}}
        page = json.loads((directory / "page.stdout.json").read_bytes())
        page["items"] = [{"kind": "SOURCE_NAVIGATION", "id": binding["id"], "record": {
            "source": binding["source"], "recordDigest": binding["recordDigest"],
            "fullRecordReference": "s9", "textDelivered": False}}]
        (directory / "page.stdout.json").write_text(encode(page))
        outcome.pop("declaration"); outcome.update(status="RETAINED_SOURCE_RETURNED", sourceBinding=binding,
            selection={"file": outcome["source"]["file"]})
        discovery = json.loads((directory / "discovery.input.json").read_bytes())
        discovery["choices"] = [copy.deepcopy(binding)]
        refresh(directory, packet, discovery)
        A["load"](manifest)
        page["items"][0]["record"]["fullRecordReference"] = "s-other"
        (directory / "page.stdout.json").write_text(encode(page))
        refresh(directory, packet, discovery)
        self.refused(manifest)

    def test_symlinked_and_escaping_receipt_artifacts_are_refused(self):
        for symlink in (False, True):
            manifest, packet = self.fixture(str(symlink))
            directory = manifest.parent
            raw = (directory / "part.stdout.json").read_bytes()
            outside = self.root / ("outside-" + str(symlink) + ".json"); outside.write_bytes(raw)
            if symlink:
                (directory / "part.stdout.json").unlink()
                (directory / "part.stdout.json").symlink_to(outside)
            else:
                receipts = json.loads((directory / "receipts.json").read_bytes())
                receipts[-1]["stdout"] = "../" + outside.name
                (directory / "receipts.json").write_text(encode(receipts))
            self.refused(manifest)


if __name__ == "__main__":
    unittest.main()
