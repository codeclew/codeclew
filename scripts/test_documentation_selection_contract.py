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
READER_PACKET_SCHEMA_PATH = ROOT / "schemas/documentation/reader-packet.schema.json"
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
READER_PACKET_SCHEMA = json.loads(READER_PACKET_SCHEMA_PATH.read_text(encoding="utf-8"))


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


def definition_errors(
    value: dict[str, object],
    schema: dict[str, object],
    definitions: dict[str, object],
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
    def test_process_graph_work_request_requires_its_exact_root_and_question(self) -> None:
        request_schema = WORK_SCHEMA["$defs"]["request"]
        valid = {
            "schema": "codeclew-documentation-work-request/1.0",
            "audience": "internal readers",
            "contextProfile": "process-graph-v1",
            "rootDeclaration": "orders:symbol:root",
            "question": "How does this internal operation behave?",
            "entrypoint": None,
        }
        self.assertEqual(
            definition_errors(valid, request_schema, WORK_SCHEMA["$defs"]), []
        )
        self.assertEqual(
            definition_errors(
                {
                    "schema": "codeclew-documentation-work-request/1.0",
                    "audience": "internal readers",
                    "contextProfile": "endpoint-context-v3",
                    "entrypoint": "orders:endpoint:one",
                },
                request_schema,
                WORK_SCHEMA["$defs"],
            ),
            [],
            "existing endpoint requests remain admitted without process-only fields",
        )
        for invalid in [
            {**valid, "question": ""},
            {key: value for key, value in valid.items() if key != "question"},
            {**valid, "entrypoint": "orders:endpoint:one"},
            {**valid, "contextProfile": "endpoint-context-v3"},
            {
                "schema": "codeclew-documentation-work-request/1.0",
                "audience": "internal readers",
                "contextProfile": "endpoint-context-v3",
                "entrypoint": "orders:endpoint:one",
                "rootDeclaration": "orders:symbol:root",
            },
        ]:
            with self.subTest(request=invalid):
                self.assertNotEqual(
                    definition_errors(invalid, request_schema, WORK_SCHEMA["$defs"]),
                    [],
                )

    def test_process_graph_reader_packet_schema_preserves_internal_authority_shape(self) -> None:
        packet = {
            "schema": "codeclew-documentation-reader-packet/1.0",
            "profile": "process-graph-v1",
            "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
            "audience": "internal readers",
            "documentationLanguage": "en",
            "notice": "Retained evidence supports an internal draft only.",
            "title": "service:orders · method:orders.Service#run()V",
            "question": "How does this internal operation behave?",
            "context": {"authority": "DERIVED_NAVIGATION_ONLY", "evidence": ["p1"]},
            "root": {
                "methodId": "m0",
                "symbolIdentity": "method:orders.Service#run()V",
                "ownerIdentity": "class:orders.Service",
                "name": "run",
                "scope": ":main",
                "declarationReference": "d1",
                "evidence": ["d1"],
            },
            "methods": [
                {
                    "id": "m0",
                    "symbolIdentity": "method:orders.Service#run()V",
                    "ownerIdentity": "class:orders.Service",
                    "name": "run",
                    "scope": ":main",
                    "declarationReference": "d1",
                    "body": {
                        "sourceReference": "s1",
                        "startByte": 20,
                        "endByte": 40,
                        "evidence": ["s1"],
                    },
                    "evidence": ["d1", "s1"],
                }
            ],
            "edges": [
                {
                    "fromMethodId": "m0",
                    "targetMethodId": "m1",
                    "targetIdentity": "method:orders.Helper#predicate()Z",
                    "kind": "TYPE_QUALIFIED_CALL",
                    "scope": ":main",
                    "authority": "SOURCE_REFERENCE_CANDIDATE",
                    "sourceReference": "s1",
                    "evidence": ["s1"],
                }
            ],
            "fields": [],
            "types": [],
            "methodSources": [
                {
                    "reference": "s1",
                    "authority": "CAPTURED_SOURCE",
                    "text": "class Service { void run() {} }",
                    "evidence": ["d1", "s1"],
                    "contextFor": "d1",
                }
            ],
            "coverage": {
                "coverage": "COMPLETE",
                "runtimeMode": "BUILD",
                "boundaries": [],
                "callAuthority": "PROVIDER_EVIDENCE",
                "evidence": ["c1"],
            },
            "limitations": [],
            "interpretationLimits": [
                "Source candidates do not establish runtime dispatch or execution order."
            ],
            "graphAuditBinding": {
                "schema": "codeclew-documentation-process-graph/1.0",
                "artifactDigest": "sha256:" + "a" * 64,
                "rootMethodId": "m0",
                "snapshot": "sha256:" + "b" * 64 + "/0",
                "service": "orders",
            },
            "citations": {"p1": "selected process context", "d1": "root", "s1": "source", "c1": "coverage"},
            "packetDigest": "sha256:" + "c" * 64,
        }
        process_packet_schema = READER_PACKET_SCHEMA["$defs"]["processPacket"]
        self.assertEqual(
            definition_errors(
                packet,
                process_packet_schema,
                READER_PACKET_SCHEMA["$defs"],
            ),
            [],
        )
        self.assertNotEqual(
            definition_errors(
                {**packet, "endpoint": {"symbol": "invented"}},
                process_packet_schema,
                READER_PACKET_SCHEMA["$defs"],
            ),
            [],
            "the internal profile cannot acquire an HTTP endpoint envelope",
        )

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
