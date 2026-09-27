#!/usr/bin/env python3
"""Keep the delivered documentation expansion selector schema bounded."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import unittest


ROOT = Path(__file__).resolve().parent.parent
SCHEMA_PATH = ROOT / "schemas/documentation/section-author.schema.json"
WORK_SCHEMA_PATH = ROOT / "schemas/documentation/work.schema.json"
VALIDATOR_PATH = Path(__file__).with_name("validate_nessy_acceptance.py")
SPEC = importlib.util.spec_from_file_location("codeclew_mini_schema", VALIDATOR_PATH)
assert SPEC is not None and SPEC.loader is not None
mini_schema = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = mini_schema
SPEC.loader.exec_module(mini_schema)

SCHEMA = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
SELECTION_SCHEMA = SCHEMA["$defs"]["selection"]
WORK_SCHEMA = json.loads(WORK_SCHEMA_PATH.read_text(encoding="utf-8"))
WORK_SELECTION_SCHEMA = WORK_SCHEMA["$defs"]["selection"]


def selection_errors(
    value: dict[str, object],
    schema: dict[str, object] = SELECTION_SCHEMA,
    definitions: dict[str, object] = SCHEMA["$defs"],
) -> list[str]:
    errors: list[str] = []
    validator = mini_schema.MiniSchema(schema, errors)
    try:
        validator.validate(value, schema, "$", definitions)
        validator.finish()
    except mini_schema.SchemaFailure as exc:
        errors.append(str(exc))
    return errors


class DocumentationSelectionContractTest(unittest.TestCase):
    def test_default_modes_and_cursor_continuations_are_admitted(self) -> None:
        valid = [
            {},
            {"references": [], "symbols": [], "query": None},
            {"references": ["d1"]},
            {"references": ["d1"], "symbols": [], "query": None},
            {"symbols": ["helper"]},
            {"references": [], "symbols": ["helper"], "query": None},
            {"query": {"kind": "SYMBOL", "symbolContains": "helper"}},
            {"query": {"kind": "SYMBOL", "projection": "RAW"}},
            {"query": {"kind": "SYMBOL", "projection": "NAVIGATION"}},
            {"references": [], "symbols": [], "query": {"kind": "SYMBOL"}},
            {"cursor": "cursor-1"},
            {"references": ["d1"], "cursor": "cursor-1"},
            {"symbols": ["helper"], "cursor": "cursor-1"},
            {"query": {"kind": "SYMBOL"}, "cursor": "cursor-1"},
            {"query": {"kind": "SYMBOL", "projection": "NAVIGATION"}, "cursor": "cursor-1"},
        ]
        for selection in valid:
            with self.subTest(selection=selection):
                self.assertEqual(selection_errors(selection), [])

    def test_nonempty_selection_modes_cannot_be_combined(self) -> None:
        mixed = [
            {
                "references": ["d1"],
                "query": {"kind": "SYMBOL", "symbolContains": "helper"},
            },
            {"references": ["d1"], "symbols": ["helper"]},
            {"symbols": ["helper"], "query": {"kind": "SYMBOL"}},
            {"query": {"kind": "HTTP", "projection": "NAVIGATION"}},
            {"query": {"kind": "SYMBOL", "projection": "SUMMARY"}},
            {
                "references": ["d1"],
                "symbols": ["helper"],
                "query": {"kind": "SYMBOL"},
            },
        ]
        for selection in mixed:
            with self.subTest(selection=selection):
                self.assertNotEqual(selection_errors(selection), [])

    def test_public_work_projection_is_optional_raw_by_default_and_symbol_only_for_navigation(self) -> None:
        for selection in [
            {"query": {"kind": "SYMBOL"}},
            {"query": {"kind": "SYMBOL", "projection": "RAW"}},
            {"query": {"kind": "SYMBOL", "projection": "NAVIGATION"}},
        ]:
            with self.subTest(selection=selection):
                self.assertEqual(
                    selection_errors(
                        selection,
                        WORK_SELECTION_SCHEMA,
                        WORK_SCHEMA["$defs"],
                    ),
                    [],
                )
        self.assertNotEqual(
            selection_errors(
                {"query": {"kind": "HTTP", "projection": "NAVIGATION"}},
                WORK_SELECTION_SCHEMA,
                WORK_SCHEMA["$defs"],
            ),
            [],
        )

    def test_existing_bounds_and_registered_only_contract_remain(self) -> None:
        self.assertEqual(selection_errors({"references": [f"d{i}" for i in range(8)]}), [])
        self.assertEqual(selection_errors({"symbols": [f"s{i}" for i in range(8)]}), [])
        self.assertEqual(
            selection_errors({"query": {"kind": "K" * 100, "symbolContains": "x" * 512}}),
            [],
        )
        self.assertNotEqual(selection_errors({"references": [f"d{i}" for i in range(9)]}), [])
        self.assertNotEqual(selection_errors({"symbols": [f"s{i}" for i in range(9)]}), [])
        self.assertNotEqual(
            selection_errors({"query": {"kind": "K" * 101}}),
            [],
        )
        self.assertNotEqual(
            selection_errors({"query": {"kind": "SYMBOL", "symbolContains": "x" * 513}}),
            [],
        )
        self.assertEqual(selection_errors({"untrackedReads": False}), [])
        self.assertNotEqual(selection_errors({"untrackedReads": True}), [])


if __name__ == "__main__":
    unittest.main()
