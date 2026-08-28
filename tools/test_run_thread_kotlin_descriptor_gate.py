import json
import os
import sys
import unittest
from pathlib import Path

sys.path.insert(0, os.fspath(Path(__file__).resolve().parent))

import run_thread_kotlin_descriptor_gate as gate
import verify_thread_kotlin_descriptor_gate as verifier


class KotlinDescriptorGateFactCategoryTest(unittest.TestCase):
    def setUp(self) -> None:
        self.context, self.session, self.side = gate.synthetic_context()
        self.expected = gate.parse_context(
            self.context,
            self.session,
            self.side,
        )

    def assert_ignored(self, matches: list[dict[str, object]]) -> None:
        context = json.loads(gate.canonical_bytes(self.context))
        context["context"]["matches"].extend(matches)
        self.assertEqual(
            gate.parse_context(context, self.session, self.side),
            self.expected,
        )

    def match(self, category: str, payload: dict[str, object]) -> dict[str, object]:
        match = json.loads(
            gate.canonical_bytes(self.context["context"]["matches"][0])
        )
        match["factKey"] = f"kotlin:{category}:{'d' * 64}"
        match["payloadRef"]["digest"] = gate.authority_digest(
            f"{category}-{payload.get('code', 'payload')}"
        )
        match["payload"] = payload
        return match

    def test_local_cfg_facts_do_not_change_descriptor_readiness(self) -> None:
        self.assert_ignored(
            [
                self.match("local-cfg", {"schema": "local-cfg/0.1"}),
                self.match(
                    "local-cfg-boundary",
                    {
                        "schema": "local-cfg-boundary/0.1",
                        "code": "INVALID_LOCAL_CFG_TOPOLOGY",
                    },
                ),
            ]
        )

    def test_open_descriptor_boundaries_do_not_qualify_a_side(self) -> None:
        located = {
            "schema": "declaration-descriptor-boundary/0.1",
            "file": "src/Sample.kt",
            "start": 0,
            "end": 10,
            "stage": "NORMALIZE",
            "code": "INVALID_JVM_DESCRIPTOR",
            "resolution": "UNKNOWN",
            "provider": "CODECLEW_DESCRIPTOR_NORMALIZER",
            "module": "root",
            "sourceSet": "main",
            "compilerAuthority": "fir-facts-extractor/0.6",
        }
        unresolved = {
            "schema": "declaration-descriptor-boundary/0.1",
            "file": "src/Sample.kt",
            "stage": "NORMALIZE",
            "code": "UNRESOLVED_DESCRIPTOR_TYPE",
            "resolution": "UNKNOWN",
            "provider": "COMPILER_DESCRIPTOR_NORMALIZER",
            "module": "root",
            "sourceSet": "main",
            "compilerAuthority": "fir-facts-extractor/0.6",
        }
        self.assert_ignored(
            [
                self.match("descriptor-boundary", located),
                self.match("descriptor-boundary", unresolved),
            ]
        )

    def test_syntax_only_boundary_remains_rejected(self) -> None:
        payload = {
            "schema": "declaration-descriptor-boundary/0.1",
            "stage": "ANALYSIS",
            "code": "SYNTAX_ONLY",
            "resolution": "UNKNOWN",
            "provider": "WORKER",
            "module": "root",
            "sourceSet": "main",
            "compilerAuthority": "fir-facts-extractor/0.6",
        }
        with self.assertRaisesRegex(gate.GateError, "SYNTAX_FALLBACK_REJECTED"):
            self.assert_ignored(
                [self.match("descriptor-boundary", payload)]
            )


class KotlinDescriptorGateR2AuthorityTest(unittest.TestCase):
    def corpus(self) -> dict[str, object]:
        return {
            "schema": gate.PRIVATE_CORPUS_SCHEMA,
            "frozenAt": gate.FROZEN_AT,
            "selectionRule": "local declared service pair",
            "services": [
                {
                    "serviceAlias": alias,
                    "serviceId": f"private-{index}",
                    "repositoryPath": f"/private/service-{index}",
                    "revision": "a" * 40,
                }
                for index, alias in enumerate(verifier.EXPECTED_SERVICES, 1)
            ],
            "tasks": [
                {
                    "taskId": task_id,
                    "pairId": pair_id,
                    "provider": provider,
                    "consumer": consumer,
                    "scenario": (
                        "PROVIDER_CONTRACT_CHANGE"
                        if index < 8
                        else (
                            "CONSUMER_REQUEST_SHAPE"
                            if index == 8
                            else "PROVIDER_RESPONSE_SHAPE"
                        )
                    ),
                }
                for index, (task_id, pair_id, (provider, consumer)) in enumerate(
                    zip(
                        verifier.EXPECTED_TASKS,
                        verifier.EXPECTED_PAIRS,
                        verifier.EXPECTED_BINDINGS,
                        strict=True,
                    )
                )
            ],
            "topologyAuthorities": [
                {
                    "repositoryPath": "/private/topology",
                    "revision": "b" * 40,
                    "relativeFile": "catalog/service-wiring.yaml",
                    "blobOid": "c" * 40,
                }
            ],
        }

    def test_r2_corpus_is_exactly_two_units_and_one_pair(self) -> None:
        corpus = gate.parse_corpus(self.corpus(), validate_paths=False)
        self.assertEqual(len(corpus.services), 2)
        self.assertEqual({task.pair_id for task in corpus.tasks}, {"pair-01"})
        self.assertEqual(len(corpus.topology_authorities), 1)

    def test_topology_authority_is_closed_and_content_bound(self) -> None:
        value = self.corpus()
        value["topologyAuthorities"][0].pop("blobOid")
        with self.assertRaises((gate.GateError, verifier.EvidenceError)):
            gate.parse_corpus(value, validate_paths=False)

    def test_public_r2_fixture_passes_closed_verifier(self) -> None:
        verifier.verify_value(verifier._valid_fixture())


if __name__ == "__main__":
    unittest.main()
