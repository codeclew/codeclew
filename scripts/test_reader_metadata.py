#!/usr/bin/env python3
"""Installed skill entry points over owned retained proof and native protocol doubles."""
import json
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parent.parent
F = runpy.run_path(str(ROOT / "scripts/test_reader_markdown_repair.py"))


class ReaderMetadataTest(unittest.TestCase):
    def setUp(self):
        self.case = F["ReaderMarkdownRepairTest"]("test_correct_replacement_preserves_markdown_bindings_and_rendered_controls")
        self.case.setUp()
        self.addCleanup(self.case.doCleanups)
        self.root = self.case.root
        destination = self.root / "installed-skills"
        installed = subprocess.run([str(ROOT / "clew"), "skill", "install", "--destination", str(destination)],
                                   capture_output=True, text=True)
        self.assertEqual(installed.returncode, 0, installed.stderr)
        self.skill = destination / "codeclew"
        self.calls = self.root / "native-calls.jsonl"
        # Renderer remains explicitly caller-owned. Copy the shipped renderer
        # asset so installed helpers do not rely on source-tree Python imports.
        local_asset = self.root / "markdown.js"
        shutil.copyfile(ROOT / "crates/clew/assets/documentation/markdown.js", local_asset)
        renderer = self.case.renderer.read_text().replace(str(ROOT / "crates/clew/assets/documentation/markdown.js"), str(local_asset))
        self.case.renderer.write_text(renderer)

    def native_double(self, *, supported=True):
        """Only the native query/base boundary is doubled; no SOURCE commands."""
        path = self.root / "owned-clew"
        source_map = json.loads((self.case.base / "sources.json").read_bytes())
        path.write_text("#!" + sys.executable + "\n" + '''import json,sys,shutil
from pathlib import Path
config=json.loads(CONFIG)
args=sys.argv[1:]
with Path(config['calls']).open('a') as log:log.write(json.dumps(args)+'\\n')
if args[:3]==['docs','reader','query']:
 value={'reference':'owned-reference','omitted':[],'nextCursor':None,'items':[]}
 if '--cursor' not in args:
  value.update(nextCursor='second-page',items=[{'kind':'SECTION','id':'target','repairSupported':config['supported']}])
 elif args[args.index('--cursor')+1]!='second-page':sys.exit(3)
elif args[:3]==['docs','reader','repair-base']:
 output=Path(args[args.index('--output')+1]);shutil.copytree(config['base'],output)
 value={'claim':'target','old':config['old'],'baseMarkdownDigest':config['markdownDigest'],'baseMapDigest':config['mapDigest']}
else:sys.exit('Unexpected acquisition or mutation command')
print(json.dumps(value))
'''.replace("CONFIG", repr(json.dumps({"calls": str(self.calls), "supported": supported,
            "base": str(self.case.base), "old": self.case.old,
            "markdownDigest": source_map["markdownDigest"], "mapDigest": source_map["mapDigest"]}))))
        path.chmod(0o700)
        return path

    def invoke(self, *, supported=True, old=None):
        replacement = self.root / "replacement.json"
        replacement.write_text(json.dumps({"old": old or self.case.old, "new": self.case.new}))
        output = self.root / "addressed"
        argv = [sys.executable, "-I", "-S", str(self.skill / "scripts/reader_metadata.py"),
                "--clew", str(self.native_double(supported=supported)), "--root", str(self.root / "docs"),
                "--reference", "owned-reference", "--file", str(self.root / "copy.html"),
                "--replacement", str(replacement), "--manifest", str(self.case.manifest),
                "--renderer-command", str(self.case.command()), "--output-dir", str(output)]
        return subprocess.run(argv, capture_output=True, text=True), output

    def test_source_launcher_installs_self_contained_guarded_entry_and_checks_saved_proof(self):
        original = {p.name: p.read_bytes() for p in self.case.base.iterdir()}
        result, output = self.invoke()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout)["status"], "QUERIED_READER_PARAGRAPH_REPAIR_RENDERED")
        self.assertEqual((output / "repair/candidate/index.md").read_bytes(), self.case.text.replace(self.case.old, self.case.new).encode())
        self.assertEqual({p.name: p.read_bytes() for p in self.case.base.iterdir()}, original)
        calls = [json.loads(line)[:3] for line in self.calls.read_text().splitlines()]
        self.assertEqual(calls, [["docs", "reader", "query"], ["docs", "reader", "query"], ["docs", "reader", "repair-base"]])
        command = [sys.executable, "-I", "-S", str(self.skill / "scripts/author_citations.py"),
                   "check-repair", "--repair-directory", str(output / "repair"), "--output", str(self.root / "check.json")]
        checked = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(checked.returncode, 0, checked.stdout + checked.stderr)
        self.assertFalse(json.loads(checked.stdout)["semanticTruthAssessed"])
        (self.case.manifest.parent / "part.stdout.json").write_text("corrupt proof")
        command[-1] = str(self.root / "changed-proof-check.json")
        refused = subprocess.run(command, capture_output=True, text=True)
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("REUSE_NATIVE_STREAM_DIGEST_MISMATCH", refused.stdout)

    def test_unmapped_query_cannot_invoke_base_or_repair(self):
        result, _ = self.invoke(supported=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("READER_SECTION_HAS_NO_GUARDED_MARKDOWN_REPAIR_ADAPTER", result.stdout)
        self.assertTrue(all(json.loads(line)[:3] == ["docs", "reader", "query"] for line in self.calls.read_text().splitlines()))

    def test_explicit_old_text_must_equal_queried_original_paragraph(self):
        result, output = self.invoke(old="An unrelated paragraph.")
        self.assertEqual(result.returncode, 2)
        self.assertIn("QUERIED_PARAGRAPH_DOES_NOT_MATCH_EXPLICIT_OLD_TEXT", result.stdout)
        self.assertFalse((output / "repair").exists())


if __name__ == "__main__":
    unittest.main()
