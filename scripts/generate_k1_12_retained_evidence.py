#!/usr/bin/env python3
"""One-shot canonical inventory for the superseded K1.11 production store."""

from __future__ import annotations

import hashlib
import json
import os
import stat
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
RUN = Path("/private/tmp/codeclew-k1-11-production.LULaPQ/run")
STORE = RUN / "store"
OUTPUT = ROOT / "docs/experiments/evidence/codeclew-k1.11-prepare-infrastructure-retained-evidence.json"
Q02_STDOUT = "sha256:0d5b6c1386d5e895929f3352669a9a0f8e21882f45c100ada67f6ed3e9a95d83"
Q02_STDERR = "sha256:5b14ed36343d51f76a41bef1aea4aacee3c5141d4120aa7fd189018922cacf4c"
FAILURE_DETAIL = "dependency PREPARE security/authority failure: K1-Q02"


def canonical(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()


def digest_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def digest_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return "sha256:" + digest.hexdigest()


def load(path: Path) -> Any:
    raw = path.read_bytes()
    value = json.loads(raw)
    if canonical(value) != raw:
        raise RuntimeError(f"noncanonical JSON: {path}")
    return value


def member_manifest(root: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for directory, directories, files in os.walk(root, followlinks=False):
        directory_path = Path(directory)
        for name in sorted(directories):
            path = directory_path / name
            metadata = path.lstat()
            if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
                raise RuntimeError(f"unsafe directory member: {path}")
            rows.append({
                "kind": "DIRECTORY", "mode": stat.S_IMODE(metadata.st_mode),
                "path": path.relative_to(root).as_posix(),
            })
        for name in sorted(files):
            path = directory_path / name
            metadata = path.lstat()
            if not stat.S_ISREG(metadata.st_mode):
                raise RuntimeError(f"unsafe file member: {path}")
            rows.append({
                "kind": "FILE", "mode": stat.S_IMODE(metadata.st_mode),
                "path": path.relative_to(root).as_posix(),
                "sha256": digest_file(path), "size": metadata.st_size,
            })
    rows.sort(key=lambda row: row["path"])
    return rows


def current_nodes() -> list[dict[str, str]]:
    rows = []
    for path in sorted((STORE / "current").glob("*.json")):
        pointer = load(path)
        rows.append({
            "node": pointer["node"], "pointerSha256": digest_file(path),
            "receiptDigest": pointer["receiptDigest"],
        })
    return rows


def main() -> None:
    if not STORE.is_dir() or (OUTPUT.exists() and not OUTPUT.is_file()):
        raise RuntimeError("K1.11 source store missing or output path unsafe")
    store_identity = load(STORE / "STORE.json")
    members = member_manifest(STORE)
    files = [row for row in members if row["kind"] == "FILE"]
    nodes = current_nodes()
    if any(any((STORE / name).iterdir()) for name in ("attempts", "starts", "qualification", "holdout")):
        raise RuntimeError("K1.11 semantic or child-start evidence unexpectedly present")
    if (RUN / "qualificationDependencySeed").exists() \
            or list(RUN.glob(".qualificationDependencySeed.prepare-*")):
        raise RuntimeError("K1.11 PREPARE output or staging unexpectedly present")

    stdout_path = STORE / "blobs" / f"{Q02_STDOUT[7:]}.blob"
    stderr_path = STORE / "blobs" / f"{Q02_STDERR[7:]}.blob"
    stdout = stdout_path.read_bytes()
    stderr = stderr_path.read_bytes()
    if digest_bytes(stdout) != Q02_STDOUT or digest_bytes(stderr) != Q02_STDERR \
            or b"100%" not in stdout or stdout.count(b"__SEMANTIC_THREAD_MODEL__") != 1 \
            or stdout.count(b"BUILD SUCCESSFUL") != 1 \
            or stderr.lower().count(b"tcpincomingconnector.accept") != 4 \
            or stderr.lower().count(b"net.bind0") != 2 \
            or stderr.lower().count(b"java.net.socketexception: operation not permitted") != 6:
        raise RuntimeError("K1.11 Q02 retained stdout/stderr mismatch")

    envelope = {
        "schema": "codeclew.kotlin-k1-harness-error/0.1", "status": "FAILED",
        "reason": "HarnessError", "detailSha256": digest_bytes(FAILURE_DETAIL.encode()),
    }
    envelope_raw = canonical(envelope)
    baseline_path = RUN / "baselinePacket"
    self_test_path = RUN / "harnessSelfTestPacket"
    baseline = load(baseline_path)
    self_test = load(self_test_path)
    policies = [(row["policy"], row["observed"]) for row in baseline["commands"]]
    evidence = {
        "schema": "codeclew.kotlin-k1-prepare-infrastructure-retained-evidence/0.7",
        "seriesId": "KOTLIN_REAL_REPOSITORY_K1_11_2026_08_13",
        "kind": "PREPARE_INFRASTRUCTURE_FAILURE_NO_PREPARE_RECEIPT_NO_QUALIFICATION_NO_HOLDOUT_NO_MODEL",
        "disposition": "SUPERSEDED_STORE_MUST_NOT_BE_REUSED",
        "sourceStoreAbsolutePath": str(STORE),
        "sourceStoreRootMode": stat.S_IMODE(STORE.stat().st_mode),
        "sourceStoreFileCount": len(files),
        "sourceStoreDirectoryCount": len(members) - len(files),
        "sourceStoreFileBytes": sum(row["size"] for row in files),
        "sourceStoreSymlinkCount": 0,
        "sourceStoreMemberManifest": members,
        "sourceStoreMemberManifestCount": len(members),
        "sourceStoreMemberManifestSha256": digest_bytes(canonical(members)),
        "sourceStoreTreeSha256": digest_bytes(canonical({
            "schema": "codeclew.live-tree/0.2", "members": files,
        })),
        "storeId": store_identity["storeId"],
        "storeIdentity": store_identity,
        "storeIdentitySha256": digest_file(STORE / "STORE.json"),
        "candidateToolsSha256": digest_file(RUN.parent / "candidate-tools.json"),
        "liveInputsSha256": digest_file(RUN / "live-inputs.json"),
        "candidateSourcesSha256": digest_file(RUN / "candidateSources.json"),
        "candidateBinariesSha256": digest_file(RUN / "candidateBinaries.json"),
        "trustedWorkerDistributionBuilderSha256": "sha256:6d853cfe8966dbde89caf6177b6757eb39256cecc9ec92afe8c8d6046d082030",
        "currentNodeCount": len(nodes), "currentNodes": nodes,
        "guardState": "OPEN", "guardMarkerSha256": digest_file(STORE / "guards/OPEN.json"),
        "baseline": {
            "schema": baseline["schema"], "packetSha256": digest_file(baseline_path),
            "commandCount": len(baseline["commands"]), "requiredGreen": baseline["requiredGreen"],
            "requiredGreenPassCount": policies.count(("REQUIRED_GREEN", "PASS")),
            "requiredGreenFailCount": sum(p == "REQUIRED_GREEN" and o != "PASS" for p, o in policies),
            "historicalFailCount": policies.count(("HISTORICAL_BASELINE", "FAIL")),
            "historicalPassCount": policies.count(("HISTORICAL_BASELINE", "PASS")),
        },
        "harnessSelfTest": {
            "schema": self_test["schema"], "packetSha256": digest_file(self_test_path),
            "status": self_test["status"], "counterexamples": self_test["counterexamples"],
            "requirementCaseCount": len(self_test["requirementCases"]),
            "supervisorCaseCount": len(self_test["supervisor"]["cases"]),
            "modelCalls": self_test["modelCalls"],
            "harnessSourceSha256": self_test["sourceAnchorPacket"]["sources"]["harness"],
        },
        "officialPrepare": {
            "attempts": 1, "classification": "PRODUCTION_INFRASTRUCTURE_FAILURE_NOT_SEMANTIC_OUTCOME",
            "entry": "K1-Q02", "phase": "OFFLINE_DEPENDENCY_VERIFICATION",
            "completedEntriesBeforeFailure": ["K1-Q01"],
            "officialPrepareReceiptPublished": False,
            "qualificationDependencySeedPublished": False,
            "qualificationDependencyStagingPresent": False,
            "failure": {
                "exceptionType": "HarnessError",
                "operation": "GRADLE_OFFLINE_DAEMON_LOCAL_TCP_BIND_UNDER_STRICT_NETWORK_DENY",
                "failureDetail": FAILURE_DETAIL,
                "failureDetailBytes": len(FAILURE_DETAIL.encode()),
                "failureDetailNewlineTerminated": False,
                "failureDetailSha256": digest_bytes(FAILURE_DETAIL.encode()),
                "fullOutputEnvelopeBytesRetained": True,
                "outputEnvelope": envelope,
                "outputEnvelopeAscii": envelope_raw.decode("ascii"),
                "outputEnvelopeBytes": len(envelope_raw),
                "outputEnvelopeNewlineTerminated": True,
                "outputEnvelopeSha256": digest_bytes(envelope_raw),
                "cliExitCode": 2, "semanticOutcomeObserved": False,
            },
        },
        "diagnosticInvestigation": {
            "classification": "RETAINED_BLOB_DIAGNOSIS_NO_PRODUCTION_RERUN",
            "entry": "K1-Q02", "buildDsl": "GRADLE_KOTLIN_DSL",
            "onlinePhase": {
                "combinedStdoutBytes": len(stdout), "combinedStdoutSha256": Q02_STDOUT,
                "wrapperDownloadReached100Percent": True, "semanticThreadModelCount": 1,
                "buildSuccessfulCount": 1,
            },
            "offlineFailure": {
                "combinedStderrBytes": len(stderr), "combinedStderrSha256": Q02_STDERR,
                "socketExceptionOperationNotPermittedCount": 6,
                "tcpIncomingConnectorAcceptCount": 4, "netBind0Count": 2,
                "externalConnectMarkerCount": 0, "fileLockCommunicatorMarkerCount": 0,
                "missingJvmTmpdirMarkerCount": 0,
                "strictNetworkDenyPreserved": True,
            },
            "productionStoreMutated": False, "holdoutOpened": False,
            "holdoutSourceMaterialized": False, "modelCalls": 0,
        },
        "officialDependencyPrepareAttempts": 1,
        "officialPrepareReceiptPublished": False,
        "qualificationDependencyPreparePointerPresent": False,
        "qualificationDependencySeedPublished": False,
        "qualificationDependencyStagingPresent": False,
        "qualificationAttemptCount": 0, "retainedAttemptCount": 0,
        "childStartJournalCount": 0, "holdoutAttemptCount": 0,
        "holdoutOpened": False, "holdoutSourceMaterialized": False,
        "modelCalls": 0, "decisionIssued": False,
    }
    expected = {
        "sourceStoreFileCount": 44, "sourceStoreDirectoryCount": 9,
        "sourceStoreFileBytes": 114691, "sourceStoreMemberManifestCount": 53,
        "sourceStoreMemberManifestSha256": "sha256:9a2ce6a6324f3c7840cb2106b92bca336b42823f7d5b24053dc9cb4e4727caa5",
        "sourceStoreTreeSha256": "sha256:47371fad64ecbe016373a06ba4ae903b7a4cc3526550a0498e26353f31a1f660",
        "storeId": "6dc767daf7cff3d609dc61cca6af1d087f81ba1b48d9b17572965624b3028257",
        "storeIdentitySha256": "sha256:25445690b538db93030fd88ac3b7b630cc86c126826111add0dfd6657a7a03bf",
        "candidateToolsSha256": "sha256:2d8c4c5ae7ff04e05c04e516d3e176e56a5b970c92a62481d7237f9fea4d8933",
        "liveInputsSha256": "sha256:1762041f586c4c1cf994909644e8fcb8a659e5591de7c052302af92f8aad1f19",
    }
    if any(evidence[key] != value for key, value in expected.items()) \
            or evidence["currentNodeCount"] != 8 \
            or evidence["harnessSelfTest"]["packetSha256"] != "sha256:a4dad5c75f35a6b1e776715b248239d2b0cce6b7370d40f184dda84c5a4f7a59" \
            or evidence["harnessSelfTest"]["counterexamples"] != 131 \
            or evidence["harnessSelfTest"]["requirementCaseCount"] != 64 \
            or evidence["baseline"]["packetSha256"] != "sha256:f7e44e6fa76ea90b1a3d416cc220dcd59e22d136cd19709ca94f0e48da2fe92d":
        raise RuntimeError("K1.11 retained evidence identity mismatch")
    raw = canonical(evidence)
    OUTPUT.write_bytes(raw)
    print(f"{digest_bytes(raw)}  {OUTPUT}")


if __name__ == "__main__":
    main()
