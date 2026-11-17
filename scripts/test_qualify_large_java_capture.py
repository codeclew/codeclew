#!/usr/bin/env python3
"""Owned fixture and qualification-integrity controls; no compiler capture."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec=importlib.util.spec_from_file_location("large_capture",Path(__file__).with_name("qualify_large_java_capture.py"))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class QualificationControls(unittest.TestCase):
    def test_fixture_reproducibility_and_frozen_membership(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            first=module.generate(root/"one",2,3,3)
            second=module.generate(root/"two",2,3,3)
            self.assertEqual(first,second)
            self.assertTrue(first["predictionIsNotProducerMeasurement"])
            for name,digest in first["files"].items():
                self.assertEqual(module.file_sha256(root/"one/repository"/name),digest)
            self.assertEqual(len(first["expectedSourceFiles"]),3)
            with self.assertRaises(ValueError):module.generate(root/"one",2,3,3)

    def test_selected_generation_inventory_rejects_missing_duplicate_reordered_and_wrong_totals(self):
        selected=[":a/main",":b/main",":c/main"]
        def events(ordinals,total=3):
            return [{"phase":"COMPILATION_GENERATION","event":"COMPLETED","counts":{"compilationOrdinal":n,"compilationsTotal":total}} for n in ordinals]
        self.assertTrue(module.progress_inventory(events([1,2,3]),selected)["allSelectedGenerationsCompleted"])
        for rows in (events([1,2]),events([1,2,2]),events([3,2,1]),events([1,2,3],4)):
            self.assertFalse(module.progress_inventory(rows,selected)["allSelectedGenerationsCompleted"])

    def test_semantic_fixture_preserves_excluded_invalid_module_and_exact_expectations(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)/"semantic";fixture=module.generate_semantic(root)
            expected=json.loads((root/"expected.json").read_text())
            service=json.loads((root/"service.json").read_text())
            self.assertEqual(expected["revision"],fixture["revision"])
            self.assertNotIn(":unselected/main",service["compilations"])
            self.assertIn("deliberately invalid",(root/"repository/unselected/src/main/java/owned/NotSelected.java").read_text())
            self.assertEqual(expected["selectedCompilations"],service["compilations"])
            for name,fragment in expected["files"].items():
                self.assertEqual(fragment["source"],(root/"repository"/name).read_text())
                self.assertEqual(fragment["sha256"],module.file_sha256(root/"repository"/name))

if __name__=="__main__":unittest.main()
