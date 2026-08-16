#!/usr/bin/env python3
"""One-shot canonical inventory for the superseded K1.10 production store."""

from __future__ import annotations

import hashlib
import json
import os
import stat
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
RUN = Path("/private/tmp/codeclew-k1-10-production.p3wn4I/run")
STORE = RUN / "store"
SEED = RUN / "qualificationDependencySeed"
OUTPUT = ROOT / "docs/experiments/evidence/codeclew-k1.10-qualification-infrastructure-retained-evidence.json"


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
                "kind": "DIRECTORY",
                "mode": stat.S_IMODE(metadata.st_mode),
                "path": path.relative_to(root).as_posix(),
            })
        for name in sorted(files):
            path = directory_path / name
            metadata = path.lstat()
            if not stat.S_ISREG(metadata.st_mode):
                raise RuntimeError(f"unsafe file member: {path}")
            rows.append({
                "kind": "FILE",
                "mode": stat.S_IMODE(metadata.st_mode),
                "path": path.relative_to(root).as_posix(),
                "sha256": digest_file(path),
                "size": metadata.st_size,
            })
    rows.sort(key=lambda row: row["path"])
    return rows


def pointer_rows() -> list[dict[str, str]]:
    rows = []
    for path in sorted((STORE / "current").glob("*.json")):
        pointer = load(path)
        rows.append({
            "node": pointer["node"],
            "pointerSha256": digest_file(path),
            "receiptDigest": pointer["receiptDigest"],
        })
    return rows


def refusal_rows(manifest: dict[str, Any]) -> list[dict[str, Any]]:
    rows = []
    for entry in manifest["entries"]:
        if entry["outcome"] != "TYPED_REFUSAL":
            continue
        entry_id = entry["entry"]
        path = SEED / "entries" / entry_id / "PREPARED_REFUSAL.json"
        refusal = load(path)
        refusal_projection = dict(refusal)
        refusal_projection["objectDigest"] = ""
        object_digest = digest_bytes(canonical(refusal_projection).removesuffix(b"\n"))
        if refusal.get("objectDigest") != object_digest \
                or refusal.get("preparationReceiptDigest") != digest_bytes(canonical(entry)):
            raise RuntimeError(f"K1.10 prepared refusal self-seal mismatch: {entry_id}")
        rows.append({
            "commonCauseCode": "GRADLE_PREPARE_JVM_TMPDIR_INHERITED_NONEXISTENT_HOST_PATH",
            "entry": entry_id,
            "fileSha256": digest_file(path),
            "fileSize": path.stat().st_size,
            "object": refusal,
            "prepareRow": entry,
            "retainedClassification": "INFRASTRUCTURE_MISCLASSIFIED_NOT_PRODUCT_SEMANTIC_REFUSAL_MUST_NOT_REUSE",
        })
    return rows


def main() -> None:
    if not STORE.is_dir() or not SEED.is_dir() or (OUTPUT.exists() and not OUTPUT.is_file()):
        raise RuntimeError("K1.10 source authorities missing or output path is unsafe")
    store_identity = load(STORE / "STORE.json")
    cohort_path = SEED / "CODECLEW_K1_DEPENDENCY_COHORT.json"
    marker_path = SEED / "CODECLEW_K1_DEPENDENCY_COHORT"
    cohort_raw = cohort_path.read_bytes()
    cohort = load(cohort_path)
    marker_raw = marker_path.read_bytes()
    expected_marker = (digest_bytes(cohort_raw) + "\n").encode()
    if marker_raw != expected_marker:
        raise RuntimeError("K1.10 dependency cohort marker mismatch")
    store_members = member_manifest(STORE)
    seed_members = member_manifest(SEED)
    store_tree_sha256 = digest_bytes(canonical({
        "schema": "codeclew.live-tree/0.2",
        "members": [row for row in store_members if row["kind"] == "FILE"],
    }))
    seed_tree_sha256 = digest_bytes(canonical({
        "schema": "codeclew.live-tree/0.2",
        "members": [row for row in seed_members if row["kind"] == "FILE"],
    }))
    current_nodes = pointer_rows()
    if any(any((STORE / name).iterdir()) for name in ("attempts", "starts", "qualification", "holdout")):
        raise RuntimeError("K1.10 semantic or child-start evidence unexpectedly present")
    baseline_path = RUN / "baselinePacket"
    self_test_path = RUN / "harnessSelfTestPacket"
    baseline = load(baseline_path)
    self_test = load(self_test_path)
    policies = [(row["policy"], row["observed"]) for row in baseline["commands"]]
    stderr_evidence = []
    for entry in cohort["entries"]:
        if entry["entry"] == "K1-Q01":
            continue
        stderr_digest = entry["stderrSha256"]
        stdout_digest = entry["stdoutSha256"]
        stderr_path = STORE / "blobs" / f"{stderr_digest[7:]}.blob"
        stdout_path = STORE / "blobs" / f"{stdout_digest[7:]}.blob"
        stderr = stderr_path.read_bytes()
        stdout = stdout_path.read_bytes()
        marker = b"java.io.tmpdir is set to a directory that doesn't exist: /var/folders/l0/2_7b6j512jjgg1_qwx96frd40000gp/T"
        if marker not in stderr or b"100%" not in stdout:
            raise RuntimeError(f"K1.10 common Gradle infrastructure evidence mismatch: {entry['entry']}")
        stderr_evidence.append({
            "entry": entry["entry"],
            "stderrBytes": len(stderr),
            "stderrSha256": stderr_digest,
            "stdoutBytes": len(stdout),
            "stdoutSha256": stdout_digest,
            "wrapperDownloadReached100Percent": True,
            "missingJvmTmpdirExcerptPresent": True,
        })
    evidence = {
        "baseline": {
            "commandCount": len(baseline["commands"]),
            "executionContextId": baseline["executionContextId"],
            "historicalFailCount": policies.count(("HISTORICAL_BASELINE", "FAIL")),
            "historicalPassCount": policies.count(("HISTORICAL_BASELINE", "PASS")),
            "packetSha256": digest_file(baseline_path),
            "repositoryHeadAfter": baseline["repositoryHeadAfter"],
            "repositoryHeadBefore": baseline["repositoryHeadBefore"],
            "requiredGreen": baseline["requiredGreen"],
            "requiredGreenFailCount": sum(
                policy == "REQUIRED_GREEN" and observed != "PASS" for policy, observed in policies
            ),
            "requiredGreenPassCount": policies.count(("REQUIRED_GREEN", "PASS")),
            "schema": baseline["schema"],
        },
        "candidateBinariesSha256": digest_file(RUN / "candidateBinaries.json"),
        "candidateSourcesSha256": digest_file(RUN / "candidateSources.json"),
        "candidateToolsSha256": digest_file(RUN.parent / "candidate-tools.json"),
        "childStartJournalCount": 0,
        "currentNodeCount": len(current_nodes),
        "currentNodes": current_nodes,
        "decisionIssued": False,
        "diagnosticInvestigation": {
            "causes": [
                {
                    "code": "CORPUS_RUNNER_LOCAL_NAME_SHADOWED_GLOBAL_SNAPSHOT_INPUT",
                    "entry": "K1-Q01",
                    "invocation": "COLD",
                    "observation": "UNBOUNDLOCALERROR_BEFORE_FIRST_CHILD_START_OR_ATTEMPT_PUBLICATION",
                },
                {
                    "code": "GRADLE_PREPARE_JVM_TMPDIR_INHERITED_NONEXISTENT_HOST_PATH",
                    "entries": [f"K1-Q{number:02d}" for number in range(2, 7)],
                    "observation": "WRAPPER_DOWNLOAD_100_PERCENT_THEN_GRADLE_JVM_REJECTED_NONEXISTENT_JAVA_IO_TMPDIR",
                },
            ],
            "classification": "POST_FAILURE_STATIC_AND_RETAINED_BLOB_DIAGNOSIS_NO_PRODUCTION_RERUN",
            "diagnosticAlternatives": [
                {
                    "classification": "REJECTED_UNSAFE_MUST_NOT_USE",
                    "code": "ALLOW_LOCALHOST_INBOUND_OUTBOUND_LOOPBACK_PROFILE",
                    "reason": "LOCALHOST_SELECTOR_ALLOWED_WILDCARD_BIND_AND_UNSANDBOXED_LAN_INBOUND",
                },
                {
                    "classification": "REJECTED_REQUIREMENT_WIDENING_MUST_NOT_USE",
                    "code": "GRADLE_OFFLINE_LOCAL_UDP_BIND_ONLY_EXCEPTION",
                    "reason": "K1_R13_STRICT_DECISION_NETWORK_DENIAL_AND_BOUNDED_K1_11_CORRECTION_PRESERVED",
                },
            ],
            "holdoutOpened": False,
            "holdoutSourceMaterialized": False,
            "modelCalls": 0,
            "productionStoreMutated": False,
            "qualificationOnly": True,
        },
        "disposition": "SUPERSEDED_STORE_AND_PUBLISHED_COHORT_MUST_NOT_BE_REUSED",
        "guardMarkerSha256": digest_file(STORE / "guards/OPEN.json"),
        "guardState": "OPEN",
        "harnessSelfTest": {
            "counterexamples": self_test["counterexamples"],
            "packetSha256": digest_file(self_test_path),
            "schema": self_test["schema"],
            "status": self_test["status"],
            "supervisorCaseCount": len(self_test["supervisor"]["cases"]),
        },
        "harnessSourceSha256": self_test["sourceAnchorPacket"]["sources"]["harness"],
        "holdoutAttemptCount": 0,
        "holdoutOpened": False,
        "holdoutSourceMaterialized": False,
        "kind": "PUBLISHED_PREPARE_COHORT_AND_PRE_CHILD_QUALIFICATION_INFRASTRUCTURE_FAILURE_NO_SEMANTIC_OUTCOME",
        "liveInputsSha256": digest_file(RUN / "live-inputs.json"),
        "modelCalls": 0,
        "officialDependencyPrepareAttempts": 1,
        "officialPrepareReceiptPublished": True,
        "officialQualificationRunner": {
            "classification": "PRE_CHILD_HARNESS_INFRASTRUCTURE_FAILURE_NOT_SEMANTIC_OUTCOME",
            "cliInvocations": 1,
            "entry": "K1-Q01",
            "failure": {
                "cliExitCode": 1,
                "exceptionType": "UnboundLocalError",
                "fullTracebackBytesRetained": False,
                "harnessJsonEnvelopeEmitted": False,
                "operation": "CORPUS_RUNNER_INITIAL_DEPENDENCY_SEED_SNAPSHOT",
                "semanticOutcomeObserved": False,
                "stderrFinalLine": "UnboundLocalError: cannot access local variable 'snapshot_input' where it is not associated with a value",
                "stderrFinalLineCanonicalDetailBytes": 104,
                "stderrFinalLineCanonicalDetailSha256": "sha256:3e887d45362a9c4e92574823b8ccf63afeba3b1f1331979ebda10889ca743ce5",
                "sourceStderrFinalLineTerminationRetained": False,
                "tracebackStream": "STDERR",
            },
            "invocation": "COLD",
            "phase": "BEFORE_CHILD_START_JOURNAL_AND_ATTEMPT_PUBLICATION",
            "retries": 0,
        },
        "publishedDependencyCohort": {
            "cohortDigest": cohort["cohortDigest"],
            "fileBytes": sum(row.get("size", 0) for row in seed_members if row["kind"] == "FILE"),
            "fileCount": sum(row["kind"] == "FILE" for row in seed_members),
            "directoryCount": sum(row["kind"] == "DIRECTORY" for row in seed_members),
            "manifest": cohort,
            "manifestBytes": len(cohort_raw),
            "manifestSha256": digest_bytes(cohort_raw),
            "markerAscii": marker_raw.decode("ascii"),
            "markerBytes": len(marker_raw),
            "markerSha256": digest_bytes(marker_raw),
            "memberManifest": seed_members,
            "memberManifestCount": len(seed_members),
            "memberManifestSha256": digest_bytes(canonical(seed_members)),
            "misclassifiedInfrastructureRefusals": refusal_rows(cohort),
            "rootMode": stat.S_IMODE(SEED.stat().st_mode),
            "sealedModes": {"directories": 0o500, "files": 0o400},
            "sealedTreeValidated": True,
            "symlinkCount": 0,
            "sourceAbsolutePath": str(SEED),
            "treeSha256": seed_tree_sha256,
        },
        "qualificationAttemptCount": 0,
        "qualificationDependencyPreparePointerPresent": True,
        "qualificationDependencySeedPublished": True,
        "qualificationDependencySeedVerifyPointerPresent": True,
        "qualificationRunnerInvocationCount": 1,
        "retainedAttemptCount": 0,
        "schema": "codeclew.kotlin-k1-qualification-infrastructure-retained-evidence/0.1",
        "seriesId": "KOTLIN_REAL_REPOSITORY_K1_10_2026_08_13",
        "sourceStoreAbsolutePath": str(STORE),
        "sourceStoreDirectoryCount": sum(row["kind"] == "DIRECTORY" for row in store_members),
        "sourceStoreFileBytes": sum(
            row.get("size", 0) for row in store_members if row["kind"] == "FILE"
        ),
        "sourceStoreFileCount": sum(row["kind"] == "FILE" for row in store_members),
        "sourceStoreMemberManifest": store_members,
        "sourceStoreMemberManifestCount": len(store_members),
        "sourceStoreMemberManifestSha256": digest_bytes(canonical(store_members)),
        "sourceStoreRootMode": stat.S_IMODE(STORE.stat().st_mode),
        "sourceStoreTreeSha256": store_tree_sha256,
        "sourceStoreSymlinkCount": 0,
        "storeId": store_identity["storeId"],
        "storeIdentity": store_identity,
        "storeIdentitySha256": digest_file(STORE / "STORE.json"),
        "trustedWorkerDistributionBuilderSha256": "sha256:6d853cfe8966dbde89caf6177b6757eb39256cecc9ec92afe8c8d6046d082030",
    }
    if evidence["sourceStoreFileCount"] != 56 or evidence["sourceStoreDirectoryCount"] != 9:
        raise RuntimeError("K1.10 store member count mismatch")
    if evidence["candidateToolsSha256"] \
        != "sha256:3cbd96aaf2271e7b9c7f33cb716aa46a86ea8292a4f10cefe42ea51194c3066d" \
        or evidence["storeId"] \
        != "1a620cbf192e0c109df738a78e56ec34b45602625adf1109e9811bca55ea626c" \
        or evidence["storeIdentitySha256"] \
        != "sha256:6bcfbacdec359070602e514be5c1e2a73c551c94684b5cbab8c5c2e892ade1de" \
        or evidence["sourceStoreMemberManifestSha256"] \
        != "sha256:bf227f08f797c6137627c546224a9c09ae8792303ef6a0481af7ec68dcf11759" \
        or evidence["sourceStoreFileBytes"] != 2_009_696 \
        or store_tree_sha256 \
        != "sha256:74a297a2732b4888a8a453d7885527dded875c29bbbf1d763a39958da6676fd1" \
        or seed_tree_sha256 \
        != "sha256:e46c43573024e32586729cd8e0fb26edbb71d1d987a32adc3f84e791bca3be77":
        raise RuntimeError("K1.10 candidate/store/cohort identity mismatch")
    if evidence["publishedDependencyCohort"]["fileCount"] != 5617 \
        or evidence["publishedDependencyCohort"]["directoryCount"] != 2320 \
        or evidence["publishedDependencyCohort"]["fileBytes"] != 789_540_505 \
        or evidence["publishedDependencyCohort"]["manifestSha256"] \
        != "sha256:5b8dff76560bca785181dbce13bcbace74a0057021a9213b028c333b0bedaa7f" \
        or evidence["publishedDependencyCohort"]["markerSha256"] \
        != "sha256:e74801c43c4e8f512d8fb93aa03913727199b472651c1e59fddde3208f5d9320" \
        or evidence["publishedDependencyCohort"]["memberManifestSha256"] \
        != "sha256:35b9dd03328c107540ff7350129f3f95d690a5fcf20a5e5a7ced474d3f32b28e" \
        or evidence["publishedDependencyCohort"]["cohortDigest"] \
        != "sha256:461a82b26b3f38ca1b843993dd56016f0e70011af8abdea0a7b876e7274c662b":
        raise RuntimeError("K1.10 dependency cohort member count mismatch")
    if evidence["baseline"]["packetSha256"] \
        != "sha256:4ac34c6dbc60658af9790b0e884ece5ac06ccfa3ce566c90a58cddac6856037d" \
        or evidence["harnessSelfTest"]["packetSha256"] \
        != "sha256:2ecb9465846257799d87530a7b5cc9639c4dcda74f9d88c25aa7522a3c3ceffd" \
        or evidence["harnessSelfTest"]["counterexamples"] != 118 \
        or evidence["harnessSelfTest"]["supervisorCaseCount"] != 18:
        raise RuntimeError("K1.10 baseline or harness self-test identity mismatch")
    raw = canonical(evidence)
    OUTPUT.write_bytes(raw)
    print(f"{digest_bytes(raw)}  {OUTPUT}")


if __name__ == "__main__":
    main()
