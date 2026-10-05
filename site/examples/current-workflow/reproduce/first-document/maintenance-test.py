#!/usr/bin/env python3
"""Future local-exercise test: --check accepts SVG, rejects other XML, calls no dot.

Usage: python3 -I -S maintenance-test.py CLONE/scripts/build_cli_documentation.py
This is synthetic focused input, not a production or final-release test result.
"""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(sys.argv.pop(1)).resolve()
SPEC = importlib.util.spec_from_file_location("walkthrough_generator", SOURCE)
generator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(generator)


class SavedSvgRootTest(unittest.TestCase):
    def exercise_check(self, saved_svg, reject):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            diagrams = root / "site/diagrams"
            diagrams.mkdir(parents=True)
            data = {"repositoryRevision": "a" * 40, "claims": [],
                    "diagram": {"nodes": [], "edges": []}}
            data_path = root / "input.json"
            data_path.write_text(json.dumps(data))
            dot, mermaid = generator.diagram_sources(data)
            (diagrams / "nav-query.dot").write_text(dot)
            (diagrams / "nav-query.mmd").write_text(mermaid)
            (diagrams / "nav-query.svg").write_text(saved_svg)
            (root / "site/nav-query.html").write_text(saved_svg)
            before = {str(p.relative_to(root)): p.read_bytes()
                      for p in root.rglob("*") if p.is_file()}
            with patch.object(generator, "ROOT", root), patch.object(generator, "DATA", data_path), \
                    patch.object(sys, "argv", [str(SOURCE), "--check"]), \
                    patch.object(generator.subprocess, "check_output",
                                 side_effect=AssertionError("check must not invoke a subprocess")), \
                    contextlib.redirect_stdout(io.StringIO()):
                if reject:
                    with self.assertRaisesRegex(AssertionError, "saved diagram is not an SVG root"):
                        generator.main()
                else:
                    generator.main()
            after = {str(p.relative_to(root)): p.read_bytes()
                     for p in root.rglob("*") if p.is_file()}
            self.assertEqual(after, before)

    def test_check_accepts_svg_without_graphviz_or_writes(self):
        self.exercise_check('<svg xmlns="http://www.w3.org/2000/svg"/>', False)

    def test_check_rejects_non_svg_xml_without_graphviz_or_writes(self):
        self.exercise_check("<not-svg/>", True)


if __name__ == "__main__":
    unittest.main()
