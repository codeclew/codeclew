#!/usr/bin/env python3
"""One-shot, fail-closed K1.11 -> K1.12 authority identity transition."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
K1 = ROOT / "benchmarks/kotlin-real-repository/k1"
OLD_SERIES = "KOTLIN_REAL_REPOSITORY_K1_11_2026_08_13"
NEW_SERIES = "KOTLIN_REAL_REPOSITORY_K1_12_2026_08_13"
OLD_AMENDMENT = "sha256:f8bbd1296a777dff321852326deadc5600b424700ab2a97979667f17230f205e"
RETAINED = "sha256:087aaffb3cc1e5efbd4a249ac0d78324fa49435150cb9dab099145cd854a51c7"
FUNCTIONAL_HARNESS = "sha256:6e19ea72d6905757bdbcebdd792e964fb3380aa10a62a1ee71c90d2ddd6c0cb3"
FUNCTIONAL_AUDITOR = "sha256:47577f1c4e676b5fd92b08cb76ad10b747c3b6c85bff842458cae7d0d13e56aa"
OLD_AUTHORITY_DIGESTS = {
    "requirements": "sha256:38900636dd92f80cdca59104168e29047836dccace2c24c9d5cdb363343069be",
    "corpus": "sha256:bf8e6f40ca408c128eefce1c0598b435c472799a4569ce4c26012daccdbe2bfc",
    "corpusEligibilityEvidence": "sha256:c76ad7f47384983af94391ddb88269b378417dcd360f0668b58586e4744a91fa",
    "readinessGraph": "sha256:dbe64435921607350efe71e9e637af5e5ddec01d48f4c62189067b8560edef27",
    "holdoutEligibilityAudit": "sha256:d7ba2daeb629a11ed18c9e9e16d2dc90f9473a5a25cbacbdbc43aaeb42741894",
}


def canonical(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()


def digest(raw: bytes) -> str:
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def replace_once(raw: bytes, before: bytes, after: bytes, label: str) -> bytes:
    if raw.count(before) != 1:
        raise RuntimeError(f"{label}: expected one replacement, found {raw.count(before)}")
    return raw.replace(before, after)


def checked(path: Path, expected: str) -> bytes:
    raw = path.read_bytes()
    if digest(raw) != expected:
        raise RuntimeError(f"{path}: expected {expected}, got {digest(raw)}")
    return raw


def main() -> None:
    paths = {
        "requirements": K1 / "requirements.json",
        "corpus": K1 / "corpus.json",
        "corpusEligibilityEvidence": K1 / "corpus-eligibility-evidence.json",
        "readinessGraph": K1 / "readiness-graph.json",
        "holdoutEligibilityAudit": K1 / "holdout-eligibility-audit.json",
    }
    old = {name: checked(paths[name], expected) for name, expected in OLD_AUTHORITY_DIGESTS.items()}
    checked(K1 / "preregistration-amendment-k1.11.json", OLD_AMENDMENT)
    retained_raw = checked(
        ROOT / "docs/experiments/evidence/codeclew-k1.11-prepare-infrastructure-retained-evidence.json",
        RETAINED,
    )
    retained = json.loads(retained_raw)
    if canonical(retained) != retained_raw:
        raise RuntimeError("retained K1.11 evidence is not canonical")
    checked(ROOT / "scripts/k1_kotlin_real_repository.py", FUNCTIONAL_HARNESS)
    auditor_path = ROOT / "scripts/k1_independent_auditor.py"
    auditor = checked(auditor_path, FUNCTIONAL_AUDITOR)
    auditor = replace_once(auditor, b"Pinned, read-only K1.11 final-audit recomputation.",
                           b"Pinned, read-only K1.12 final-audit recomputation.", "auditor doc identity")
    auditor = replace_once(auditor, OLD_SERIES.encode(), NEW_SERIES.encode(), "auditor series identity")
    auditor_path.write_bytes(auditor)
    transition_auditor = digest(auditor)

    failure = retained["officialPrepare"]["failure"]
    diagnostic = retained["diagnosticInvestigation"]
    correction = {
        "candidateAndHarnessDisposition": "REBUILD_AND_REBIND_BEFORE_NEW_STORE",
        "functionalFreeze": {
            "preCorrectionHarness": "sha256:2dac918f53bb8d215048eb5a2d406f6d7a9e72b53b27b27d837c47f819fec9d8",
            "preCorrectionIndependentAuditor": "sha256:a3bc4ad85496136df5bfd2112ca61adc6084321e4b5013572c63f7f9dd398ed0",
            "preTransitionHarness": FUNCTIONAL_HARNESS,
            "preTransitionIndependentAuditor": FUNCTIONAL_AUDITOR,
            "postTransitionIndependentAuditor": transition_auditor,
            "selfTestCounterexamples": 132,
            "requirementCases": 64,
            "supervisorCases": 18,
            "modelCalls": 0,
            "redTeam": "ACCEPT_STRUCTURAL_PUBLICATION_BOUNDARY_NO_GUARANTEED_BLOCKER",
        },
        "gradleStrictOfflineTypedRefusalAuthority": {
            "scope": "GRADLE_OFFLINE_DEPENDENCY_VERIFICATION_ONLY",
            "typedReasonCode": "OFFLINE_MODEL_PROBE_FAILED",
            "classificationBoundary": {
                "buildDsl": ["GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"],
                "failedCommand": "SECOND_COMMAND_EXACT_OFFLINE_ARGV_WITH_ONE_OFFLINE_FLAG",
                "commandResultCount": 2,
                "exitCode": "NONZERO",
                "offlineSandboxProfile": "EXACT_ONE_DENY_NETWORK_ZERO_ALLOW_NETWORK",
                "offlineSentinel": "EXIT_ZERO_EMPTY_STDOUT_EMPTY_STDERR_BEFORE_MODEL_PROBE",
                "sourceAuthority": "FROZEN_SOURCE_BEFORE_EQUALS_AFTER_BEFORE_CLASSIFICATION",
                "stderrSemantics": "NOT_CLASSIFIED",
                "publicationGate": "FULL_PREPARATION_NETWORK_EVIDENCE_VALIDATED_BEFORE_PREPARED_REFUSAL",
            },
            "neverTypedRefusal": [
                "ONLINE_PHASE_FAILURE", "MAVEN_FAILURE", "WRONG_NETWORK_PROFILE",
                "SENTINEL_FAILURE", "SOURCE_MUTATION", "ZERO_EXIT_READY",
                "LAUNCH_FAILURE", "TIMEOUT", "OUTPUT_LIMIT", "RESIDENT_LIMIT", "SIGNAL_TERMINATION",
            ],
            "cases": [
                "prepareGradleStrictOfflineFailureTypedRefusal",
                "prepareGradleStrictOfflineWrongProfileSecurityRejected",
                "prepareGradleOnlineSecurityFailureRejected",
                "prepareMavenOfflineSecurityFailureRejected",
            ],
        },
        "preparedRefusalIdentity": {
            "schemaBefore": "codeclew.kotlin-k1-dependency-preparation-refusal/0.10",
            "schemaAfter": "codeclew.kotlin-k1-dependency-preparation-refusal/0.11",
            "seriesBefore": OLD_SERIES,
            "seriesAfter": NEW_SERIES,
        },
        "preserved": [
            "DECISION_THRESHOLDS", "REQUIREMENTS_EXCEPT_SERIES_AND_AMENDMENT_BINDING",
            "CORPUS_EXCEPT_SERIES", "ELIGIBILITY_EXCEPT_SERIES",
            "HOLDOUT_ELIGIBILITY_PROCEDURE_MEMBERS_AND_DECISION", "READINESS_GRAPH_EXCEPT_GRAPH_ID",
            "K0_1_BYTE_EXACT", "WORKLOAD", "SOURCE_TREE_AND_SANDBOX_NETWORK_OUTPUT_AUTHORITY",
            "STRICT_OFFLINE_AND_DECISION_NETWORK_DENY_WITHOUT_ALLOW",
            "BASELINE_PACKET_AND_CONTEXT_SCHEMAS", "BASELINE_LOGICAL_COMMAND_ARGV_TARGETS_AND_TEST_FILTERS",
            "CARGO_LOCK_DERIVED_SEED", "RUST_GRADLE_MAVEN_JDK_LAUNCHER_IDENTITIES",
            "BASELINE_GREEN_MEASUREMENT", "PREPARE_NETWORK_SPLIT_AND_SENTINEL",
            "PREPARE_ANCESTOR_TRAVERSAL_AUTHORITY", "PREPARE_MAVEN_RUNTIME_MINIMAL_AUTHORITY",
            "DISPOSABLE_ARCHIVE_DUAL_IDENTITY_AUTHORITY", "GRADLE_WRAPPER_BOOTSTRAP_PRIVATE_HOME",
            "GRADLE_PRIVATE_JVM_TMPDIR", "MAVEN_ONLINE_MODEL_PREFETCH",
            "DEPENDENCY_SEED_PHYSICAL_SEALING", "DISPOSABLE_SOURCE_SAFE_CLEANUP",
            "PREPARE_RESOURCE_AND_OUTPUT_CAPS", "NO_EDIT_APPLY_OR_MODEL_BENCHMARK",
            "NO_SEALED_PROJECT_MODEL_OR_NETWORK_PROFILE_WIDENING",
        ],
        "strictNetworkAuthority": {
            "offlineProfile": "EXACT_DENY_NETWORK_WITH_ZERO_ALLOW_NETWORK_CLAUSES",
            "decisionWorker": "EXACT_DENY_NETWORK_UNCHANGED",
            "profileWidening": "FORBIDDEN",
        },
        "supersededStoreDisposition": "PREPARE_INFRASTRUCTURE_FAILURE_RETAINED_EVIDENCE_STORE_MUST_NOT_BE_REUSED",
    }
    evidence = {
        "retainedEvidenceSha256": RETAINED,
        "schema": retained["schema"], "seriesId": retained["seriesId"],
        "kind": retained["kind"], "disposition": retained["disposition"],
        "sourceStoreAbsolutePath": retained["sourceStoreAbsolutePath"],
        "storeId": retained["storeId"], "storeIdentitySha256": retained["storeIdentitySha256"],
        "sourceStoreMemberManifestCount": retained["sourceStoreMemberManifestCount"],
        "sourceStoreMemberManifestSha256": retained["sourceStoreMemberManifestSha256"],
        "sourceStoreTreeSha256": retained["sourceStoreTreeSha256"],
        "guardState": retained["guardState"], "guardMarkerSha256": retained["guardMarkerSha256"],
        "currentNodeCount": retained["currentNodeCount"],
        "failure": {
            "entry": retained["officialPrepare"]["entry"],
            "phase": retained["officialPrepare"]["phase"],
            "classification": retained["officialPrepare"]["classification"],
            "operation": failure["operation"], "failureDetailSha256": failure["failureDetailSha256"],
            "fullOutputEnvelopeBytesRetained": failure["fullOutputEnvelopeBytesRetained"],
            "outputEnvelopeAscii": failure["outputEnvelopeAscii"],
            "outputEnvelopeBytes": failure["outputEnvelopeBytes"],
            "outputEnvelopeSha256": failure["outputEnvelopeSha256"],
            "cliExitCode": failure["cliExitCode"], "semanticOutcomeObserved": failure["semanticOutcomeObserved"],
            "combinedStdoutBytes": diagnostic["onlinePhase"]["combinedStdoutBytes"],
            "combinedStdoutSha256": diagnostic["onlinePhase"]["combinedStdoutSha256"],
            "combinedStderrBytes": diagnostic["offlineFailure"]["combinedStderrBytes"],
            "combinedStderrSha256": diagnostic["offlineFailure"]["combinedStderrSha256"],
        },
        "counts": {
            "officialDependencyPrepareAttempts": retained["officialDependencyPrepareAttempts"],
            "officialPrepareReceiptPublished": retained["officialPrepareReceiptPublished"],
            "qualificationDependencySeedPublished": retained["qualificationDependencySeedPublished"],
            "qualificationAttempts": retained["qualificationAttemptCount"],
            "retainedAttempts": retained["retainedAttemptCount"],
            "childStarts": retained["childStartJournalCount"],
            "holdoutAttempts": retained["holdoutAttemptCount"],
            "holdoutOpened": retained["holdoutOpened"], "modelCalls": retained["modelCalls"],
            "decisionIssued": retained["decisionIssued"],
        },
    }
    amendment = {
        "schema": "codeclew.kotlin-k1-preregistration-amendment/0.12",
        "cancelledSeriesId": OLD_SERIES, "replacementSeriesId": NEW_SERIES,
        "oldAuthorityDigests": OLD_AUTHORITY_DIGESTS,
        "predecessorAmendmentSha256": OLD_AMENDMENT,
        "reasonCode": "K1_11_GRADLE_STRICT_OFFLINE_NONZERO_MODEL_PROBE_MISCLASSIFIED_AS_SECURITY_AUTHORITY_FAILURE",
        "authorityStateBeforeReplacement": "PREPARE_INFRASTRUCTURE_FAILURE_NO_PREPARE_RECEIPT_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL",
        "baselineAttempts": 1, "officialDependencyPrepareAttempts": 1,
        "qualificationAttempts": 0, "holdoutAttempts": 0, "modelCalls": 0, "holdoutOpened": False,
        "correction": correction, "prepareInfrastructureEvidence": evidence,
    }
    amendment_raw = canonical(amendment)
    amendment_path = K1 / "preregistration-amendment-k1.12.json"
    amendment_path.write_bytes(amendment_raw)
    amendment_digest = digest(amendment_raw)

    new: dict[str, bytes] = {}
    for name in ("corpus", "corpusEligibilityEvidence", "readinessGraph"):
        new[name] = replace_once(old[name], OLD_SERIES.encode(), NEW_SERIES.encode(), f"{name} series")
    requirements = replace_once(old["requirements"], OLD_SERIES.encode(), NEW_SERIES.encode(), "requirements series")
    requirements = replace_once(requirements, OLD_AMENDMENT.encode(), amendment_digest.encode(), "requirements amendment")
    new["requirements"] = requirements
    holdout = replace_once(old["holdoutEligibilityAudit"], OLD_SERIES.encode(), NEW_SERIES.encode(), "holdout series")
    holdout = replace_once(holdout, OLD_AUTHORITY_DIGESTS["corpus"].encode(), digest(new["corpus"]).encode(), "holdout corpus")
    holdout = replace_once(
        holdout, OLD_AUTHORITY_DIGESTS["corpusEligibilityEvidence"].encode(),
        digest(new["corpusEligibilityEvidence"]).encode(), "holdout eligibility",
    )
    new["holdoutEligibilityAudit"] = holdout
    for name, raw in new.items():
        paths[name].write_bytes(raw)

    schema_path = ROOT / "schemas/kotlin_k1_prepared_refusal.schema.json"
    schema = json.loads(schema_path.read_bytes())
    if schema["$id"] != "codeclew.kotlin-k1-dependency-preparation-refusal/0.10":
        raise RuntimeError("prepared-refusal predecessor schema drift")
    schema["$id"] = "codeclew.kotlin-k1-dependency-preparation-refusal/0.11"
    schema["properties"]["schema"]["const"] = schema["$id"]
    schema["properties"]["seriesId"]["const"] = NEW_SERIES
    schema_path.write_text(json.dumps(schema, ensure_ascii=False, indent=2) + "\n")

    rust_path = ROOT / "crates/evidence-adapters/src/bin/kotlin_k1.rs"
    rust = rust_path.read_bytes()
    rust = replace_once(rust, b"codeclew.kotlin-k1-dependency-preparation-refusal/0.10",
                        b"codeclew.kotlin-k1-dependency-preparation-refusal/0.11", "Rust refusal identity")
    rust = replace_once(rust, OLD_SERIES.encode(), NEW_SERIES.encode(), "Rust series identity")
    rust = replace_once(rust, b"prepared_refusal_rejects_k1_10_predecessor_identity",
                        b"prepared_refusal_rejects_k1_11_predecessor_identity", "Rust test name")
    rust = replace_once(rust, b"KOTLIN_REAL_REPOSITORY_K1_10_2026_08_13",
                        OLD_SERIES.encode(), "Rust predecessor series")
    rust = replace_once(rust, b"codeclew.kotlin-k1-dependency-preparation-refusal/0.9",
                        b"codeclew.kotlin-k1-dependency-preparation-refusal/0.10", "Rust predecessor schema")
    rust_path.write_bytes(rust)

    print(json.dumps({
        "amendment": amendment_digest,
        "authorities": {name: digest(raw) for name, raw in new.items()},
        "auditor": transition_auditor,
        "preparedRefusalSchema": digest(schema_path.read_bytes()),
        "rust": digest(rust_path.read_bytes()),
    }, sort_keys=True))


if __name__ == "__main__":
    main()
