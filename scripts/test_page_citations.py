#!/usr/bin/env python3
"""Owned two-page location binding and checked incremental provenance."""
import copy
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
F = runpy.run_path(str(ROOT / "scripts/test_author_citations.py"))
M = runpy.run_path(str(ROOT / "skills/codeclew/scripts/page_citations.py"))
E = runpy.run_path(str(ROOT / "scripts/test_citation_evidence.py"))


class PageCitationsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.pages = self.root / "pages"; self.pages.mkdir()
        self.a, av, _ = F["fixture"](self.root, "a")
        self.b, bv, _ = F["fixture"](self.root, "b", service="other", work="work-b")
        self.ha, self.hb = av["records"][0]["cite"], bv["records"][0]["cite"]
        self.first = b"# Guide\r\n\r\nRepeated.\r\n\r\n| Field | Meaning |\r\n| --- | --- |\r\n| " + "π".encode() + b" | Repeated. |\r\n\r\n- first " + "🙂".encode() + b" item\r\n- second item\r\n\r\nRepeated.\r\n"
        (self.pages / "guide.md").write_bytes(self.first)
        (self.pages / "other.md").write_bytes(b"# Other\r\n\r\nRepeated.\r\n")
        self.serial = 0

    def spec(self, name):
        raw = (self.pages / name).read_bytes(); handle = self.ha if name == "guide.md" else self.hb
        first = raw.index(b"Repeated."); last = raw.rindex(b"Repeated.")
        claims = [{"id": "target", "startByte": first, "endByte": first + 9, "evidence": [handle]}]
        if name == "guide.md":
            claims += [{"id": "table", "startLine": 5, "endLine": 7, "evidence": [handle]},
                       {"id": "list", "startLine": 9, "endLine": 10, "evidence": [handle]},
                       {"id": "again", "startByte": last, "endByte": last + 9, "evidence": [handle]}]
        return {"page": name, "markdownDigest": M["sha"](raw), "claims": claims}

    def bind(self, specs=None, previous=None):
        self.serial += 1; request = self.root / f"request-{self.serial}.json"; output = self.root / f"package-{self.serial}"
        request.write_text(json.dumps({"schema": M["REQUEST"], "pages": specs if specs is not None else [self.spec("guide.md"), self.spec("other.md")]}))
        with patch("subprocess.run", side_effect=AssertionError("Binding cannot acquire source or call a model")):
            result = M["bind"]([self.a, self.b], self.pages, request, output, previous=previous)
        return output, result

    def mapped(self, output, page):
        return json.loads((output / "maps" / (page + ".json")).read_bytes())

    def refused(self, specs, status, previous=None):
        with self.assertRaises(M["Stop"]) as error:
            self.bind(specs, previous)
        self.assertEqual(error.exception.status, status)
        self.assertFalse((self.root / f"package-{self.serial}").exists())

    def test_exact_paragraph_table_list_and_repeated_spans_keep_full_pages_and_page_local_ids(self):
        output, result = self.bind()
        self.assertEqual((result["pages"], result["claims"]), (2, 5))
        self.assertNotIn("Repeated.", json.dumps([self.spec("guide.md"), self.spec("other.md")]))
        for page, source, work in [("guide.md", self.ha, "work-a"), ("other.md", self.hb, "work-b")]:
            mapped = self.mapped(output, page)
            self.assertEqual((output / "markdown" / page).read_bytes(), (self.pages / page).read_bytes())
            self.assertEqual(mapped["schema"], "codeclew-authored-sources/1.0")
            for claim in mapped["claims"]:
                raw = (self.pages / page).read_bytes(); pos = claim["location"]
                self.assertEqual(raw[pos["startByte"]:pos["endByte"]].decode(), claim["text"])
                self.assertEqual(claim["evidence"], [source])
                self.assertEqual(claim["sources"][0]["work"], work)
            self.assertEqual(mapped["claims"][0]["id"], "target")
        claims = self.mapped(output, "guide.md")["claims"]
        self.assertEqual(claims[0]["text"], claims[-1]["text"])
        self.assertNotEqual(claims[0]["location"], claims[-1]["location"])
        self.assertIn("| π | Repeated. |\r\n", claims[1]["text"])
        self.assertEqual(claims[2]["text"], "- first 🙂 item\r\n- second item\r\n")
        index = json.loads((output / "index.json").read_bytes()); self.assertNotIn("markdownDigest", index)
        self.assertEqual(M["check"]([self.a, self.b], output / "index.json")["claims"], 5)

    def test_minimal_update_rechecks_and_reuses_unchanged_map_with_explicit_removal_and_rebinding(self):
        output, _ = self.bind(); old_map = (output / "maps/other.md.json").read_bytes()
        old_page = (output / "markdown/other.md").read_bytes()
        old_spec = self.spec("guide.md")
        (self.pages / "guide.md").write_bytes(self.first.replace(b"first ", b"updated first "))
        self.refused([old_spec], "STALE_PAGE_DIGEST", output / "index.json")
        spec = self.spec("guide.md"); spec["claims"] = [c for c in spec["claims"] if c["id"] != "list"]
        spec["claims"][0]["evidence"] = [self.hb]
        self.refused([spec], "EXPLICIT_CLAIM_REMOVAL_OR_REBIND_REQUIRED", output / "index.json")
        spec.update(remove=["list"], replace=["target"])
        updated, result = self.bind([spec], output / "index.json")
        self.assertEqual(result["reusedPages"], 1)
        self.assertEqual((updated / "maps/other.md.json").read_bytes(), old_map)
        self.assertEqual((updated / "markdown/other.md").read_bytes(), old_page)
        self.assertEqual(self.mapped(updated, "guide.md")["claims"][0]["sources"][0]["work"], "work-b")
        self.assertEqual((updated / "markdown/guide.md").read_bytes(), (self.pages / "guide.md").read_bytes())
        self.assertEqual(M["check"]([self.a, self.b], updated / "index.json")["claims"], 4)

    def test_unspecified_changed_page_cannot_be_reused(self):
        output, _ = self.bind(); (self.pages / "other.md").write_bytes(b"Changed without fresh selection.\n")
        self.refused([self.spec("guide.md")], "PAGE_SELECTION_REFRESH_REQUIRED", output / "index.json")

    def test_invalid_utf8_crlf_ranges_digests_and_ambiguous_selectors_refuse(self):
        raw = self.first; pi = raw.index("π".encode()); cr = raw.index(b"\r\n")
        cases = [({"startByte": pi + 1, "endByte": pi + 2}, "INVALID_UTF8_SELECTION"),
                 ({"startByte": 0, "endByte": cr + 1}, "SELECTION_SPLITS_CRLF"),
                 ({"startByte": 1, "endByte": 1}, "INVALID_BYTE_RANGE"),
                 ({"startByte": 0, "endByte": len(raw) + 1}, "INVALID_BYTE_RANGE"),
                 ({"startLine": 0, "endLine": 2}, "INVALID_LINE_RANGE"),
                 ({"startLine": 2, "endLine": 2}, "EMPTY_LOCATION_CLAIM"),
                 ({"startLine": 3, "endLine": 3, "startByte": 0, "endByte": 4}, "AMBIGUOUS_OR_MISSING_LOCATION")]
        for selection, status in cases:
            spec = self.spec("guide.md"); spec["claims"] = [{"id": "target", "evidence": [self.ha], **selection}]
            with self.subTest(selection=selection): self.refused([spec], status)
        spec = self.spec("guide.md"); spec["claims"][0]["textDigest"] = "wrong"
        self.refused([spec], "SELECTION_DIGEST_MISMATCH")
        spec = self.spec("guide.md"); spec["claims"][0]["text"] = "Repeated."
        self.refused([spec], "INVALID_LOCATION_CLAIM")

    def test_duplicate_page_claim_and_foreign_evidence_are_not_merged(self):
        spec = self.spec("guide.md"); spec["claims"].append(copy.deepcopy(spec["claims"][0]))
        self.refused([spec], "INVALID_OR_DUPLICATE_PAGE_CLAIM")
        spec = self.spec("guide.md"); spec["claims"][0]["evidence"] = ["foreign:e1"]
        self.refused([spec], "UNKNOWN_OR_FOREIGN_CITATION_HANDLE")
        self.refused([self.spec("guide.md"), self.spec("guide.md")], "DUPLICATE_PAGE_SELECTION")

    def test_unrepresentable_overlapping_literal_is_never_moved_to_another_occurrence(self):
        raw = b"aaaa\n"; (self.pages / "overlap.md").write_bytes(raw)
        spec = {"page": "overlap.md", "markdownDigest": M["sha"](raw),
                "claims": [{"id": "target", "startByte": 1, "endByte": 4, "evidence": [self.ha]}]}
        self.refused([spec], "LOCATION_NOT_REPRESENTABLE_BY_LITERAL_CONTRACT")

    def test_rehashed_index_or_page_maps_cannot_substitute_another_pages_proof(self):
        output, _ = self.bind(); path = output / "index.json"; index = json.loads(path.read_bytes())
        original = path.read_bytes(); index["indexDigest"] = "wrong"; path.write_text(json.dumps(index))
        with self.assertRaises(M["Stop"]) as error: M["check"]([self.a, self.b], path)
        self.assertEqual(error.exception.status, "INVALID_PAGE_INDEX_DIGEST"); path.write_bytes(original)
        index = json.loads(original); mapped = self.mapped(output, "other.md")
        (output / "maps/guide.md.json").write_bytes((output / "maps/other.md.json").read_bytes())
        index["pages"][0]["mapDigest"] = mapped["mapDigest"]
        index["pages"][0]["markdownDigest"] = mapped["markdownDigest"]
        index["indexDigest"] = M["digest"]({k: v for k, v in index.items() if k != "indexDigest"}); path.write_text(M["encode"](index))
        with self.assertRaises(M["Stop"]): M["check"]([self.a, self.b], path)

    def test_check_reconstructs_native_bindings_after_outer_hashes_are_recomputed(self):
        output, _ = self.bind(); directory = self.a.parent
        packet = json.loads((directory / "packet.json").read_bytes())
        packet["outcomes"][0]["source"]["revision"] = "foreign"
        E["refresh"](directory, packet)
        with self.assertRaises(M["Stop"]): M["check"]([self.a, self.b], output / "index.json")

    def test_wrong_page_map_refuses_even_when_complete_reader_pages_have_identical_hashes(self):
        (self.pages / "other.md").write_bytes(self.first)
        first = self.spec("guide.md"); second = copy.deepcopy(first); second["page"] = "other.md"
        for claim in second["claims"]: claim["evidence"] = [self.hb]
        output, _ = self.bind([first, second])
        path = output / "index.json"; index = json.loads(path.read_bytes())
        self.assertEqual(index["pages"][0]["markdownDigest"], index["pages"][1]["markdownDigest"])
        (output / "maps/guide.md.json").write_bytes((output / "maps/other.md.json").read_bytes())
        index["pages"][0]["mapDigest"] = index["pages"][1]["mapDigest"]
        index["indexDigest"] = M["digest"]({k: v for k, v in index.items() if k != "indexDigest"})
        path.write_text(M["encode"](index))
        with self.assertRaises(M["Stop"]) as error: M["check"]([self.a, self.b], path)
        self.assertEqual(error.exception.status, "PAGE_MANIFEST_CLOSURE_MISMATCH")

    def test_rehashed_native_part_corruption_cannot_pass_index_reconstruction(self):
        output, _ = self.bind(); directory = self.a.parent
        packet = json.loads((directory / "packet.json").read_bytes())
        path = directory / "part.stdout.json"; part = json.loads(path.read_bytes())
        part["text"] = part["text"].replace("Ledger", "Forged")
        part["fragmentDigest"] = M["sha"](part["text"].encode())
        part["receiptDigest"] = M["digest"]({k: v for k, v in part.items() if k != "receiptDigest"})
        path.write_text(M["encode"](part))
        packet["outcomes"][0]["sourceParts"] = [E["part_proof"](path.name, part)]
        E["refresh"](directory, packet)
        with self.assertRaises(M["Stop"]) as error: M["check"]([self.a, self.b], output / "index.json")
        self.assertEqual(error.exception.status, "REUSE_RECORD_DIGEST_MISMATCH")

    def test_rehashed_checks_cannot_invent_semantic_approval(self):
        output, _ = self.bind(); path = output / "index.json"; index = json.loads(path.read_bytes())
        entry = index["pages"][0]; checks_path = output / entry["checks"]
        checks = json.loads(checks_path.read_bytes()); checks["semanticTruthAssessed"] = True
        checks_path.write_text(M["encode"](checks))
        entry["checksDigest"] = M["sha"](checks_path.read_bytes())
        index["indexDigest"] = M["digest"]({k: v for k, v in index.items() if k != "indexDigest"})
        path.write_text(M["encode"](index))
        with self.assertRaises(M["Stop"]) as error: M["check"]([self.a, self.b], path)
        self.assertEqual(error.exception.status, "PAGE_CHECKS_PROOF_MISMATCH")

    def test_explicit_cite_expansion_retains_original_selection_and_rechecks_without_flattening(self):
        markdown = ("Read [source](cite:" + self.ha + ").\r\n").encode()
        (self.pages / "links.md").write_bytes(markdown)
        spec = {"page": "links.md", "markdownDigest": M["sha"](markdown),
                "claims": [{"id": "link", "startByte": 0, "endByte": len(markdown), "evidence": [self.ha]}]}
        request = self.root / "links.json"; request.write_text(json.dumps({"schema": M["REQUEST"], "pages": [spec]}))
        mapping = self.root / "repositories.json"; mapping.write_text(json.dumps({"owned": {
            "repository": "https://example.invalid/owned", "urlTemplate": "https://example.invalid/owned/blob/{revision}/{path}#L{startLine}-L{endLine}"}}))
        output = self.root / "linked"
        M["bind"]([self.a], self.pages, request, output, repositories=mapping)
        self.assertEqual((output / "inputs/links.md").read_bytes(), markdown)
        self.assertIn(b"https://example.invalid/owned/blob/", (output / "markdown/links.md").read_bytes())
        self.assertEqual((self.pages / "links.md").read_bytes(), markdown)
        self.assertEqual(M["check"]([self.a], output / "index.json")["claims"], 1)

    def test_supported_installed_skill_locations_bind_and_check_are_body_free_on_stdout(self):
        destination = self.root / "installed"
        installed = subprocess.run([str(ROOT / "clew"), "skill", "install", "--destination", str(destination)], capture_output=True, text=True)
        self.assertEqual(installed.returncode, 0, installed.stderr)
        cli = [sys.executable, "-I", "-S", str(destination / "codeclew/scripts/page_citations.py")]
        locations = self.root / "locations.json"
        located = subprocess.run([*cli, "locations", "--markdown-root", str(self.pages), "--page", "guide.md", "--page", "other.md", "--output", str(locations)], capture_output=True, text=True)
        self.assertEqual(located.returncode, 0, located.stdout + located.stderr)
        digests = {p["page"]: p["markdownDigest"] for p in json.loads(locations.read_bytes())["pages"]}
        specs = [self.spec(name) for name in digests]
        for spec in specs: spec["markdownDigest"] = digests[spec["page"]]
        request = self.root / "cli-request.json"; request.write_text(json.dumps({"schema": M["REQUEST"], "pages": specs}))
        output = self.root / "cli-package"
        bound = subprocess.run([*cli, "bind", "--markdown-root", str(self.pages), "--request", str(request), "--manifest", str(self.a), "--manifest", str(self.b), "--output-dir", str(output)], capture_output=True, text=True)
        self.assertEqual(bound.returncode, 0, bound.stdout + bound.stderr)
        checked = subprocess.run([*cli, "check", "--index", str(output / "index.json"), "--manifest", str(self.a), "--manifest", str(self.b)], capture_output=True, text=True)
        self.assertEqual(checked.returncode, 0, checked.stdout + checked.stderr)
        for result in (located, bound, checked):
            self.assertLess(len(result.stdout.encode()), 2048)
            self.assertNotIn("Repeated.", result.stdout)
            self.assertNotIn("🙂", result.stdout)
        self.assertEqual(json.loads(checked.stdout)["claims"], 5)


if __name__ == "__main__":
    unittest.main()
