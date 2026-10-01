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
OPERATION_ANSWER_SCHEMA_PATH = ROOT / "schemas/documentation/operation-answer-1.2.schema.json"
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
OPERATION_ANSWER_SCHEMA = json.loads(OPERATION_ANSWER_SCHEMA_PATH.read_text(encoding="utf-8"))


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


def operation_answer_schema_errors(value: dict[str, object]) -> list[str]:
    class NotAwareMiniSchema(mini_schema.MiniSchema):
        """Support JSON Schema's `not` assertion in the local subset checker."""

        def validate(self, instance: object, node: dict[str, object], path: str, definitions: dict[str, object]) -> None:
            if "not" in node:
                forbidden = node["not"]
                forbidden_errors: list[str] = []
                probe = mini_schema.MiniSchema(forbidden, forbidden_errors)
                probe.validate(instance, forbidden, f"{path}.not", definitions)
                if not forbidden_errors:
                    self.errors.append(f"{path}: value matches a forbidden schema")
                node = {key: child for key, child in node.items() if key != "not"}
            super().validate(instance, node, path, definitions)

    schema = {
        key: sub_schema
        for key, sub_schema in OPERATION_ANSWER_SCHEMA.items()
        if key != "$id"
    }
    errors: list[str] = []
    validator = NotAwareMiniSchema(schema, errors)
    try:
        validator.validate(value, schema, "$", schema["$defs"])
        validator.finish()
    except mini_schema.SchemaFailure as exc:
        errors.append(str(exc))
    return errors


class DocumentationSelectionContractTest(unittest.TestCase):
    def test_operation_answer_1_2_schema_accepts_linked_glossary_and_predicates(self) -> None:
        answer = {
            "schema": "codeclew-operation-answer/1.2",
            "packetDigest": "sha256:" + "a" * 64,
            "title": "Internal transfer",
            "summary": {
                "text": "The operation prepares an order value.",
                "evidence": ["d1"],
                "glossaryRefs": ["order"],
            },
            "steps": [
                {
                    "id": "order-check",
                    "kind": "decision",
                    "predicateRef": "valid-order",
                    "glossaryRefs": ["order"],
                    "meaning": {
                        "text": "Check whether the order can be accepted.",
                        "evidence": ["d1"],
                        "glossaryRefs": ["order"],
                    },
                    "children": [
                        {
                            "id": "accept-order",
                            "kind": "return",
                            "glossaryRefs": ["order"],
                            "meaning": {
                                "text": "Return the accepted order.",
                                "evidence": ["d1"],
                                "glossaryRefs": ["order"],
                            },
                            "preparationRefs": ["shared", "partial"],
                        }
                    ],
                    "otherwise": [],
                }
            ],
            "glossary": [
                {
                    "id": "order",
                    "label": "Order",
                    "kind": "business_entity",
                    "definition": {
                        "text": "The captured source identifies this value as an order.",
                        "evidence": ["d1"],
                        "glossaryRefs": [],
                    },
                    "subjectRefs": ["type:orders.Order"],
                    "technicalNames": ["orders.Order"],
                }
            ],
            "predicates": [
                {
                    "id": "valid-order",
                    "label": "The order is valid",
                    "meaning": {
                        "text": "The source accepts an order when its identifier is present.",
                        "evidence": ["d1"],
                        "glossaryRefs": ["order"],
                    },
                    "sourceCheck": {
                        "text": "order.id != null",
                        "evidence": ["d1"],
                        "glossaryRefs": ["order"],
                    },
                    "evaluation": {
                        "text": "The comparison is false when id is null; otherwise it is true.",
                        "evidence": ["d1"],
                        "glossaryRefs": ["order"],
                    },
                }
            ],
            "preparations": [
                {
                    "id": "shared",
                    "title": "Shared mapping",
                    "subjectReference": "method:example.Mapper#map()V",
                    "summary": {
                        "text": "The mapper creates a value.",
                        "evidence": ["d1"],
                        "glossaryRefs": ["order"],
                    },
                    "steps": [
                        {
                            "id": "map-input",
                            "kind": "action",
                            "glossaryRefs": ["order"],
                            "meaning": {
                                "text": "Map the input.",
                                "evidence": ["d1"],
                                "glossaryRefs": ["order"],
                            },
                            "from": "request.input",
                            "to": "payload.value",
                        }
                    ],
                },
                {
                    "id": "partial",
                    "title": "Caller-side partial work",
                    "summary": {
                        "text": "The caller prepares a value before helper dispatch.",
                        "evidence": ["d1"],
                        "uncertainty": "The exact helper declaration is not retained.",
                        "glossaryRefs": ["order"],
                    },
                    "steps": [],
                },
            ],
            "uncertainties": [],
        }
        self.assertEqual(operation_answer_schema_errors(answer), [])

        missing_uncertainty = json.loads(json.dumps(answer))
        del missing_uncertainty["preparations"][1]["summary"]["uncertainty"]
        self.assertNotEqual(operation_answer_schema_errors(missing_uncertainty), [])

        duplicate_reference = json.loads(json.dumps(answer))
        duplicate_reference["steps"][0]["glossaryRefs"] = ["order", "order"]
        self.assertNotEqual(operation_answer_schema_errors(duplicate_reference), [])

        missing_step_id = json.loads(json.dumps(answer))
        del missing_step_id["steps"][0]["id"]
        self.assertNotEqual(operation_answer_schema_errors(missing_step_id), [])

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
            "sourceContexts": [
                {
                    "kind": "METHOD_SOURCE",
                    "authority": "SOURCE_REFERENCE_CANDIDATE",
                    "symbolIdentity": "method:orders.Helper#predicate()Z",
                    "ownerIdentity": "class:orders.Helper",
                    "scope": ":main",
                    "declarationReference": "d2",
                    "sourceReference": "s2",
                    "referencedFromSourceReference": "s1",
                    "evidence": ["d2", "s2", "s1"],
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
            "citations": {"p1": "selected process context", "d1": "root", "d2": "helper declaration", "s1": "source", "s2": "helper source", "c1": "coverage"},
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
        long_interaction_text = "x" * 2049
        process_intent = {
            "authority": "USER_INTENTION_NOT_SOURCE_EVIDENCE",
            "definitionReference": "scenario",
            "definitionDigest": "sha256:" + "d" * 64,
            "title": "Reserve an order",
            "summary": "Capture and fulfill a customer order.",
            "scope": "Internal order reservation",
            "trigger": "A customer submits an order.",
            "desiredOutcomes": ["An order is reserved."],
            "declaredContinuations": [
                {
                    "id": "reserve",
                    "reference": "i1",
                    "digest": "sha256:" + "e" * 64,
                    "authority": "DECLARED_INTERACTION_NOT_EXECUTED",
                    "definition": {
                        "schema": "codeclew-documentation-interaction/1.0",
                        "id": "reserve",
                        "title": long_interaction_text,
                        "from": {
                            "service": "orders",
                            "callSite": {"target": long_interaction_text},
                        },
                        "to": {
                            "service": "inventory",
                            "selector": {
                                "language": "java",
                                "owner": long_interaction_text,
                                "name": long_interaction_text,
                                "parameterTypes": [long_interaction_text],
                                "scope": ":main",
                            },
                        },
                        "transport": {
                            "kind": "http",
                            "method": "POST",
                            "path": "/" + long_interaction_text,
                            "destinationConfigKey": long_interaction_text,
                        },
                        "declaration": {
                            "origin": "human",
                            "rationale": long_interaction_text,
                        },
                        "applicability": {"environments": [long_interaction_text]},
                        "contractReference": long_interaction_text,
                    },
                }
            ],
            "linkedSubviews": [
                {
                    "id": "payment",
                    "authority": "USER_INTENTION_ONLY_NOT_RESOLVED_AS_A_CONTINUATION",
                }
            ],
        }
        scenario_packet = {**packet, "processIntent": process_intent}
        self.assertEqual(
            definition_errors(
                scenario_packet,
                process_packet_schema,
                READER_PACKET_SCHEMA["$defs"],
            ),
            [],
            "a saved scenario may attach typed intention with explicit non-evidence authority",
        )
        for invalid_intent in [
            {**process_intent, "authority": "SOURCE_EVIDENCE"},
            {
                **process_intent,
                "declaredContinuations": [
                    {
                        **process_intent["declaredContinuations"][0],
                        "authority": "EXECUTED",
                    }
                ],
            },
            {**process_intent, "futureHint": "not part of the contract"},
            {
                **process_intent,
                "declaredContinuations": [
                    {
                        **process_intent["declaredContinuations"][0],
                        "definition": {
                            **process_intent["declaredContinuations"][0]["definition"],
                            "futureHint": "not part of the interaction contract",
                        },
                    }
                ],
            },
        ]:
            with self.subTest(process_intent=invalid_intent):
                self.assertNotEqual(
                    definition_errors(
                        {**packet, "processIntent": invalid_intent},
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
        service_packet = {
            "schema": "codeclew-documentation-reader-packet/1.0",
            "profile": "endpoint-context-v3",
            "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
            "audience": "internal readers",
            "documentationLanguage": "en",
            "notice": "Retained evidence supports an internal draft only.",
            "title": "service:orders · endpoint:orders",
            "summary": "A retained service endpoint context.",
            "endpoint": {"symbol": None, "trigger": None, "boundaries": [], "evidence": []},
            "types": [],
            "callMap": {
                "authority": "RETAINED_TARGET_RELATIONS",
                "order": "NOT_EXECUTION_ORDER",
                "nodes": [],
                "edges": [],
            },
            "methodSources": [],
            "methodBodies": [],
            "constants": [],
            "coverage": {
                "coverage": "COMPLETE",
                "runtimeMode": "BUILD",
                "boundaries": [],
                "callAuthority": "NONE",
                "evidence": [],
            },
            "limitations": [],
            "interpretationLimits": [],
            "runtimeAndSerialization": "UNKNOWN_FROM_THIS_PACKET",
            "citations": {"p1": "selected service context"},
            "packetDigest": "sha256:" + "f" * 64,
        }
        reader_packet_schema = {
            key: value for key, value in READER_PACKET_SCHEMA.items() if key != "$id"
        }
        self.assertEqual(
            definition_errors(
                service_packet,
                reader_packet_schema,
                READER_PACKET_SCHEMA["$defs"],
            ),
            [],
        )
        self.assertNotEqual(
            definition_errors(
                {**service_packet, "processIntent": process_intent},
                READER_PACKET_SCHEMA["$defs"]["packet"],
                READER_PACKET_SCHEMA["$defs"],
            ),
            [],
            "the endpoint service packet shape remains closed and omits process intent",
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
