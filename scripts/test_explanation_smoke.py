#!/usr/bin/env python3
"""Fast unit checks for the explanation smoke's exact claim binding."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("explanation-smoke.py")
SPEC = importlib.util.spec_from_file_location("explanation_smoke", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


def template() -> dict[str, object]:
    return {
        "schema": "codeclew-explanation-claim-template/0.1",
        "locale": "en",
        "rootContains": "ProductService.saveProduct",
        "claims": [
            {
                "kind": "CALL_EXISTS",
                "localId": "outbox-save",
                "targetContains": "OutboxRepository.save",
                "text": "outbox call",
            },
            {
                "boundaryCode": "VERIFY_CONTROL_FLOW_ORDER",
                "kind": "NARRATIVE_SUMMARY",
                "localId": "atomicity",
                "text": "atomicity unknown",
            },
        ],
    }


def flow() -> dict[str, object]:
    root = "callable:example/product/ProductService.saveProduct#jvm:()V"
    target = "callable:example/product/OutboxRepository.save#jvm:()V"
    return {
        "flowId": "flow:test",
        "nodes": [
            {"nodeId": "root", "symbolIdentity": root},
            {"nodeId": "target", "symbolIdentity": target},
        ],
        "edges": [
            {
                "edgeId": "edge",
                "sourceNodeId": "root",
                "targetNodeId": "target",
                "relationKind": "CALLS",
            }
        ],
        "boundaries": [
            {
                "boundaryId": "cfg-boundary",
                "code": "VERIFY_CONTROL_FLOW_ORDER",
            }
        ],
    }


class ClaimBindingTests(unittest.TestCase):
    def test_binds_exact_edge_and_negative_boundary(self) -> None:
        document = SMOKE.bind_claims(template(), flow())
        self.assertEqual(document["schema"], "codeclew-explanation-claim-input/0.1")
        call, atomicity = document["claims"]
        self.assertEqual(call["supportRefs"], ["edge"])
        self.assertEqual(call["predicate"]["object"], flow()["nodes"][1]["symbolIdentity"])
        self.assertEqual(atomicity["supportRefs"], [])
        self.assertEqual(atomicity["boundaryRefs"], ["cfg-boundary"])

    def test_ambiguous_selector_fails_closed(self) -> None:
        ambiguous = flow()
        ambiguous["edges"].append(dict(ambiguous["edges"][0], edgeId="edge-two"))
        with self.assertRaises(AssertionError):
            SMOKE.bind_claims(template(), ambiguous)


if __name__ == "__main__":
    unittest.main()
