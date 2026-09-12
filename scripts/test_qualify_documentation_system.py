#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec=importlib.util.spec_from_file_location("qualification",Path(__file__).with_name("qualify_documentation_system.py"))
q=importlib.util.module_from_spec(spec);spec.loader.exec_module(q)


class CorpusTest(unittest.TestCase):
    def setUp(self):
        self.spec=json.loads((q.ROOT/"fixtures/documentation-system/qualification/corpus.json").read_text())

    def test_corpus_is_deterministic_and_measures_real_files_with_forty_routes(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            a=q.generate(self.spec,root/"a");b=q.generate(self.spec,root/"b")
            self.assertEqual(a,b)
            self.assertEqual(len(a["services"]),40)
            self.assertEqual((root/"a/orders/Orders.java").read_text().count("@GetMapping"),40)
            files=[p for p in (root/"a").rglob("*") if p.is_file() and p.name!="corpus.json"]
            self.assertEqual(a["files"],len(files))
            self.assertEqual(a["bytes"],sum(p.stat().st_size for p in files))
            self.assertGreater(len({r["bytes"] for r in a["services"]}),4)

    def test_bounds_and_existing_output_fail_before_overwriting(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            (root/"keep").write_text("human material")
            with self.assertRaises(ValueError):q.generate(self.spec,root)
            self.assertEqual((root/"keep").read_text(),"human material")
            bad={**self.spec,"maximumServiceBytes":10}
            with self.assertRaises(ValueError):q.generate(bad,root/"small")
            for value in (0,41,True):
                with self.assertRaises(ValueError):q.generate({**self.spec,"services":value},root/str(value))


if __name__=="__main__":unittest.main()
