#!/usr/bin/env python3
"""Frozen three-arm E04 binder-only experiment controller (stdlib only).

`run` never reads controller manifests. `judge` is the separate phase that
does, after every model run has been retained.
"""

from __future__ import annotations

import argparse
import contextlib
import errno
import hashlib
import io
import json
import os
import re
import signal
import shlex
import shutil
import stat
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import xml.etree.ElementTree as ET
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any

import e04_readiness as readiness

BASE = "a6ae1e48359eccef15060c1bb249a648857f30c9"
POP_SHA = "a209f115b0a175bb74859b0539f75932cd664a495332ccf10b634b3cf1c2b9f2"
MODEL = "gpt-5.6-terra"
MODEL_TIMEOUT_SECONDS = 600
EFFORT = "low"
CODEX_VERSION = "codex-cli 0.147.0"
AST_VERSION = "ast-index 3.48.1"
BINDER_TREE_SHA = "fc349a728c92750e7eb36c39368ef693d708c98badccf4eb9c0a246279474ba4"
ARMS = ("default", "ast-index", "codeclew")
PUBLIC_KEYS = {
    "schema", "taskId", "buildSystem", "kotlinVersion", "task", "repository",
    "sourceSnapshotSha256", "buildCommand", "controllerManifestCommitment",
}
FAMILIES = (
    "producer-transform-consumer", "type-signature-propagation",
    "dto-event-api-evolution", "persistence-nullability",
    "configuration-lifecycle", "error-retry-resource",
    "test-regression-strengthening",
)
ROLES = (
    "CONTEXT_PRODUCER", "TRANSFORMER", "VALUE_EDGE", "DECLARATION", "OVERRIDE",
    "CALL_SITE", "CONTRACT_FIELD", "CONSTRUCTION_SITE", "COMPATIBILITY_POLICY",
    "PROJECTION", "DECLARED_TYPE", "QUERY_CONSUMER", "CONFIGURATION_PRODUCER",
    "INITIALIZATION_SITE", "LIFECYCLE_OWNER", "FAILURE_PATH", "RESOURCE_OWNER",
    "RETRY_OPERATION", "BEHAVIOR_UNDER_TEST", "INDEPENDENT_ORACLE",
    "PRODUCTION_CONTRACT",
)
REFUSALS = (
    "UNSUPPORTED_FAMILY", "MULTIPLE_COMPATIBLE_BINDINGS",
    "UNKNOWN_EFFECT_OR_LIFECYCLE", "EXTERNAL_POLICY_ABSENT",
    "SCHEMA_EVIDENCE_ABSENT", "QUERY_UNSUPPORTED",
    "UNRESOLVED_FRAMEWORK_BOUNDARY", "EXTERNAL_RETRY_POLICY_ABSENT",
    "UNKNOWN_SUSPEND_OR_EFFECT", "BUSINESS_ORACLE_ABSENT",
    "SELF_CONFIRMING_ORACLE", "INCOMPLETE_SEMANTIC_EVIDENCE", "PARTIAL_BUDGET",
)
R7_MAX_TURNS = 1
R7_MAX_ACTION_CALLS = 8
R7_MAX_CONTEXT_BYTES = 32 * 1024
R7_MAX_GOAL_BYTES = 1024
R7_CANARY_MAX_NONCACHED_TOKENS = 45_000
R7_CANARY_MAX_ACTION_CALLS = 12
FAMILY_CONTRACTS = {
    "producer-transform-consumer": {
        "roles": ["CONTEXT_PRODUCER", "TRANSFORMER", "VALUE_EDGE"],
        "obligations": ["bind producer and consumer", "preserve value-flow contract", "prove transform placement"],
    },
    "type-signature-propagation": {
        "roles": ["DECLARATION", "OVERRIDE", "CALL_SITE"],
        "obligations": ["propagate declared type", "preserve override compatibility", "preserve call-site assignability"],
    },
    "dto-event-api-evolution": {
        "roles": ["CONTRACT_FIELD", "CONSTRUCTION_SITE", "COMPATIBILITY_POLICY"],
        "obligations": ["update contract field", "propagate construction sites", "preserve compatibility policy"],
    },
    "persistence-nullability": {
        "roles": ["PROJECTION", "DECLARED_TYPE", "QUERY_CONSUMER"],
        "obligations": ["align projection and declared type", "preserve nullability", "link query result to consumer"],
    },
    "configuration-lifecycle": {
        "roles": ["CONFIGURATION_PRODUCER", "INITIALIZATION_SITE", "LIFECYCLE_OWNER"],
        "obligations": ["bind configuration producer", "preserve initialization order", "respect lifecycle region"],
    },
    "error-retry-resource": {
        "roles": ["FAILURE_PATH", "RESOURCE_OWNER", "RETRY_OPERATION"],
        "obligations": ["preserve failure path", "preserve resource closure", "preserve retry cardinality and order"],
    },
    "test-regression-strengthening": {
        "roles": ["BEHAVIOR_UNDER_TEST", "INDEPENDENT_ORACLE", "PRODUCTION_CONTRACT"],
        "obligations": ["identify independent oracle", "detect omitted behavior", "preserve production contract"],
    },
}
ROLE_SEMANTICS = {
    "CONTEXT_PRODUCER": "callable that produces the context value exactly once",
    "TRANSFORMER": "callable that maps one value plus the bound context to one value",
    "VALUE_EDGE": "workflow callable whose parameter-to-result value edge is changed",
    "DECLARATION": "declared interface or base callable whose type changes",
    "OVERRIDE": "concrete overriding callable that must remain compatible",
    "CALL_SITE": "caller whose assigned or returned type must remain compatible",
    "CONTRACT_FIELD": "externally meaningful DTO/event/API field being evolved",
    "CONSTRUCTION_SITE": "callable that constructs the contract value",
    "COMPATIBILITY_POLICY": "repository symbol that supplies the selected compatibility/default policy",
    "PROJECTION": "declared persistence projection field; the query producer is evidence, not this role",
    "DECLARED_TYPE": "the declaration carrying the projection type obligation; it may equal PROJECTION",
    "QUERY_CONSUMER": "callable that consumes the query result",
    "CONFIGURATION_PRODUCER": "callable that produces the selected configuration value",
    "INITIALIZATION_SITE": "method where the configuration is installed",
    "LIFECYCLE_OWNER": "class or object that owns the initialized state",
    "FAILURE_PATH": "workflow callable whose failure/retry path changes",
    "RESOURCE_OWNER": "closing callable (<type>.close), not merely the resource class",
    "RETRY_OPERATION": "callable that owns retry invocation/order",
    "BEHAVIOR_UNDER_TEST": "production callable whose omission must be detected",
    "INDEPENDENT_ORACLE": "symbol providing expected behavior without calling BEHAVIOR_UNDER_TEST",
    "PRODUCTION_CONTRACT": "production behavior callable protected by the regression; it may equal BEHAVIOR_UNDER_TEST",
}
ORACLE_SEMANTICS = {
    "DERIVED": "expected behavior is mechanically derivable from independent repository contracts/tests",
    "PARAMETRIC": "repository fixes the oracle shape but explicit business values remain parameters",
    "MODEL_AUTHORED": "the task explicitly delegates an otherwise absent expected value to the model",
    "EXTERNAL_SPEC": "the public task text itself explicitly states the required behavior independently of current source",
}
REFUSAL_SEMANTICS = {
    "UNSUPPORTED_FAMILY": "the requested family is outside the arm's declared product capability",
    "MULTIPLE_COMPATIBLE_BINDINGS": "more than eight or non-enumerable compatible sets remain; use AMBIGUOUS for two to eight enumerable sets",
    "UNKNOWN_EFFECT_OR_LIFECYCLE": "symbols resolve, but effect/lifecycle preservation cannot be proved",
    "EXTERNAL_POLICY_ABSENT": "a required ABI, serialization, compatibility, or configuration policy is absent",
    "SCHEMA_EVIDENCE_ABSENT": "the query form is supported but required database schema facts are absent",
    "QUERY_UNSUPPORTED": "the query language/source cannot be parsed or linked to Kotlin symbols",
    "UNRESOLVED_FRAMEWORK_BOUNDARY": "reflection or dependency-injection ownership cannot be statically resolved",
    "EXTERNAL_RETRY_POLICY_ABSENT": "retry count/backoff/order is externally owned and absent",
    "UNKNOWN_SUSPEND_OR_EFFECT": "suspend or effect behavior crosses an unresolved boundary",
    "BUSINESS_ORACLE_ABSENT": "no independent expected business value exists in task or repository",
    "SELF_CONFIRMING_ORACLE": "the only oracle calls the production behavior it is meant to check",
    "INCOMPLETE_SEMANTIC_EVIDENCE": "generic fallback only after every more-specific boundary above is ruled out",
    "PARTIAL_BUDGET": "required semantic closure exceeds the declared bounded evidence budget",
}
STATE_DIRECTORIES = {".git", ".e04-state", ".semantic-thread", ".gradle", "build", "target"}
CLEW_ENV_ALLOWLIST = {
    "PATH", "HOME", "JAVA_HOME", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE",
    "SHELL", "TERM", "RUST_BACKTRACE", "SSL_CERT_FILE", "SSL_CERT_DIR", "CODEX_HOME",
}
CLEW_ENV_DENY = {
    "GRADLE_OPTS", "GRADLE_USER_HOME", "JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS",
    "_JAVA_OPTIONS", "MAVEN_OPTS", "MAVEN_ARGS", "MAVEN_CONFIG",
}
RUNS_LOCK = threading.Lock()
PREFLIGHT_TIMEBOX_CONTEXT: dict[str, Any] | None = None

ROOT = Path(__file__).resolve().parents[1]
POPULATION = ROOT / "benchmarks/semantic-change/editing-population-v1.json"
OUTPUT_SCHEMA = ROOT / "benchmarks/semantic-change/e04-model-output.schema.json"
FREEZE_MANIFEST = ROOT / "benchmarks/semantic-change/e04-freeze.json"
READINESS_GRAPH = ROOT / "benchmarks/semantic-change/e04-readiness-graph.json"
REFUSAL_ADAPTER_FILE = ROOT / "scripts/fixtures/e04-product-refusal-adapter.json"
TYPED_GOAL_ISSUABLE_ROOTS = ("MAP_EDGE", "TYPE_ASSIGNABLE", "PROPAGATE_DECLARED_TYPE")
CORPUS_FILES = (
    ROOT / "Cargo.lock",
    ROOT / "crates/semantic-corpus/Cargo.toml",
    ROOT / "crates/semantic-corpus/src/lib.rs",
    ROOT / "crates/semantic-corpus/src/main.rs",
    ROOT / "crates/semantic-corpus/src/e04.rs",
    ROOT / "crates/semantic-corpus/src/population.rs",
    ROOT / "crates/semantic-corpus/src/e04_authorization.rs",
    ROOT / "crates/semantic-corpus/src/e04_hidden_verification.rs",
    ROOT / "crates/semantic-corpus/src/product_coverage.rs",
    ROOT / "benchmarks/semantic-change/e04-product-coverage-v1.json",
)


def compact(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def self_module() -> Any:
    return sys.modules[__name__]


def sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha_file(path: Path) -> str:
    return sha_bytes(path.read_bytes())


def tree_sha256(root: Path) -> tuple[str, int, int]:
    if not root.is_dir():
        raise RuntimeError(f"dependency seed tree is missing: {root}")
    digest = hashlib.sha256(); files = bytes_total = 0
    entries = list(root.rglob("*"))
    symlink = next((item for item in entries if item.is_symlink()), None)
    if symlink is not None:
        raise RuntimeError(f"dependency seed contains a symlink: {symlink}")
    for path in sorted((item for item in entries if item.is_file()), key=lambda item: item.relative_to(root).as_posix()):
        relative = path.relative_to(root).as_posix().encode()
        data = path.read_bytes()
        digest.update(relative); digest.update(b"\0"); digest.update(data); digest.update(b"\0")
        files += 1; bytes_total += len(data)
    return digest.hexdigest(), files, bytes_total


def make_tree_read_only(root: Path) -> None:
    for path in sorted(root.rglob("*"), key=lambda item: len(item.parts), reverse=True):
        path.chmod(0o555 if path.is_dir() else 0o444)
    root.chmod(0o555)


def validate_read_only_regular_tree(root: Path) -> None:
    paths = [root, *root.rglob("*")]
    for path in paths:
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not (stat.S_ISDIR(metadata.st_mode) or stat.S_ISREG(metadata.st_mode)):
            raise RuntimeError(f"published dependency seed contains an unsupported entry: {path}")
        if metadata.st_mode & 0o222:
            raise RuntimeError(f"published dependency seed entry is writable: {path}")


def atomic_publish_dependency_seed(staged: Path, output: Path) -> dict[str, Any]:
    staged = staged.absolute(); output = output.absolute(); marker = Path(str(output) + ".complete")
    if os.path.lexists(output) or os.path.lexists(marker):
        raise RuntimeError(f"dependency seed output already exists: {output}")
    before = _validate_dependency_seed_payload(staged)
    envelope = staged.parent / "sealed-envelope"
    if os.path.lexists(envelope): raise RuntimeError("dependency seed staging envelope already exists")
    envelope.mkdir(mode=0o700); payload = envelope / "payload"; staged.replace(payload)
    make_tree_read_only(payload); validate_read_only_regular_tree(payload)
    frozen = _validate_dependency_seed_payload(payload)
    if frozen["manifestSha256"] != before["manifestSha256"]:
        raise RuntimeError("dependency seed changed while becoming read-only")
    envelope_identity = {"device":envelope.lstat().st_dev,"inode":envelope.lstat().st_ino}
    seal = {
        "schema":"semantic-editing-e04-dependency-seed-seal/0.1", "payload":"payload",
        "manifestSha256":frozen["manifestSha256"],
        "treeSha256":{name:frozen[name]["treeSha256"] for name in ("gradle","gradleWrapper","maven")},
        "envelopeIdentity":envelope_identity,
    }
    seal_path = envelope / "SEAL.json"; write_json(seal_path, seal); seal_path.chmod(0o444)
    envelope.replace(output)
    output.chmod(0o555); validate_read_only_regular_tree(output)
    completion = {"schema":"semantic-editing-e04-dependency-seed-completion/0.1","sealSha256":sha_file(output / "SEAL.json"),"envelopeIdentity":envelope_identity}
    marker_temporary = output.parent / f".{output.name}.complete-{os.getpid()}-{threading.get_ident()}"
    descriptor = os.open(marker_temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o444)
    try:
        os.write(descriptor, (compact(completion) + "\n").encode()); os.fsync(descriptor)
    finally:
        os.close(descriptor)
    try:
        os.link(marker_temporary, marker)
    finally:
        marker_temporary.unlink(missing_ok=True)
    published = validate_dependency_seed(output)
    if published["manifestSha256"] != frozen["manifestSha256"]:
        raise RuntimeError("published dependency seed digest mismatch")
    return published


def make_tree_writable(root: Path) -> None:
    root.chmod(0o755)
    for path in root.rglob("*"):
        path.chmod(0o755 if path.is_dir() else 0o644)


def clone_tree(source: Path, destination: Path) -> None:
    if destination.exists() or destination.is_symlink():
        raise RuntimeError(f"clone destination already exists: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    attempts = (
        ["cp", "-cR", str(source), str(destination)],
        ["cp", "--reflink=auto", "-a", str(source), str(destination)],
    )
    for command in attempts:
        result = subprocess.run(command, text=True, capture_output=True, check=False)
        if result.returncode == 0:
            return
    shutil.copytree(source, destination)


def sanitized_clew_environment(environment: dict[str, str]) -> dict[str, str]:
    clean = {
        key:value for key, value in environment.items()
        if key in CLEW_ENV_ALLOWLIST and key not in CLEW_ENV_DENY
        and not key.startswith("ORG_GRADLE_PROJECT_")
    }
    missing = [key for key in ("PATH", "HOME", "TMPDIR") if not clean.get(key)]
    if missing:
        raise RuntimeError(f"Clew sanitized environment lacks required variables: {missing}")
    return clean


def verified_clone_tree(source: Path, destination: Path, expected_sha: str) -> str:
    before, _, _ = tree_sha256(source)
    if before != expected_sha:
        raise RuntimeError(f"dependency snapshot changed before clone: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary_parent = Path(tempfile.mkdtemp(prefix=f".{destination.name}-clone-", dir=destination.parent))
    temporary_tree = temporary_parent / "tree"
    try:
        clone_tree(source, temporary_tree)
        cloned, _, _ = tree_sha256(temporary_tree)
        after, _, _ = tree_sha256(source)
        if cloned != expected_sha or after != expected_sha:
            raise RuntimeError(f"dependency snapshot changed during clone: {source}")
        # File modes are deliberately outside the content digest contract.
        # Only the per-run clone becomes writable; the frozen source remains
        # read-only and is revalidated after the atomic publication.
        make_tree_writable(temporary_tree)
        temporary_tree.replace(destination)
        final, _, _ = tree_sha256(destination)
        final_source, _, _ = tree_sha256(source)
        if final != expected_sha or final_source != expected_sha:
            raise RuntimeError(f"dependency clone digest mismatch: {destination}")
        return final
    finally:
        shutil.rmtree(temporary_parent, ignore_errors=True)


def freeze_dependency_seed(output: Path, gradle_cache: Path, gradle_wrapper: Path, maven_repo: Path) -> dict[str, Any]:
    if output.exists() or Path(str(output) + ".complete").exists():
        raise RuntimeError(f"dependency seed output already exists: {output}")
    gradle_cache, gradle_wrapper, maven_repo = gradle_cache.resolve(), gradle_wrapper.resolve(), maven_repo.resolve()
    staging_parent = Path(tempfile.mkdtemp(prefix=f".{output.name}-freeze-", dir=output.parent)); staged = staging_parent / "snapshot"
    try:
        staged.mkdir()
        clone_tree(gradle_cache, staged / "gradle-modules")
        clone_tree(maven_repo, staged / "maven-repository")
        if not gradle_wrapper.is_dir():
            raise RuntimeError(f"Gradle wrapper distribution cache is missing: {gradle_wrapper}")
        clone_tree(gradle_wrapper, staged / "gradle-wrapper-dists")
        gradle_sha, gradle_files, gradle_bytes = tree_sha256(staged / "gradle-modules")
        maven_sha, maven_files, maven_bytes = tree_sha256(staged / "maven-repository")
        wrapper_sha, wrapper_files, wrapper_bytes = tree_sha256(staged / "gradle-wrapper-dists")
        manifest = {
            "schema": "semantic-editing-e04-dependency-seed/0.1",
            "gradle": {"treeSha256": gradle_sha, "files": gradle_files, "bytes": gradle_bytes},
            "gradleWrapper": {"treeSha256": wrapper_sha, "files": wrapper_files, "bytes": wrapper_bytes},
            "maven": {"treeSha256": maven_sha, "files": maven_files, "bytes": maven_bytes},
        }
        write_json(staged / "manifest.json", manifest)
        return atomic_publish_dependency_seed(staged, output)
    finally:
        shutil.rmtree(staging_parent, ignore_errors=True)


def _validate_dependency_seed_payload(root: Path) -> dict[str, Any]:
    root = root.resolve(); manifest_path = root / "manifest.json"
    if not manifest_path.is_file():
        raise RuntimeError(f"dependency seed manifest is missing: {manifest_path}")
    manifest = load(manifest_path)
    schemas = {"semantic-editing-e04-dependency-seed/0.1":{"schema", "gradle", "gradleWrapper", "maven"}, "semantic-editing-e04-dependency-seed/0.2":{"schema", "gradle", "gradleWrapper", "maven", "augmentation"}}
    if not isinstance(manifest, dict) or manifest.get("schema") not in schemas or set(manifest) != schemas[manifest.get("schema")]:
        raise RuntimeError("invalid dependency seed manifest")
    for name, directory in (("gradle", root / "gradle-modules"), ("gradleWrapper", root / "gradle-wrapper-dists"), ("maven", root / "maven-repository")):
        actual_sha, actual_files, actual_bytes = tree_sha256(directory.resolve())
        expected = manifest.get(name)
        if not isinstance(expected, dict) or expected != {"treeSha256": actual_sha, "files": actual_files, "bytes": actual_bytes}:
            raise RuntimeError(f"dependency seed mismatch: {name}")
    return {**manifest, "root": str(root), "manifestSha256": sha_file(manifest_path)}


def validate_dependency_seed(root: Path) -> dict[str, Any]:
    root = root.absolute()
    if root.is_symlink() or not root.is_dir():
        raise RuntimeError(f"dependency seed root is invalid: {root}")
    if (root / "manifest.json").is_file():
        return _validate_dependency_seed_payload(root)
    marker = Path(str(root) + ".complete")
    if marker.is_symlink() or not marker.is_file():
        raise RuntimeError(f"sealed dependency seed completion marker is missing: {marker}")
    marker_mode = marker.lstat().st_mode
    if not stat.S_ISREG(marker_mode) or marker_mode & 0o222:
        raise RuntimeError("sealed dependency seed completion marker is invalid")
    entries = {path.name for path in root.iterdir()}
    if entries != {"payload", "SEAL.json"}:
        raise RuntimeError(f"sealed dependency seed envelope entries mismatch: {sorted(entries)}")
    validate_read_only_regular_tree(root)
    payload, seal_path = root / "payload", root / "SEAL.json"
    payload_record = _validate_dependency_seed_payload(payload)
    seal = load(seal_path)
    expected_seal_keys = {"schema","payload","manifestSha256","treeSha256","envelopeIdentity"}
    identity = {"device":root.lstat().st_dev,"inode":root.lstat().st_ino}
    expected_trees = {name:payload_record[name]["treeSha256"] for name in ("gradle","gradleWrapper","maven")}
    if not isinstance(seal, dict) or set(seal) != expected_seal_keys or seal.get("schema") != "semantic-editing-e04-dependency-seed-seal/0.1" or seal.get("payload") != "payload" or seal.get("manifestSha256") != payload_record["manifestSha256"] or seal.get("treeSha256") != expected_trees or seal.get("envelopeIdentity") != identity:
        raise RuntimeError("sealed dependency seed seal mismatch")
    completion = load(marker)
    expected_completion = {"schema":"semantic-editing-e04-dependency-seed-completion/0.1","sealSha256":sha_file(seal_path),"envelopeIdentity":identity}
    if completion != expected_completion:
        raise RuntimeError("sealed dependency seed completion mismatch")
    return {**payload_record,"envelopeRoot":str(root),"completionMarker":str(marker),"sealSha256":expected_completion["sealSha256"]}


def public_maven_seed_plan(experiment: Path) -> dict[str, Any]:
    tasks = [(path, value) for path, value in discover_public(experiment) if str(value["buildSystem"]).upper() == "MAVEN"]
    if len(tasks) != 21:
        raise RuntimeError(f"Maven dependency closure requires 21 public tasks, found {len(tasks)}")
    dependencies: set[str] = set(); plugins: set[str] = set(); poms = []
    for manifest_path, public in tasks:
        repository = manifest_path.parent / public["repository"]
        root_pom = repository / "pom.xml"
        if not root_pom.is_file():
            raise RuntimeError(f"public Maven root POM missing: {public['taskId']}")
        for pom in sorted(repository.rglob("pom.xml")):
            root = ET.parse(pom).getroot()
            properties = {xml_local_name(child.tag):(child.text or "").strip() for node in root if xml_local_name(node.tag) == "properties" for child in node}
            def value(node: ET.Element, name: str, default: str = "") -> str:
                found = xml_child(node, name); text = (found.text or "").strip() if found is not None else default
                for key, replacement in properties.items(): text = text.replace("${" + key + "}", replacement)
                return text
            for node in root.iter():
                kind = xml_local_name(node.tag)
                if kind == "dependency":
                    coordinate = ":".join((value(node,"groupId"), value(node,"artifactId"), value(node,"version")))
                    if coordinate.count(":") == 2 and not coordinate.startswith(":") and not coordinate.endswith(":"): dependencies.add(coordinate)
                elif kind == "plugin":
                    artifact, version = value(node,"artifactId"), value(node,"version")
                    if artifact and version: plugins.add(":".join((value(node,"groupId","org.apache.maven.plugins"), artifact, version)))
            poms.append({"taskId":public["taskId"], "path":pom.relative_to(repository).as_posix(), "sha256":sha_file(pom)})
    return {"tasks":[public["taskId"] for _, public in tasks], "poms":poms, "declaredDependencies":sorted(dependencies), "declaredPlugins":sorted(plugins)}


def maven_reactor_leaves(repository: Path) -> list[dict[str, Any]]:
    repository = repository.resolve(); pending = [repository / "pom.xml"]; leaves = []; seen = set(); gavs = set()
    while pending:
        pom = pending.pop(0).resolve()
        if not pom.is_relative_to(repository) or pom in seen or not pom.is_file() or pom.is_symlink():
            raise RuntimeError("invalid or duplicate Maven reactor POM")
        seen.add(pom); root = ET.parse(pom).getroot()
        gav = tuple((xml_child(root, name).text or "").strip() if xml_child(root, name) is not None else "" for name in ("groupId","artifactId","version"))
        if not all(gav) or gav in gavs: raise RuntimeError(f"duplicate or incomplete Maven reactor GAV: {gav}")
        gavs.add(gav); modules_node = xml_child(root, "modules")
        modules = [(child.text or "").strip() for child in xml_children(modules_node, "module")] if modules_node is not None else []
        if modules:
            for module in modules:
                relative = Path(module)
                if not relative.parts or relative.is_absolute() or ".." in relative.parts:
                    raise RuntimeError(f"unsafe Maven module path: {module}")
                child_pom = (pom.parent / relative / "pom.xml").resolve()
                if not child_pom.is_relative_to(repository): raise RuntimeError(f"Maven module escapes repository: {module}")
                pending.append(child_pom)
        else:
            dependencies = xml_child(root, "dependencies")
            dependency_bearing = dependencies is not None and bool(xml_children(dependencies, "dependency"))
            leaves.append({"pom":pom,"relativePom":pom.relative_to(repository).as_posix(),"gav":"{}:{}:{}".format(*gav),"dependencyBearing":dependency_bearing})
    if not leaves: raise RuntimeError("Maven reactor has no leaf modules")
    return sorted(leaves, key=lambda item:item["relativePom"])


def validate_maven_leaf_classpath(output: Path, checkout: Path, maven_repository: Path, dependency_bearing: bool, used_outputs: set[Path]) -> dict[str, Any]:
    output = output.resolve(); checkout = checkout.resolve(); maven_repository = maven_repository.resolve()
    if output in used_outputs: raise RuntimeError("MAVEN_LEAF_CLASSPATH_OUTPUT_COLLISION")
    used_outputs.add(output)
    if not output.is_relative_to(checkout) or output.is_symlink() or not output.is_file():
        raise RuntimeError("MAVEN_LEAF_CLASSPATH_OUTPUT_INVALID")
    entries = [Path(item).resolve() for item in output.read_text(encoding="utf-8").strip().split(os.pathsep) if item]
    if dependency_bearing and not entries: raise RuntimeError("MAVEN_LEAF_CLASSPATH_EMPTY")
    for entry in entries:
        if not entry.is_relative_to(maven_repository) or entry.is_symlink() or not entry.is_file():
            raise RuntimeError(f"MAVEN_LEAF_CLASSPATH_ARTIFACT_INVALID:{entry}")
    return {"output":str(output),"outputSha256":sha_file(output),"artifacts":[{"path":entry.relative_to(maven_repository).as_posix(),"sha256":sha_file(entry),"bytes":entry.stat().st_size} for entry in entries]}


def build_augmented_dependency_seed(base_seed_path: Path, experiment: Path, output: Path, repository_url: str) -> dict[str, Any]:
    if repository_url != "https://repo.maven.apache.org/maven2" or output.exists() or output.is_symlink():
        raise RuntimeError("augmentation requires absent output and the pinned Maven Central repository")
    base = validate_dependency_seed(base_seed_path); plan = public_maven_seed_plan(experiment)
    staging_parent = Path(tempfile.mkdtemp(prefix=f".{output.name}-augment-", dir=output.parent))
    try:
        fetched = staging_parent / "fetched-maven-repository"; fetched.mkdir()
        settings = staging_parent / "settings.xml"
        settings.write_text(f'<settings><mirrors><mirror><id>e04-central</id><name>E04 Central</name><url>{repository_url}</url><mirrorOf>*</mirrorOf></mirror></mirrors></settings>', encoding="utf-8")
        commands = []
        for index, task_id in enumerate(plan["tasks"]):
            source = experiment / "agent" / task_id / "repository"
            checkout = staging_parent / f"checkout-{index:02d}"; shutil.copytree(source, checkout)
            command = ["mvn", "-B", "-ntp", "-s", str(settings), f"-Dmaven.repo.local={fetched}", "dependency:go-offline", "test-compile"]
            result = subprocess.run(command, cwd=checkout, text=True, capture_output=True, check=False)
            commands.append({"taskId":task_id, "exitCode":result.returncode, "argv":command})
            if result.returncode: raise RuntimeError(f"online Maven closure failed for {task_id}: {(result.stdout + result.stderr)[-2000:]}")
        offline_commands = []
        for index, task_id in enumerate(plan["tasks"]):
            source = experiment / "agent" / task_id / "repository"
            checkout = staging_parent / f"offline-checkout-{index:02d}"; shutil.copytree(source, checkout)
            used_outputs: set[Path] = set(); leaf_results = []
            for leaf_index, leaf in enumerate(maven_reactor_leaves(checkout)):
                classpath = (checkout / "target" / f"e04-classpath-{leaf_index:02d}.txt").resolve(); classpath.parent.mkdir(parents=True, exist_ok=True)
                command = ["mvn", "-o", "-B", "-ntp", "-s", str(settings), f"-Dmaven.repo.local={fetched}", "-f", str(leaf["pom"]), "test-compile", "dependency:build-classpath", f"-Dmdep.outputFile={classpath}"]
                result = subprocess.run(command, cwd=checkout, text=True, capture_output=True, check=False)
                if result.returncode: raise RuntimeError(f"offline Maven closure failed for {task_id}/{leaf['relativePom']}: {(result.stdout + result.stderr)[-2000:]}")
                verified = validate_maven_leaf_classpath(classpath, checkout, fetched, leaf["dependencyBearing"], used_outputs)
                leaf_results.append({"gav":leaf["gav"],"relativePom":leaf["relativePom"],"dependencyBearing":leaf["dependencyBearing"],"exitCode":result.returncode,"argv":command,**verified})
            offline_commands.append({"taskId":task_id,"leaves":leaf_results})
        fetched_sha, fetched_files, fetched_bytes = tree_sha256(fetched)
        resolved_files = [{"path":path.relative_to(fetched).as_posix(), "bytes":path.stat().st_size, "sha256":sha_file(path)} for path in sorted(fetched.rglob("*")) if path.is_file()]
        staged = staging_parent / "snapshot"; staged.mkdir()
        for source_name, target_name in (("gradle-modules","gradle-modules"),("gradle-wrapper-dists","gradle-wrapper-dists"),("maven-repository","maven-repository")):
            clone_tree(Path(base["root"]) / source_name, staged / target_name)
        make_tree_writable(staged)
        shutil.copytree(fetched, staged / "maven-repository", dirs_exist_ok=True)
        records = {}
        for name, directory in (("gradle",staged/"gradle-modules"),("gradleWrapper",staged/"gradle-wrapper-dists"),("maven",staged/"maven-repository")):
            sha, files, size = tree_sha256(directory); records[name] = {"treeSha256":sha,"files":files,"bytes":size}
        manifest = {"schema":"semantic-editing-e04-dependency-seed/0.2",**records,"augmentation":{"baseManifestSha256":base["manifestSha256"],"repository":repository_url,"plan":plan,"fetchTreeSha256":fetched_sha,"fetchFiles":fetched_files,"fetchBytes":fetched_bytes,"resolvedFiles":resolved_files,"onlineCommands":commands,"offlineVerificationCommands":offline_commands}}
        write_json(staged / "manifest.json", manifest)
        return atomic_publish_dependency_seed(staged, output)
    finally:
        shutil.rmtree(staging_parent, ignore_errors=True)


def load(path: Path) -> Any:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def validate_typed_goal_catalog(value: Any) -> dict[str, Any]:
    required = {
        "schema", "version", "requestSchema", "goalSchema", "decisionSchema",
        "maxRequestBytes", "variableDomains", "operators", "executableDomains",
        "productRefusalReasons",
    }
    if not isinstance(value, dict) or set(value) != required or value["schema"] != "typed-goal-language-schema/0.1" or value["version"] != "0.1":
        raise RuntimeError("invalid typed-goal catalog schema")
    if not isinstance(value["maxRequestBytes"], int) or value["maxRequestBytes"] <= 0:
        raise RuntimeError("invalid typed-goal catalog request limit")
    domains = value["variableDomains"]
    if not isinstance(domains, list) or not domains or len(set(domains)) != len(domains) or not all(isinstance(item, str) and item for item in domains):
        raise RuntimeError("invalid typed-goal variable domains")
    operators = value["operators"]
    if not isinstance(operators, list) or not operators:
        raise RuntimeError("typed-goal catalog has no operators")
    names = set()
    for operator in operators:
        if not isinstance(operator, dict) or set(operator) != {"operator", "arity", "operandDomains", "requiredEvidenceRelations", "auxiliaryOnly", "refusalOnUnknown", "constraintDomain", "mandatoryApplications"}:
            raise RuntimeError("invalid typed-goal operator catalog entry")
        operand_domains = operator["operandDomains"]
        if operator["operator"] in names or not isinstance(operator["arity"], int) or operator["arity"] < 1 or not isinstance(operand_domains, list) or len(operand_domains) != operator["arity"] or any(not isinstance(choices, list) or not choices or any(domain not in domains for domain in choices) for choices in operand_domains):
            raise RuntimeError("invalid typed-goal operator arity/domain")
        if not isinstance(operator["auxiliaryOnly"], bool) or not isinstance(operator["refusalOnUnknown"], bool) or not isinstance(operator["constraintDomain"], str) or not isinstance(operator["requiredEvidenceRelations"], list) or not all(isinstance(item, str) and item for item in operator["requiredEvidenceRelations"]):
            raise RuntimeError("invalid typed-goal operator flags")
        names.add(operator["operator"])
    if not isinstance(value["executableDomains"], list) or not all(isinstance(item, str) for item in value["executableDomains"]):
        raise RuntimeError("invalid typed-goal executable domains")
    specs = {operator["operator"]:operator for operator in operators}
    for operator in operators:
        if not isinstance(operator["mandatoryApplications"], list):
            raise RuntimeError("invalid typed-goal mandatory applications")
        for application in operator["mandatoryApplications"]:
            if not isinstance(application, dict) or set(application) != {"operator", "operandIndices"} or application["operator"] not in names or not isinstance(application["operandIndices"], list) or len(application["operandIndices"]) != specs[application["operator"]]["arity"] or any(not isinstance(index, int) or index < 0 or index >= operator["arity"] for index in application["operandIndices"]):
                raise RuntimeError("invalid typed-goal mandatory application")
    refusals = value["productRefusalReasons"]
    if not isinstance(refusals, list) or not refusals or len(set(refusals)) != len(refusals) or not all(isinstance(item, str) for item in refusals):
        raise RuntimeError("invalid product refusal reasons")
    return value


def typed_goal_capabilities(catalog: dict[str, Any]) -> dict[str, Any]:
    specs = {operator["operator"]:operator for operator in catalog["operators"]}
    missing_roots = [operator for operator in TYPED_GOAL_ISSUABLE_ROOTS if operator not in specs or specs[operator]["auxiliaryOnly"] or specs[operator]["constraintDomain"] not in catalog["executableDomains"]]
    if missing_roots:
        raise RuntimeError(f"typed-goal issuable root contract changed: {missing_roots}")
    auxiliary = sorted(operator["operator"] for operator in catalog["operators"] if operator["auxiliaryOnly"])
    if auxiliary != ["BIND_UNIQUE", "VALUE_FLOWS_TO"]:
        raise RuntimeError(f"typed-goal auxiliary operator contract changed: {auxiliary}")
    all_domains = {operator["constraintDomain"] for operator in catalog["operators"]}
    non_executable = sorted(all_domains - set(catalog["executableDomains"]))
    if non_executable != ["NULLABLE_CONSTRUCTION", "PROJECTION", "RESOURCE_LIFETIME"]:
        raise RuntimeError(f"typed-goal non-executable domain contract changed: {non_executable}")
    return {
        "issuableRoots":list(TYPED_GOAL_ISSUABLE_ROOTS),
        "auxiliaryOperators":auxiliary,
        "nonExecutableDomains":non_executable,
    }


def load_refusal_adapter(catalog: dict[str, Any]) -> dict[str, Any]:
    value = load(REFUSAL_ADAPTER_FILE)
    if not isinstance(value, dict) or set(value) != {"schema", "mapping"} or value["schema"] != "e04-product-refusal-adapter/0.1" or not isinstance(value["mapping"], dict):
        raise RuntimeError("invalid E04 refusal adapter")
    if set(value["mapping"]) != set(catalog["productRefusalReasons"]) or any(mapped not in REFUSALS for mapped in value["mapping"].values()):
        raise RuntimeError("E04 refusal adapter does not cover the frozen product catalog")
    return {**value, "adapterSha256":sha_bytes(compact(value).encode())}


def load_typed_goal_catalog(clew: Path) -> dict[str, Any]:
    if not clew.is_absolute() or not clew.is_file():
        raise RuntimeError("typed-goal catalog requires an absolute frozen Codeclew binary")
    result = subprocess.run([str(clew), "schema", "typed-goal"], text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"Codeclew typed-goal catalog failed: {(result.stdout + result.stderr)[-2000:]}")
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("Codeclew typed-goal catalog is not JSON") from error
    catalog = validate_typed_goal_catalog(value)
    return {**catalog, "derivedCapabilities":typed_goal_capabilities(catalog), "catalogSha256":sha_bytes(compact(catalog).encode()), "binarySha256":sha_file(clew)}


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    temporary.replace(path)


def write_canonical_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(compact(value) + "\n", encoding="utf-8")
    temporary.replace(path)


def append_jsonl(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(compact(value) + "\n")
        handle.flush()
        os.fsync(handle.fileno())


def command_output(args: list[str], cwd: Path = ROOT) -> str:
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True, check=False)
    if result.returncode:
        raise RuntimeError(f"command failed ({' '.join(args)}): {result.stderr.strip()}")
    return result.stdout.strip()


def ast_index_provenance() -> dict[str, str]:
    discovered = shutil.which("ast-index")
    if not discovered:
        raise RuntimeError("ast-index executable is unavailable")
    real = Path(discovered).resolve(strict=True)
    metadata = real.lstat()
    if real.is_symlink() or not real.is_file() or not stat.S_ISREG(metadata.st_mode):
        raise RuntimeError(f"ast-index canonical executable is not a regular non-symlink file: {real}")
    version = command_output([str(real), "--version"])
    if version != AST_VERSION:
        raise RuntimeError(f"ast-index version mismatch: {version!r}")
    return {"realPath":str(real), "binarySha256":sha_file(real), "version":version}


def require_frozen_ast_index(provenance: dict[str, str]) -> None:
    manifest = load(FREEZE_MANIFEST)
    expected = {
        "realPath":manifest.get("astIndexExecutableRealPath"),
        "binarySha256":manifest.get("astIndexBinarySha256"),
        "version":manifest.get("astIndexVersion"),
    }
    if provenance != expected:
        raise RuntimeError(f"ast-index executable provenance mismatch: {provenance} != {expected}")


AST_STATS_KEYS = {
    "file_count", "ios_assets_count", "module_count", "refs_count",
    "resources_count", "storyboard_usages_count", "symbol_count", "xml_usages_count",
}


@contextlib.contextmanager
def anchored_ast_state(temporary_parent: Path, root_name: str = "state"):
    if not temporary_parent.is_absolute() or root_name in {"", ".", ".."} or "/" in root_name:
        raise RuntimeError("AST_STATE_UNSAFE_ANCHOR")
    nofollow = getattr(os, "O_NOFOLLOW", 0)
    flags = os.O_RDONLY | os.O_DIRECTORY | nofollow
    parent_fd = os.open(str(temporary_parent), flags)
    try:
        parent_stat = os.fstat(parent_fd)
        if not stat.S_ISDIR(parent_stat.st_mode):
            raise RuntimeError("AST_STATE_PARENT_NOT_DIRECTORY")
        try:
            os.mkdir(root_name, 0o700, dir_fd=parent_fd)
        except FileExistsError:
            raise RuntimeError("AST_STATE_ROOT_PREEXISTS")
        root_fd = os.open(root_name, flags, dir_fd=parent_fd)
        try:
            root_stat = os.fstat(root_fd)
            if not stat.S_ISDIR(root_stat.st_mode):
                raise RuntimeError("AST_STATE_ROOT_NOT_DIRECTORY")
        finally:
            os.close(root_fd)
        yield {
            "parentFd":parent_fd, "parentIdentity":(parent_stat.st_dev, parent_stat.st_ino),
            "rootName":root_name, "rootIdentity":(root_stat.st_dev, root_stat.st_ino),
            "rootPath":temporary_parent / root_name,
        }
    finally:
        os.close(parent_fd)


def ast_db_identity(expected_db_path: Path, state_anchor: dict[str, Any]) -> tuple[Path, Path]:
    containment_root = Path(state_anchor["rootPath"])
    if not expected_db_path.is_absolute() or not containment_root.is_absolute():
        raise RuntimeError("AST_DB_PATH_NOT_ABSOLUTE")
    try:
        relative = expected_db_path.relative_to(containment_root)
    except ValueError:
        raise RuntimeError("AST_DB_ESCAPES_CONTAINMENT")
    if not relative.parts or any(part in {"", ".", ".."} for part in relative.parts) or relative.is_absolute():
        raise RuntimeError("AST_DB_UNSAFE_RELATIVE_PATH")
    return containment_root.joinpath(relative), relative


def parse_ast_readiness(stdout: str, expected_db_path: Path) -> dict[str, Any]:
    try:
        payload = json.loads(stdout)
    except (TypeError, json.JSONDecodeError):
        raise RuntimeError("AST_READINESS_MALFORMED_JSON")
    if not isinstance(payload, dict) or set(payload) != {"db_path", "db_size_bytes", "stats"}:
        raise RuntimeError("AST_READINESS_WRONG_CONTRACT")
    stats = payload.get("stats")
    if not isinstance(stats, dict) or set(stats) != AST_STATS_KEYS:
        raise RuntimeError("AST_READINESS_WRONG_STATS_SCHEMA")
    if not all(isinstance(value, int) and not isinstance(value, bool) and value >= 0 for value in stats.values()):
        raise RuntimeError("AST_READINESS_INVALID_COUNTS")
    expected = str(expected_db_path)
    if payload.get("db_path") != expected:
        raise RuntimeError("AST_READINESS_WRONG_DB_PATH")
    if not isinstance(payload.get("db_size_bytes"), int) or isinstance(payload.get("db_size_bytes"), bool) or payload["db_size_bytes"] <= 0:
        raise RuntimeError("AST_READINESS_EMPTY_DB")
    if any(stats[key] <= 0 for key in ("file_count", "module_count", "symbol_count", "refs_count")):
        raise RuntimeError("AST_READINESS_TRIVIAL_INDEX")
    return {
        "schema":"semantic-editing-e04-ast-readiness/0.1", "status":"READY",
        "dbPath":expected, "dbSizeBytes":payload["db_size_bytes"],
        "fileCount":stats["file_count"], "moduleCount":stats["module_count"],
        "symbolCount":stats["symbol_count"], "refsCount":stats["refs_count"],
    }


def attest_ast_db_artifact(
    summary: dict[str, Any],
    expected_db_path: Path,
    state_anchor: dict[str, Any],
    expected_sha256: str | None = None,
    test_hook: Any | None = None,
) -> dict[str, Any]:
    canonical_database, relative = ast_db_identity(expected_db_path, state_anchor)
    nofollow = getattr(os, "O_NOFOLLOW", 0)
    directory_flags = os.O_RDONLY | os.O_DIRECTORY | nofollow
    file_flags = os.O_RDONLY | nofollow
    directory_fds: list[int] = []
    database_fd: int | None = None
    try:
        parent_stat = os.fstat(state_anchor["parentFd"])
        if (parent_stat.st_dev, parent_stat.st_ino) != tuple(state_anchor["parentIdentity"]):
            raise RuntimeError("AST_STATE_PARENT_IDENTITY_CHANGED")
        try:
            root_fd = os.open(state_anchor["rootName"], directory_flags, dir_fd=state_anchor["parentFd"])
        except OSError as error:
            if error.errno in {errno.ELOOP, errno.ENOTDIR, errno.ENOENT}:
                raise RuntimeError("AST_STATE_ROOT_SYMLINK_OR_MISSING")
            raise
        directory_fds.append(root_fd)
        root_stat = os.fstat(root_fd)
        if not stat.S_ISDIR(root_stat.st_mode):
            raise RuntimeError("AST_DB_INVALID_CONTAINMENT_ROOT")
        if (root_stat.st_dev, root_stat.st_ino) != tuple(state_anchor["rootIdentity"]):
            raise RuntimeError("AST_STATE_ROOT_IDENTITY_CHANGED")
        current_fd = root_fd
        for index, part in enumerate(relative.parts[:-1]):
            if test_hook is not None:
                test_hook("before_parent_open", {"index":index, "part":part})
            try:
                next_fd = os.open(part, directory_flags, dir_fd=current_fd)
            except OSError as error:
                if error.errno in {errno.ELOOP, errno.ENOTDIR, errno.ENOENT}:
                    raise RuntimeError("AST_DB_SYMLINKED_PARENT")
                raise
            directory_fds.append(next_fd); current_fd = next_fd
        if test_hook is not None:
            test_hook("before_db_open", {"name":relative.parts[-1]})
        try:
            database_fd = os.open(relative.parts[-1], file_flags, dir_fd=current_fd)
        except OSError as error:
            if error.errno == errno.ENOENT:
                raise RuntimeError("AST_DB_MISSING")
            if error.errno in {errno.ELOOP, errno.EMLINK}:
                raise RuntimeError("AST_DB_SYMLINK")
            raise
        before = os.fstat(database_fd)
        if not stat.S_ISREG(before.st_mode):
            raise RuntimeError("AST_DB_NOT_REGULAR")
        if before.st_size <= 0 or before.st_size != summary.get("dbSizeBytes"):
            raise RuntimeError("AST_DB_SIZE_MISMATCH")
        if test_hook is not None:
            test_hook("after_fstat_before_read", {"fd":database_fd})
        os.lseek(database_fd, 0, os.SEEK_SET)
        digest = hashlib.sha256()
        while True:
            chunk = os.read(database_fd, 1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
        after = os.fstat(database_fd)
        stable_before = (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
        stable_after = (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
        if stable_before != stable_after:
            raise RuntimeError("AST_DB_CHANGED_DURING_READ")
        actual_sha = digest.hexdigest()
        if expected_sha256 is not None and actual_sha != expected_sha256:
            raise RuntimeError("AST_DB_SHA256_MISMATCH")
        enriched = {**summary, "dbPath":str(canonical_database), "actualDbSizeBytes":before.st_size, "dbSha256":actual_sha}
        validate_ast_readiness_summary(enriched, canonical_database)
        return enriched
    finally:
        if database_fd is not None:
            os.close(database_fd)
        for descriptor in reversed(directory_fds):
            os.close(descriptor)


def validate_ast_readiness_summary(value: Any, expected_db_path: Path) -> None:
    required = {"schema", "status", "dbPath", "dbSizeBytes", "actualDbSizeBytes", "dbSha256", "fileCount", "moduleCount", "symbolCount", "refsCount"}
    if not isinstance(value, dict) or set(value) != required or value.get("schema") != "semantic-editing-e04-ast-readiness/0.1" or value.get("status") != "READY":
        raise RuntimeError("AST_READINESS_INVALID_NORMALIZED_SCHEMA_OR_STATUS")
    if value.get("dbPath") != str(expected_db_path):
        raise RuntimeError("AST_READINESS_WRONG_DB_PATH")
    if any(not isinstance(value.get(key), int) or isinstance(value.get(key), bool) or value[key] <= 0 for key in ("dbSizeBytes", "actualDbSizeBytes", "fileCount", "moduleCount", "symbolCount", "refsCount")):
        raise RuntimeError("AST_READINESS_TRIVIAL_INDEX")
    if value["actualDbSizeBytes"] != value["dbSizeBytes"]:
        raise RuntimeError("AST_DB_SIZE_MISMATCH")
    if not isinstance(value.get("dbSha256"), str) or not re.fullmatch(r"[0-9a-f]{64}", value["dbSha256"]):
        raise RuntimeError("AST_DB_SHA256_INVALID")


def population() -> dict[str, Any]:
    if sha_file(POPULATION) != POP_SHA:
        raise RuntimeError("frozen population digest mismatch")
    value = load(POPULATION)
    if value.get("plannedTaskCount") != 42 or len(value.get("slots", [])) != 42:
        raise RuntimeError("frozen population must contain exactly 42 slots")
    return value


def binder_paths() -> list[str]:
    paths = command_output(["git", "ls-tree", "-r", "--name-only", BASE]).splitlines()
    selected = []
    for path in paths:
        if path in {"Cargo.toml", "Cargo.lock", "build.gradle.kts", "settings.gradle.kts"}:
            selected.append(path)
        elif path == "crates/clew/Cargo.toml" or path.startswith("crates/clew/src/"):
            selected.append(path)
        elif path.startswith("schemas/"):
            selected.append(path)
        elif path == "workers/kotlin/build.gradle.kts" or path.startswith("workers/kotlin/src/main/"):
            selected.append(path)
        elif path == "workers/kotlin21/build.gradle.kts":
            selected.append(path)
    if len(selected) != 44:
        raise RuntimeError(f"frozen binder contour changed: expected 44 paths, found {len(selected)}")
    return sorted(selected)


def binder_tree_sha256() -> str:
    digest = hashlib.sha256()
    for path in binder_paths():
        data = subprocess.run(
            ["git", "show", f"{BASE}:{path}"], cwd=ROOT, capture_output=True, check=True
        ).stdout
        digest.update(path.encode())
        digest.update(b"\0")
        digest.update(data)
        digest.update(b"\0")
    return digest.hexdigest()


def obligations_catalog(spec: dict[str, Any]) -> tuple[str, ...]:
    return tuple(sorted({item for family in spec["families"] for item in family["requiredObligations"]}))


def prompt_typed_goal_catalog(catalog: dict[str, Any]) -> dict[str, Any]:
    return {key:value for key, value in catalog.items() if key not in {"catalogSha256", "binarySha256", "refusalMapping"}}


def common_prompt(spec: dict[str, Any], typed_catalog: dict[str, Any]) -> str:
    catalog = {
        "familyContracts": FAMILY_CONTRACTS,
        "roleSemantics": ROLE_SEMANTICS,
        "refusalCodes": REFUSAL_SEMANTICS,
        "oracleClasses": ORACLE_SEMANTICS,
        "typedGoalCatalog": prompt_typed_goal_catalog(typed_catalog),
    }
    return (
        "E04 is a binder-only experiment. Do not create, edit, delete, format, or build source. "
        "Return exactly one JSON object accepted by semantic-editing-e04-model-output/0.1. "
        "Infer only from the public task and permitted evidence. BOUND requires exactly one complete "
        "binding for every role of the inferred family, the exact three catalog obligations, evidence "
        "anchors that name each bound symbol, and an explicit oracle class. Never guess: return "
        "AMBIGUOUS with every complete binding set when two or more equally valid sets remain; return "
        "REFUSED with the most specific catalog code when evidence, policy, lifecycle/effect knowledge, "
        "query/schema support, or an independent oracle is missing. A tool being unsupported is not by "
        "itself evidence that the source task is semantically unsupported. "
        "The following global catalog is shared unchanged by every task and reveals no current "
        f"slot label:\n{compact(catalog)}"
    )


def mode_prompt(
    arm: str,
    clew: Path | None,
    compilation: str = ":/main",
    test_compilation: str = ":/test",
    state: Path | None = None,
    base_revision: str = "<ISOLATED_HEAD>",
    typed_catalog: dict[str, Any] | None = None,
) -> str:
    if arm == "default":
        return (
            "Use ordinary read-only filesystem search and exact source reads. Run one command per tool "
            "call; shell pipelines, conjunctions, and source-writing redirects are forbidden (stderr "
            "suppression with `2>/dev/null` is allowed). Do not use ast-index or Codeclew."
        )
    if arm == "ast-index":
        return (
            "Use ast-index 3.48.1 for every navigation decision. Run one command per tool call: "
            "first `ast-index rebuild`, then individual ast-index queries. Exact source reads are allowed "
            "only as a separate `sed -n 'START,ENDp' PATH` call where PATH appeared in prior ast-index "
            "output and the span is at most 240 lines. Shell pipelines, conjunctions, redirects, grep, "
            "rg, find, broad cat, and Codeclew are forbidden."
        )
    binary = str(clew.resolve()) if clew else "<ABSOLUTE_CLEW_BINARY>"
    if typed_catalog is None:
        raise RuntimeError("typed-goal catalog is required for prompt construction")
    return (
        f"Use only the absolute binary {binary}. The repository argument is always exactly `--repo .`. "
        f"The discovered production compilation is `{compilation}` and the test compilation is "
        f"`{test_compilation}`; pass them explicitly. "
        "Run one command per tool call; shell pipelines, conjunctions, redirects, filesystem reads, "
        "grep/search, ast-index, projection, symbol-directed queries, and help commands are forbidden. "
        "The only allowed invocations are: "
        f"`{binary} project inspect --repo . --compilation {compilation}`; and "
        f"`{binary} prove typed-goal --repo . --compilation {compilation} --request-json '<COMPACT_CANONICAL_JSON>'`. "
        f"The request must use baseRevision `{base_revision}`, compilation `{compilation}`, be at most "
        f"{typed_catalog['maxRequestBytes']} UTF-8 bytes, and conform to the global family-neutral catalog above. "
        "The harness binds the authored request digest to this task packet. "
        "If the permitted evidence cannot bind the task, return the most specific REFUSED code."
    )


def validate_public(value: Any, path: Path) -> None:
    if not isinstance(value, dict) or set(value) != PUBLIC_KEYS:
        raise RuntimeError(f"invalid public manifest keys: {path}")
    if value["schema"] != "semantic-editing-e04-public-task/0.1":
        raise RuntimeError(f"invalid public schema: {path}")
    if value["repository"] != "repository" or not isinstance(value["task"], str):
        raise RuntimeError(f"invalid public repository/task: {path}")
    if not re.fullmatch(r"e04-[0-9a-f]{16}", value["taskId"]):
        raise RuntimeError(f"invalid opaque task ID: {path}")


def discover_public(experiment: Path) -> list[tuple[Path, dict[str, Any]]]:
    found = []
    for path in sorted((experiment / "agent").glob("*/task-manifest.json")):
        value = load(path)
        validate_public(value, path)
        repo = path.parent / value["repository"]
        if not repo.is_dir():
            raise RuntimeError(f"public repository missing: {repo}")
        found.append((path, value))
    if len(found) != 42 or len({value["taskId"] for _, value in found}) != 42:
        raise RuntimeError(f"E04 requires 42 unique public tasks, found {len(found)}")
    return found


def frozen_checks(
    check_tools: bool = True,
    check_manifest: bool = True,
    dependency_seed: dict[str, Any] | None = None,
    typed_catalog: dict[str, Any] | None = None,
    refusal_adapter: dict[str, Any] | None = None,
    freeze_manifest_path: Path | None = None,
) -> dict[str, Any]:
    freeze_manifest_path = freeze_manifest_path or FREEZE_MANIFEST
    spec = population()
    if sha_file(OUTPUT_SCHEMA) == "":
        raise RuntimeError("output schema missing")
    head = command_output(["git", "rev-parse", "HEAD"])
    binder_digest = binder_tree_sha256()
    if binder_digest != BINDER_TREE_SHA:
        raise RuntimeError(f"binder tree mismatch: expected {BINDER_TREE_SHA}, got {binder_digest}")
    result = {
        "harnessCommit": head,
        "productBaseCommit": BASE,
        "binderTreeSha256": binder_digest,
        "populationSha256": POP_SHA,
        "outputSchemaSha256": sha_file(OUTPUT_SCHEMA),
        "commonPromptSha256": sha_bytes(common_prompt(spec, typed_catalog).encode()) if typed_catalog else None,
        "plannedTasks": 42,
        "plannedRuns": 126,
        "model": MODEL,
        "reasoning": EFFORT,
    }
    if check_tools:
        codex = command_output(["codex", "--version"])
        ast_provenance = ast_index_provenance()
        require_frozen_ast_index(ast_provenance)
        if codex != CODEX_VERSION:
            raise RuntimeError(f"tool freeze mismatch: codex={codex!r}, ast-index={ast_provenance['version']!r}")
        result.update(codexVersion=codex, astIndexVersion=ast_provenance["version"], astIndexExecutable=ast_provenance)
    if freeze_manifest_path.is_file() and check_manifest:
        manifest = load(freeze_manifest_path)
        if typed_catalog is None:
            raise RuntimeError("frozen experiment requires the Codeclew typed-goal catalog")
        if refusal_adapter is None:
            raise RuntimeError("frozen experiment requires the E04 refusal adapter")
        if manifest.get("codeclewBinarySha256") != typed_catalog["binarySha256"]:
            raise RuntimeError("Codeclew binary does not match catalog provenance")
        if dependency_seed is None:
            seed_root = os.environ.get("E04_DEPENDENCY_SEED")
            if not seed_root:
                raise RuntimeError("E04_DEPENDENCY_SEED is required by the frozen experiment")
            dependency_seed = validate_dependency_seed(Path(seed_root))
        seed = dependency_seed
        expected = {
            "productBaseCommit": BASE,
            "binderTreeSha256": binder_digest,
            "populationSha256": POP_SHA,
            "model": MODEL,
            "reasoning": EFFORT,
            "outputSchemaSha256": result["outputSchemaSha256"],
            "commonPromptSha256": result["commonPromptSha256"],
            "runnerSha256": sha_file(Path(__file__)),
            "corpusFileSha256": {str(path.relative_to(ROOT)): sha_file(path) for path in CORPUS_FILES},
            "dependencySeedManifestSha256": seed["manifestSha256"],
            "typedGoalCatalogSha256": typed_catalog["catalogSha256"],
            "refusalAdapterSha256": refusal_adapter["adapterSha256"],
        }
        if manifest.get("schema") != "semantic-editing-e04-freeze/0.1":
            raise RuntimeError("invalid E04 freeze manifest schema")
        for key, value in expected.items():
            if manifest.get(key) != value:
                raise RuntimeError(f"E04 freeze manifest mismatch: {key}")
        result["freezeManifestSha256"] = sha_file(freeze_manifest_path)
        result["dependencySeed"] = seed
        result["harnessCommit"] = manifest["harnessCommit"]
        result["freezeState"] = "FROZEN"
    elif not freeze_manifest_path.is_file():
        result["freezeState"] = "PENDING_MANIFEST"
    else:
        result["freezeState"] = "MANIFEST_CHECK_SKIPPED"
    return result


def slot_id(slot: dict[str, Any], index: int) -> str:
    raw = f"{index}:{slot['family']}:{slot['variant']}:{slot['buildSystem']}:{slot['ordinal']}"
    return "unmaterialized-" + sha_bytes(raw.encode())[:16]


def matrix(experiment: Path | None) -> list[dict[str, Any]]:
    spec = population()
    if experiment:
        tasks = [(value["taskId"], sha_file(path), str(path)) for path, value in discover_public(experiment)]
    else:
        tasks = [(slot_id(slot, i), None, None) for i, slot in enumerate(spec["slots"])]
    rows = []
    for task_id, manifest_sha, manifest_path in tasks:
        arm_order = sorted(ARMS, key=lambda arm: sha_bytes(f"{POP_SHA}:{task_id}:{arm}".encode()))
        for order, arm in enumerate(arm_order):
            rows.append({
                "runId": f"{task_id}--{arm}", "taskId": task_id, "arm": arm,
                "taskArmOrder": order, "publicManifest": manifest_path,
                "publicManifestSha256": manifest_sha, "state": "PLANNED",
            })
    if len(rows) != 126:
        raise RuntimeError("run matrix is not 42x3")
    return sorted(rows, key=lambda row: (row["taskId"], row["taskArmOrder"]))


def plan_packets(
    output: Path,
    experiment: Path | None,
    check_tools: bool,
    check_manifest: bool = True,
    dependency_seed: dict[str, Any] | None = None,
    typed_catalog: dict[str, Any] | None = None,
    refusal_adapter: dict[str, Any] | None = None,
    freeze_manifest_path: Path | None = None,
    r1_matrix: bool = False,
) -> dict[str, Any]:
    freeze = frozen_checks(check_tools, check_manifest, dependency_seed, typed_catalog, refusal_adapter, freeze_manifest_path)
    rows = matrix(experiment)
    output.mkdir(parents=True, exist_ok=True)
    for row in rows:
        write_json(output / "planned" / row["runId"] / "run-packet.json", row)
    manifest = {
        "schema": "semantic-editing-e04-plan/0.1", "freeze": freeze,
        "experimentRoot": str(experiment.resolve()) if experiment else None,
        "runs": rows,
    }
    if experiment:
        grouped: dict[str, list[dict[str, Any]]] = {}
        for row in rows:
            grouped.setdefault(row["taskId"], []).append(row)
        selected=preregistered_r1_triplets(grouped) if r1_matrix else preregistered_canary_triplets(grouped)
        manifest["r7CanaryTaskIds"] = [triplet[0]["taskId"] for triplet in selected]
    write_json(output / "plan.json", manifest)
    return manifest


def source_digest(root: Path) -> str:
    digest = hashlib.sha256()
    entries = sorted(
        ((path.relative_to(root).as_posix(), path) for path in root.rglob("*")),
        key=lambda entry: entry[0],
    )
    for relative_text, path in entries:
        relative_path = path.relative_to(root)
        if any(part in STATE_DIRECTORIES for part in relative_path.parts):
            continue
        if path.is_symlink():
            raise RuntimeError(f"source snapshot contains a symlink: {relative_path}")
        if not path.is_file():
            continue
        relative = relative_text.encode()
        digest.update(relative); digest.update(b"\0")
        digest.update(path.read_bytes()); digest.update(b"\0")
    return digest.hexdigest()


def event_metrics(lines: list[str]) -> tuple[dict[str, Any], list[dict[str, Any]], list[str]]:
    events, errors = [], []
    input_tokens = cached = output_tokens = 0
    native = False; turns = 0; tool_items: dict[str, dict[str, Any]] = {}; tool_bytes = 0
    for number, line in enumerate(lines, 1):
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            errors.append(f"INVALID_JSONL_LINE:{number}"); continue
        events.append(event)
        kind = event.get("type")
        if kind == "turn.completed":
            turns += 1; usage = event.get("usage") or {}
            def token(name: str) -> int:
                return int(usage.get(name, usage.get(''.join([name.split('_')[0]] + [p.title() for p in name.split('_')[1:]]), 0)) or 0)
            input_tokens += token("input_tokens"); cached += token("cached_input_tokens"); output_tokens += token("output_tokens"); native = bool(usage) or native
        item = event.get("item") if isinstance(event.get("item"), dict) else None
        if item and item.get("type") in {"command_execution", "mcp_tool_call", "web_search", "file_change"}:
            key = str(item.get("id", f"{number}:{item.get('type')}")); tool_items[key] = item
            for field in ("aggregated_output", "output", "result"):
                if field in item:
                    tool_bytes += len((item[field] if isinstance(item[field], str) else compact(item[field])).encode())
    commands = []
    for item in tool_items.values():
        if item.get("type") == "command_execution":
            command = item.get("command", "")
            rendered = " ".join(command) if isinstance(command, list) else str(command)
            output = ""
            for field in ("aggregated_output", "output", "result"):
                if field in item:
                    output += item[field] if isinstance(item[field], str) else compact(item[field])
            commands.append({"command": rendered, "output": output, "exitCode": item.get("exit_code", item.get("exitCode"))})
        elif item.get("type") == "file_change":
            commands.append({"command": "FILE_CHANGE_EVENT", "output": ""})
    metrics = {
        "turns": turns, "actionCalls": len(tool_items), "toolOutputBytes": tool_bytes,
        "inputTokens": input_tokens if native else None,
        "cachedInputTokens": cached if native else None,
        "outputTokens": output_tokens if native else None,
        "noncachedTokens": input_tokens - cached + output_tokens if native else None,
        "nativeTokenTelemetryAvailable": native,
    }
    return metrics, events, commands + [{"command": error, "output": ""} for error in errors]


def unwrap_codex_command(command: str) -> tuple[list[str], list[str]]:
    """Return one simple command from the exact Codex zsh event envelope.

    Codex JSONL records command executions as `/bin/zsh -lc '<body>'` (or
    `-c`).  Auditing the outer executable caused every real S0 AST/Codeclew
    call to be classified as unused.  This parser deliberately accepts only
    that exact envelope and one command body; it is not a general shell parser.
    """
    try:
        outer = shlex.split(command)
    except ValueError:
        return [], ["INVALID_COMMAND_ENVELOPE"]
    if len(outer) != 3 or outer[0] != "/bin/zsh" or outer[1] not in {"-lc", "-c"}:
        return [], ["INVALID_COMMAND_ENVELOPE"]
    body = outer[2]
    if "\n" in body or "`" in body or "$(" in body:
        return [], ["COMPOUND_COMMAND"]
    try:
        lexer = shlex.shlex(body, posix=True, punctuation_chars=";&|<>")
        lexer.whitespace_split = True
        lexer.commenters = ""
        raw = list(lexer)
    except ValueError:
        return [], ["INVALID_COMMAND_BODY"]
    if any(token in {";", ";;", "&", "&&", "|", "||", "<", "<<", "<<<"} for token in raw):
        return [], ["COMPOUND_COMMAND"]

    tokens: list[str] = []
    errors: list[str] = []
    index = 0
    while index < len(raw):
        token = raw[index]
        if token in {">", ">>"}:
            fd = tokens.pop() if tokens and tokens[-1].isdigit() else "1"
            target = raw[index + 1] if index + 1 < len(raw) else ""
            if fd == "2" and target == "/dev/null":
                index += 2
                continue
            errors.append("SOURCE_EDIT_ATTEMPT")
            index += 2
            continue
        tokens.append(token)
        index += 1
    if not tokens:
        errors.append("EMPTY_COMMAND")
    return tokens, sorted(set(errors))


def valid_ast_argv(tokens: list[str]) -> bool:
    if not tokens or Path(tokens[0]).name != "ast-index" or len(tokens) < 2 or "--help" in tokens or "--version" in tokens:
        return False
    command, rest = tokens[1], tokens[2:]
    if command == "rebuild":
        return rest in ([], ["--format", "json"])
    if command not in {"search", "outline", "callers", "callees", "references"} or not rest or rest[0].startswith("-"):
        return False
    index = 1
    while index < len(rest):
        option = rest[index]
        if option == "--format" and index + 1 < len(rest) and rest[index + 1] == "json":
            index += 2
        elif option == "--limit" and index + 1 < len(rest) and rest[index + 1].isdigit():
            index += 2
        else:
            return False
    return True


def valid_default_argv(tokens: list[str]) -> bool:
    if not tokens:
        return False
    executable = Path(tokens[0]).name
    if executable == "sed":
        return len(tokens) == 4 and tokens[1] == "-n" and bool(re.fullmatch(r"\d+(?:,\d+)?p", tokens[2])) and not tokens[3].startswith("-")
    if executable != "rg" or len(tokens) < 2:
        return False
    forbidden = {"--pre", "--replace", "-r", "--passthru", "--help", "-h", "--version", "-V"}
    return not any(token in forbidden or token.startswith("--pre=") or token.startswith("--replace=") for token in tokens[1:])


def validate_inline_typed_request(
    rendered: str,
    selected_compilation: str | None,
    base_revision: str | None,
    typed_catalog: dict[str, Any],
) -> tuple[str | None, dict[str, str] | None]:
    if len(rendered.encode()) > typed_catalog["maxRequestBytes"]:
        return "INLINE_REQUEST_TOO_LARGE", None
    try:
        request = json.loads(rendered)
    except json.JSONDecodeError:
        return "INVALID_INLINE_REQUEST", None
    if compact(request) != rendered or not isinstance(request, dict) or set(request) != {"schema", "goal", "hints", "compilation"}:
        return "INVALID_INLINE_REQUEST", None
    if request["schema"] != typed_catalog["requestSchema"] or request["compilation"] != selected_compilation or not isinstance(request["hints"], list) or not all(isinstance(item, str) for item in request["hints"]):
        return "INVALID_INLINE_REQUEST", None
    goal = request["goal"]
    if not isinstance(goal, dict) or set(goal) != {"schema", "baseRevision", "variables", "operators"} or goal["schema"] != typed_catalog["goalSchema"] or goal["baseRevision"] != base_revision:
        return "INVALID_INLINE_REQUEST", None
    variables, operators = goal["variables"], goal["operators"]
    if not isinstance(variables, dict) or not variables or not all(isinstance(key, str) and key and value in typed_catalog["variableDomains"] for key, value in variables.items()):
        return "INVALID_INLINE_REQUEST", None
    if not isinstance(operators, list) or not operators:
        return "INVALID_INLINE_REQUEST", None
    specs = {item["operator"]:item for item in typed_catalog["operators"]}
    used_operand_ids: set[str] = set()
    for application in operators:
        if not isinstance(application, dict) or set(application) != {"operator", "operands"} or application["operator"] not in specs or not isinstance(application["operands"], list):
            return "INVALID_INLINE_REQUEST", None
        spec = specs[application["operator"]]; operands = application["operands"]
        if spec["auxiliaryOnly"] or len(operands) != spec["arity"] or any(operand not in variables for operand in operands) or any(variables[operand] not in domains for operand, domains in zip(operands, spec["operandDomains"])):
            return "INVALID_INLINE_REQUEST", None
        used_operand_ids.update(operands)
    if used_operand_ids != set(variables):
        return "INVALID_INLINE_REQUEST", None
    digest = sha_bytes(rendered.encode())
    return None, {"sha256":digest, "canonicalJson":rendered, "compilation":str(selected_compilation), "baseRevision":str(base_revision)}


def valid_clew_argv(
    tokens: list[str],
    expected_binary: str,
    selected_compilation: str | None,
    base_revision: str | None,
    typed_catalog: dict[str, Any],
) -> tuple[str | None, dict[str, str] | None, str | None]:
    if not tokens or tokens[0] != expected_binary or "--help" in tokens or "-h" in tokens:
        return "INVALID_TOOL_ARGUMENTS", None, None
    args = tokens[1:]
    repo_positions = [index for index, token in enumerate(args) if token == "--repo"]
    if len(repo_positions) != 1 or repo_positions[0] + 1 >= len(args) or args[repo_positions[0] + 1] != ".":
        return "INVALID_REPO_PROTOCOL", None, None
    if args[:2] == ["project", "inspect"]:
        valid = len(args) == 6 and args[2:4] == ["--repo", "."] and args[4] == "--compilation" and args[5] == selected_compilation
        return (None, None, "PROJECT_INSPECT") if valid else ("INVALID_TOOL_ARGUMENTS", None, None)
    if args[:2] == ["prove", "typed-goal"]:
        valid = (
            len(args) == 8 and args[2:4] == ["--repo", "."]
            and args[4:6] == ["--compilation", selected_compilation]
            and args[6] == "--request-json"
        )
        if not valid:
            return "INVALID_TOOL_ARGUMENTS", None, None
        error, record = validate_inline_typed_request(args[7], selected_compilation, base_revision, typed_catalog)
        return error, record, "TYPED_GOAL" if error is None else None
    return "INVALID_TOOL_ARGUMENTS", None, None


def audit(
    arm: str,
    commands: list[Any],
    before: str,
    after: str,
    clew_path: Path | None = None,
    repository: Path | None = None,
    selected_compilation: str | None = None,
    base_revision: str | None = None,
    request_records: list[dict[str, Any]] | None = None,
    typed_catalog: dict[str, Any] | None = None,
) -> tuple[list[str], int]:
    flags = []
    if before != after:
        flags.append("SOURCE_MUTATION")
    navigation = 0; used_ast = False; used_clew_proof = False
    ast_evidence = ""
    for record in commands:
        command = record if isinstance(record, str) else str(record.get("command", ""))
        command_output_text = "" if isinstance(record, str) else str(record.get("output", ""))
        exit_code = None if isinstance(record, str) else record.get("exitCode")
        if command.startswith("INVALID_JSONL_LINE:"):
            flags.append(command); continue
        if command == "FILE_CHANGE_EVENT":
            flags.append("SOURCE_EDIT_ATTEMPT"); continue
        tokens, envelope_flags = unwrap_codex_command(command)
        flags.extend(envelope_flags)
        if not tokens:
            continue
        executable = Path(tokens[0]).name
        successful = (
            exit_code == 0 and bool(command_output_text.strip())
            and not re.search(r"(?im)^\s*(usage:|options:|commands:)", command_output_text)
            and '"schema":"semantic-error/' not in command_output_text.replace(" ", "")
        )
        if executable in {"apply_patch", "rm", "mv", "cp", "tee"} or (
            executable == "sed" and "-i" in tokens[1:]
        ):
            flags.append("SOURCE_EDIT_ATTEMPT")
        if executable in {"ast-index", "rg", "grep", "find", "fd", "sed", "cat", "less", "clew"}:
            navigation += 1
        if arm == "default":
            if not valid_default_argv(tokens):
                flags.append("DISALLOWED_MODE_TOOL" if executable in {"ast-index", "clew"} else "INVALID_TOOL_ARGUMENTS")
            elif exit_code is not None and not successful:
                flags.append("TOOL_CALL_FAILED" if exit_code != 0 else "NON_SUBSTANTIVE_TOOL_OUTPUT")
        elif arm == "ast-index":
            if executable == "ast-index":
                if not valid_ast_argv(tokens):
                    flags.append("INVALID_TOOL_ARGUMENTS")
                elif not successful:
                    flags.append("TOOL_CALL_FAILED" if exit_code != 0 else "NON_SUBSTANTIVE_TOOL_OUTPUT")
                else:
                    used_ast = True
                    ast_evidence += "\n" + command_output_text
            elif executable == "sed" and len(tokens) == 4 and tokens[1] == "-n" and re.fullmatch(r"\d+(?:,\d+)?p", tokens[2]):
                numbers = [int(value) for value in tokens[2][:-1].split(",")]
                span = (numbers[-1] - numbers[0] + 1) if len(numbers) == 2 else 1
                candidate = Path(tokens[3])
                if repository is None:
                    flags.append("FALLBACK_SEARCH")
                else:
                    resolved = (repository / candidate).resolve() if not candidate.is_absolute() else candidate.resolve()
                    relative = resolved.relative_to(repository.resolve()).as_posix() if resolved.is_relative_to(repository.resolve()) else ""
                    if span > 240 or not relative or not resolved.is_file() or relative not in ast_evidence:
                        flags.append("FALLBACK_SEARCH")
            else:
                flags.append("FALLBACK_SEARCH")
        elif arm == "codeclew":
            expected = str(clew_path.resolve()) if clew_path else None
            if expected is None or not tokens or tokens[0] != expected:
                flags.append("FALLBACK_SEARCH"); continue
            if typed_catalog is None:
                flags.append("TYPED_GOAL_CATALOG_MISSING"); continue
            grammar_error, request_record, clew_kind = valid_clew_argv(tokens, expected, selected_compilation, base_revision, typed_catalog)
            if request_record is not None and request_records is not None:
                request_records.append({**request_record, "exitCode":exit_code, "outputSha256":sha_bytes(command_output_text.encode())})
            if grammar_error:
                flags.append(grammar_error)
            elif not successful:
                flags.append("TOOL_CALL_FAILED" if exit_code != 0 else "NON_SUBSTANTIVE_TOOL_OUTPUT")
            else:
                try:
                    payload = json.loads(command_output_text)
                except json.JSONDecodeError:
                    payload = None
                subcommand = tuple(tokens[1:3])
                valid_schema = isinstance(payload, dict) and isinstance(payload.get("schema"), str)
                if subcommand == ("project", "inspect"):
                    valid_schema = valid_schema and payload["schema"] == "semantic-project/0.1"
                else:
                    valid_schema = valid_schema and payload["schema"] == typed_catalog["decisionSchema"] and payload.get("status") in {"BOUND", "AMBIGUOUS", "REFUSED"}
                    if payload.get("status") == "REFUSED" and payload.get("reason") in {"INVALID_GOAL", "SNAPSHOT_MISMATCH"}:
                        valid_schema = False
                if not valid_schema:
                    flags.append("NON_SUBSTANTIVE_TOOL_OUTPUT")
                else:
                    if clew_kind == "TYPED_GOAL":
                        used_clew_proof = True
                        if request_records:
                            request_records[-1]["decision"] = payload
                            request_records[-1]["decisionSha256"] = sha_bytes(compact(payload).encode())
    if arm == "ast-index" and not used_ast: flags.append("AST_INDEX_NOT_USED")
    if arm == "codeclew":
        successful_proofs = [record for record in (request_records or []) if record.get("exitCode") == 0 and isinstance(record.get("decision"), dict)]
        if len(successful_proofs) != 1:
            if len(successful_proofs) > 1:
                flags.append("MULTIPLE_CODECLEW_PROOFS")
            used_clew_proof = False
        if not used_clew_proof:
            flags.append("CODECLEW_PROOF_NOT_USED")
    return sorted(set(flags)), navigation


def validate_binding(value: Any) -> bool:
    return isinstance(value, dict) and set(value) == {"role", "symbol"} and isinstance(value["role"], str) and isinstance(value["symbol"], str) and value["role"] in ROLES and bool(value["symbol"])


def validate_model_output(value: Any) -> list[str]:
    errors = []
    keys = {"schema", "status", "inferredFamily", "goal", "ambiguity", "refusal"}
    if not isinstance(value, dict) or set(value) != keys: return ["MODEL_OUTPUT_KEYS"]
    if value["schema"] != "semantic-editing-e04-model-output/0.1": errors.append("MODEL_OUTPUT_SCHEMA")
    if value["inferredFamily"] not in FAMILIES + ("UNKNOWN",): errors.append("INFERRED_FAMILY")
    status = value["status"]
    if status == "BOUND":
        goal = value["goal"]
        if value["ambiguity"] is not None or value["refusal"] is not None: errors.append("BOUND_EXCLUSIVITY")
        if not isinstance(goal, dict) or set(goal) != {"bindings", "obligations", "evidenceAnchors", "oracleClass"}: errors.append("BOUND_GOAL")
        elif not goal["bindings"] or not all(validate_binding(v) for v in goal["bindings"]) or not goal["obligations"] or not all(isinstance(v, str) and v for v in goal["obligations"]) or not goal["evidenceAnchors"] or not all(isinstance(v, str) and v for v in goal["evidenceAnchors"]) or goal["oracleClass"] not in {"DERIVED", "PARAMETRIC", "MODEL_AUTHORED", "EXTERNAL_SPEC"}: errors.append("BOUND_CONTENT")
    elif status == "AMBIGUOUS":
        ambiguity = value["ambiguity"]
        if value["goal"] is not None or value["refusal"] is not None: errors.append("AMBIGUOUS_EXCLUSIVITY")
        if not isinstance(ambiguity, dict) or set(ambiguity) != {"choices"} or not 2 <= len(ambiguity.get("choices", [])) <= 8: errors.append("AMBIGUITY")
        elif not all(isinstance(c, dict) and set(c) == {"bindings"} and c["bindings"] and all(validate_binding(v) for v in c["bindings"]) for c in ambiguity["choices"]): errors.append("AMBIGUITY_CHOICES")
    elif status == "REFUSED":
        refusal = value["refusal"]
        if value["goal"] is not None or value["ambiguity"] is not None: errors.append("REFUSED_EXCLUSIVITY")
        if not isinstance(refusal, dict) or set(refusal) != {"code"} or refusal["code"] not in REFUSALS: errors.append("REFUSAL")
    else: errors.append("MODEL_STATUS")
    return errors


def validate_proof_model_link(
    model_output: dict[str, Any] | None,
    request_records: list[dict[str, Any]],
    typed_catalog: dict[str, Any],
    refusal_adapter: dict[str, Any],
) -> list[str]:
    successful = [record for record in request_records if record.get("exitCode") == 0 and isinstance(record.get("decision"), dict)]
    if len(successful) != 1 or not isinstance(model_output, dict):
        return ["PROOF_MODEL_LINK_MISSING"]
    record = successful[0]; decision = record["decision"]
    if decision.get("status") != model_output.get("status"):
        return ["PROOF_MODEL_STATUS_MISMATCH"]
    status = decision["status"]
    if status == "BOUND":
        request = json.loads(record["canonicalJson"]); variable_ids = set(request["goal"]["variables"])
        if not variable_ids or not variable_ids <= set(ROLES):
            return ["PROOF_ROLE_VOCABULARY_MISMATCH"]
        proof_bindings = (decision.get("proof") or {}).get("bindings")
        final_items = model_output["goal"]["bindings"] if isinstance(model_output.get("goal"), dict) else None
        final_bindings = {item["role"]:item["symbol"] for item in final_items} if isinstance(final_items, list) else None
        if not isinstance(proof_bindings, dict) or set(proof_bindings) != variable_ids or not isinstance(final_items, list) or len(final_items) != len(final_bindings) or proof_bindings != final_bindings:
            return ["PROOF_MODEL_BINDINGS_MISMATCH"]
    elif status == "AMBIGUOUS":
        def choices(value: Any) -> set[frozenset[tuple[str, str]]]:
            result = set()
            for choice in value or []:
                bindings = choice.get("bindings") if isinstance(choice, dict) else None
                if isinstance(bindings, dict):
                    result.add(frozenset((str(key), str(symbol)) for key, symbol in bindings.items()))
                elif isinstance(bindings, list):
                    result.add(frozenset((str(item["role"]), str(item["symbol"])) for item in bindings))
            return result
        tool_choices = choices(decision.get("choices")); final_choices = choices((model_output.get("ambiguity") or {}).get("choices"))
        if not tool_choices or tool_choices != final_choices:
            return ["PROOF_MODEL_CHOICES_MISMATCH"]
    elif status == "REFUSED":
        product_reason = decision.get("reason")
        expected = refusal_adapter["mapping"].get(product_reason)
        final = (model_output.get("refusal") or {}).get("code")
        if expected is None or final != expected:
            return ["PROOF_MODEL_REFUSAL_MISMATCH"]
    else:
        return ["PROOF_MODEL_STATUS_MISMATCH"]
    return []


def task_prompt(
    spec: dict[str, Any],
    typed_catalog: dict[str, Any],
    public: dict[str, Any],
    arm: str,
    clew: Path | None,
    compilation: str = ":/main",
    test_compilation: str = ":/test",
    state: Path | None = None,
    base_revision: str = "<ISOLATED_HEAD>",
) -> str:
    safe = {key: public[key] for key in PUBLIC_KEYS if key != "controllerManifestCommitment"}
    policy = mode_prompt(arm, clew, compilation, test_compilation, state, base_revision, typed_catalog)
    return f"{common_prompt(spec, typed_catalog)}\n\nARM POLICY:\n{policy}\n\nPUBLIC TASK:\n{compact(safe)}"


def git_status(repository: Path) -> str:
    return command_output(["git", "status", "--porcelain", "--untracked-files=all"], repository)


def external_state_link(repository: Path, relative: Path, target: Path) -> None:
    link = repository / relative
    if link.exists() or link.is_symlink():
        raise RuntimeError(f"state path already exists in source snapshot: {relative}")
    target.mkdir(parents=True, exist_ok=True)
    link.parent.mkdir(parents=True, exist_ok=True)
    link.symlink_to(target, target_is_directory=True)


def initialize_isolated_repository(
    source: Path,
    isolated: Path,
    state: Path | None = None,
    dependency_seed: dict[str, Any] | None = None,
    build_system: str | None = None,
) -> tuple[str, Path, dict[str, str]]:
    shutil.copytree(source, isolated, symlinks=True)
    before = source_digest(isolated)
    commands = (
        ["git", "init", "--quiet"],
        ["git", "add", "--all"],
        ["git", "-c", "user.name=E04 Harness", "-c", "user.email=e04@invalid", "commit", "--quiet", "-m", "E04 isolated snapshot"],
    )
    for command in commands:
        result = subprocess.run(command, cwd=isolated, text=True, capture_output=True, check=False)
        if result.returncode:
            raise RuntimeError(f"isolated git setup failed ({' '.join(command)}): {result.stderr.strip()}")
    if command_output(["git", "status", "--porcelain"], isolated):
        raise RuntimeError("isolated repository is not clean after snapshot commit")
    command_output(["git", "rev-parse", "--verify", "HEAD"], isolated)

    state = state or isolated.parent / "state"
    gradle_home = isolated / ".gradle"
    maven_repo = isolated / ".semantic-thread/maven-repository"
    for directory in (state, state / "tmp"):
        directory.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    environment["AST_INDEX_DB_PATH"] = str(state / "ast-index.db")
    exclude = isolated / ".git/info/exclude"
    with exclude.open("a", encoding="utf-8") as handle:
        handle.write("\n.semantic-thread\n**/.semantic-thread\n.gradle\n**/.gradle\nbuild\n**/build\ntarget\n**/target\n")
    gradle_home.mkdir(parents=True)
    maven_repo.parent.mkdir(parents=True)
    if dependency_seed is not None:
        seed_root = Path(dependency_seed["root"])
        environment["E04_DEPENDENCY_SEED_SHA256"] = dependency_seed["manifestSha256"]
        environment["E04_GRADLE_SEED_TREE_SHA256"] = dependency_seed["gradle"]["treeSha256"]
        environment["E04_GRADLE_WRAPPER_TREE_SHA256"] = dependency_seed["gradleWrapper"]["treeSha256"]
        environment["E04_MAVEN_SEED_TREE_SHA256"] = dependency_seed["maven"]["treeSha256"]
        environment["E04_GRADLE_CACHE_CLONE_SHA256"] = verified_clone_tree(seed_root / "gradle-modules", gradle_home / "caches/modules-2", dependency_seed["gradle"]["treeSha256"])
        environment["E04_GRADLE_WRAPPER_CLONE_SHA256"] = verified_clone_tree(seed_root / "gradle-wrapper-dists", gradle_home / "wrapper/dists", dependency_seed["gradleWrapper"]["treeSha256"])
        source_repo = (seed_root / "maven-repository").resolve()
        environment["E04_MAVEN_CACHE_CLONE_SHA256"] = verified_clone_tree(source_repo, maven_repo, dependency_seed["maven"]["treeSha256"])
    else:
        (gradle_home / "caches/modules-2").mkdir(parents=True)
        (gradle_home / "wrapper/dists").mkdir(parents=True)
        maven_repo.mkdir(parents=True)
    environment["TMPDIR"] = str(state / "tmp")
    for key in CLEW_ENV_DENY:
        environment.pop(key, None)
    for key in list(environment):
        if key.startswith("ORG_GRADLE_PROJECT_"):
            environment.pop(key)
    if git_status(isolated):
        raise RuntimeError("isolated repository is dirty after ignored repository-owned state setup")
    return before, state, environment


def repository_owned_state_report(repository: Path, dependency_seed: dict[str, Any]) -> dict[str, Any]:
    gradle_modules = repository / ".gradle/caches/modules-2"
    gradle_wrapper = repository / ".gradle/wrapper/dists"
    maven_repository = repository / ".semantic-thread/maven-repository"
    current = {
        "gradleModules":tree_sha256(gradle_modules)[0],
        "gradleWrapper":tree_sha256(gradle_wrapper)[0],
        "mavenRepository":tree_sha256(maven_repository)[0],
    }
    return {
        "insideCheckout":True,
        "ignoredByGit":git_status(repository) == "",
        "regularDirectories":all(path.is_dir() and not path.is_symlink() for path in (repository / ".gradle", repository / ".semantic-thread", gradle_modules, gradle_wrapper, maven_repository)),
        "seedCloneSha256":{
            "gradleModules":dependency_seed["gradle"]["treeSha256"],
            "gradleWrapper":dependency_seed["gradleWrapper"]["treeSha256"],
            "mavenRepository":dependency_seed["maven"]["treeSha256"],
        },
        "currentTreeSha256":current,
    }


def parse_gradle_compilations(output: str) -> list[dict[str, str]]:
    found: dict[tuple[str, str], dict[str, str]] = {}
    pattern = re.compile(r"^(?P<task>(?:(?::?[A-Za-z0-9_.-]+):)*compile[A-Za-z0-9_]*Kotlin)\b")
    for raw in output.splitlines():
        match = pattern.match(raw.strip())
        if not match:
            continue
        task = match.group("task")
        pieces = task.lstrip(":").split(":")
        task_name = pieces[-1]
        project = ":" + ":".join(pieces[:-1]) if len(pieces) > 1 else ":"
        middle = task_name.removeprefix("compile").removesuffix("Kotlin")
        source_set = "main" if not middle else middle[0].lower() + middle[1:]
        compilation = f"{project}/{source_set}"
        found[(project, source_set)] = {
            "buildSystem": "GRADLE",
            "projectRoot": ".",
            "projectPath": project,
            "sourceSet": source_set,
            "compilation": compilation,
            "compileTask": (project if project != ":" else "") + ":" + task_name,
        }
    return sorted(found.values(), key=lambda item: (item["projectPath"], item["sourceSet"]))


def gradle_discovery_command(repository: Path) -> list[str]:
    return [
        str(repository / "gradlew"), "--offline", "--gradle-user-home",
        str((repository / ".gradle").resolve()), "--no-daemon", "--console=plain",
        "-q", "tasks", "--all",
    ]


def compilation_discovery_evidence(repository: Path, build_system: str, observations: list[dict[str, Any]]) -> dict[str, str]:
    if build_system.upper() == "GRADLE":
        command = observations[0]["command"]
        return {"method":"GRADLE_WRAPPER_TASKS","inputsSha256":sha_bytes(compact(command).encode())}
    poms = [{"path":str(path.relative_to(repository)),"sha256":sha_file(path)} for path in sorted(repository.rglob("pom.xml"))]
    return {"method":"MAVEN_REACTOR_POMS","inputsSha256":sha_bytes(compact(poms).encode())}


def xml_local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def xml_children(element: ET.Element, name: str) -> list[ET.Element]:
    return [child for child in element if xml_local_name(child.tag) == name]


def xml_child(element: ET.Element, name: str) -> ET.Element | None:
    return next(iter(xml_children(element, name)), None)


def discover_maven_compilations(repository: Path) -> list[dict[str, str]]:
    pending = [repository.resolve()]
    seen: set[Path] = set()
    candidates: list[dict[str, str]] = []
    while pending:
        project = pending.pop(0)
        if project in seen:
            continue
        seen.add(project)
        pom = project / "pom.xml"
        if not pom.is_file():
            raise RuntimeError(f"declared Maven module has no pom.xml: {project}")
        root = ET.parse(pom).getroot()
        modules = xml_child(root, "modules")
        declared = [child.text.strip() for child in xml_children(modules, "module") if child.text and child.text.strip()] if modules is not None else []
        for module in declared:
            resolved = (project / module).resolve()
            if not resolved.is_relative_to(repository.resolve()):
                raise RuntimeError(f"Maven module escapes repository: {module}")
            pending.append(resolved)
        plugins = [element for element in root.iter() if xml_local_name(element.tag) == "plugin"]
        kotlin_plugin = any(
            (xml_child(plugin, "artifactId") is not None and (xml_child(plugin, "artifactId").text or "").strip() == "kotlin-maven-plugin")
            for plugin in plugins
        )
        main_sources = project / "src/main/kotlin"
        if kotlin_plugin and main_sources.is_dir() and any(main_sources.rglob("*.kt")):
            relative = project.relative_to(repository.resolve()).as_posix() or "."
            candidates.append({
                "buildSystem": "MAVEN", "projectRoot": relative, "projectPath": ":",
                "sourceSet": "main", "compilation": ":/main", "compileTask": "compile",
            })
            if (project / "src/test/kotlin").is_dir():
                candidates.append({
                    "buildSystem": "MAVEN", "projectRoot": relative, "projectPath": ":",
                    "sourceSet": "test", "compilation": ":/test", "compileTask": "test-compile",
                })
    return sorted(candidates, key=lambda item: (item["projectRoot"], item["sourceSet"]))


def observed_tool(
    args: list[str],
    cwd: Path,
    environment: dict[str, str],
    authority: Path,
    timeout_seconds: float | None = None,
) -> tuple[subprocess.CompletedProcess[str], dict[str, Any]]:
    before = git_status(authority)
    if before:
        raise RuntimeError(f"authority checkout dirty before tool {' '.join(args)}: {before}")
    try:
        result = subprocess.run(args, cwd=cwd, env=environment, text=True, capture_output=True, check=False, timeout=timeout_seconds)
    except subprocess.TimeoutExpired as error:
        if PREFLIGHT_TIMEBOX_CONTEXT is not None:
            write_json(PREFLIGHT_TIMEBOX_CONTEXT["output"],{"schema":"semantic-editing-e04-preflight/0.2","status":"TIMEBOX_EXCEEDED","modelCalls":0,"tasks":len(PREFLIGHT_TIMEBOX_CONTEXT["rows"]),"completedRows":len(PREFLIGHT_TIMEBOX_CONTEXT["rows"]),"stoppedAt":PREFLIGHT_TIMEBOX_CONTEXT["taskId"],"errors":["TIMEBOX_EXCEEDED"],"rows":PREFLIGHT_TIMEBOX_CONTEXT["rows"]})
        timed_stdout=error.stdout.decode(errors="replace") if isinstance(error.stdout,bytes) else (error.stdout or "")
        timed_stderr=error.stderr.decode(errors="replace") if isinstance(error.stderr,bytes) else (error.stderr or "")
        result = subprocess.CompletedProcess(args,124,timed_stdout,timed_stderr + "\nTIMEBOX_EXCEEDED")
    after = git_status(authority)
    observation = {
        "command": args, "exitCode": result.returncode,
        "checkoutCleanBefore": before == "", "checkoutCleanAfter": after == "",
    }
    if after:
        raise RuntimeError(f"authority checkout dirty after tool {' '.join(args)}: {after}")
    return result, observation


def discover_compilations(
    repository: Path,
    build_system: str,
    environment: dict[str, str],
    timeout_seconds: float | None = None,
) -> tuple[list[dict[str, str]], list[dict[str, Any]]]:
    observations: list[dict[str, Any]] = []
    if build_system.upper() == "GRADLE":
        wrapper = repository / "gradlew"
        if not wrapper.is_file():
            raise RuntimeError("Gradle wrapper is required for compilation discovery")
        result, observation = observed_tool(
            gradle_discovery_command(repository),
            repository, environment, repository,
            timeout_seconds,
        )
        observations.append(observation)
        if result.returncode:
            raise RuntimeError(f"Gradle compilation discovery failed: {(result.stdout + result.stderr)[-2000:]}")
        compilations = parse_gradle_compilations(result.stdout + "\n" + result.stderr)
    elif build_system.upper() == "MAVEN":
        compilations = discover_maven_compilations(repository)
    else:
        raise RuntimeError(f"unsupported build system in public manifest: {build_system}")
    main = [item for item in compilations if item["sourceSet"] == "main"]
    if len(main) != 1:
        raise RuntimeError(f"expected one real Kotlin main compilation, found {len(main)}: {main}")
    return compilations, observations


def prepare_project_state(repository: Path, selected: dict[str, str], state: Path, dependency_seed: dict[str, Any]) -> Path:
    project_root = (repository / selected["projectRoot"]).resolve()
    if not project_root.is_relative_to(repository.resolve()):
        raise RuntimeError("selected project root escapes repository")
    relative_root = project_root.relative_to(repository.resolve())
    if relative_root != Path("."):
        gradle_home = project_root / ".gradle"
        maven_repository = project_root / ".semantic-thread/maven-repository"
        gradle_home.mkdir(parents=True)
        maven_repository.parent.mkdir(parents=True)
        seed_root = Path(dependency_seed["root"])
        verified_clone_tree(seed_root / "gradle-modules", gradle_home / "caches/modules-2", dependency_seed["gradle"]["treeSha256"])
        verified_clone_tree(seed_root / "gradle-wrapper-dists", gradle_home / "wrapper/dists", dependency_seed["gradleWrapper"]["treeSha256"])
        verified_clone_tree(seed_root / "maven-repository", maven_repository, dependency_seed["maven"]["treeSha256"])
    output_name = "build" if selected["buildSystem"] == "GRADLE" else "target"
    external_state_link(repository, relative_root / output_name, state / f"{selected['buildSystem'].lower()}-output")
    if git_status(repository):
        raise RuntimeError("authority checkout dirty after project state externalization")
    return project_root


def model_timeout_fields(timed_out: bool, partial_artifacts_present: bool) -> dict[str,Any]:
    return {"modelTimedOut":timed_out,"timeoutSeconds":MODEL_TIMEOUT_SECONDS if timed_out else None,"partialArtifactsPresent":partial_artifacts_present}


def execute_one(
    row: dict[str, Any],
    experiment: Path,
    output: Path,
    clew: Path,
    dependency_seed: dict[str, Any],
    typed_catalog: dict[str, Any],
    refusal_adapter: dict[str, Any],
) -> dict[str, Any]:
    public_path = Path(row["publicManifest"]); public = load(public_path); spec = population()
    ast_provenance = ast_index_provenance(); require_frozen_ast_index(ast_provenance)
    run_dir = output / "runs" / row["runId"]; run_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="codeclew-e04-") as temporary, anchored_ast_state(Path(temporary)) as ast_state_anchor:
        temporary_root = Path(temporary)
        isolated = temporary_root / "repository"
        before, state, environment = initialize_isolated_repository(
            public_path.parent / "repository", isolated, Path(ast_state_anchor["rootPath"]),
            dependency_seed, public["buildSystem"],
        )
        if before != public["sourceSnapshotSha256"]: raise RuntimeError(f"public source snapshot mismatch for {row['taskId']}")
        compilations, discovery_observations = discover_compilations(isolated, public["buildSystem"], environment)
        selected = next(item for item in compilations if item["sourceSet"] == "main")
        test = next((item for item in compilations if item["sourceSet"] == "test" and item["projectRoot"] == selected["projectRoot"] and item["projectPath"] == selected["projectPath"]), None)
        workspace = prepare_project_state(isolated, selected, state, dependency_seed)
        repository_state_before = repository_owned_state_report(workspace, dependency_seed)
        test_compilation = test["compilation"] if test else selected["compilation"].removesuffix("main") + "test"
        base_revision = command_output(["git", "rev-parse", "HEAD"], isolated)
        prompt = task_prompt(spec, typed_catalog, public, row["arm"], clew, selected["compilation"], test_compilation, state, base_revision)
        prompt_sha = sha_bytes(prompt.encode())
        (run_dir / "prompt.txt").write_text(prompt, encoding="utf-8")
        last = run_dir / "last-message.json"; events_path = run_dir / "events.jsonl"; stderr_path = run_dir / "stderr.txt"
        command = ["codex", "exec", "--ephemeral", "--ignore-user-config", "--skip-git-repo-check", "--json", "--output-schema", str(OUTPUT_SCHEMA), "-s", "workspace-write", "-m", MODEL, "-c", 'model_reasoning_effort="low"', "-C", str(workspace), "-o", str(last), "-"]
        checkout_before = git_status(isolated)
        if checkout_before:
            raise RuntimeError(f"authority checkout dirty before model: {checkout_before}")
        model_environment = sanitized_clew_environment(environment) if row["arm"] == "codeclew" else environment
        started = time.monotonic(); model_timed_out=False
        child=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,env=model_environment,start_new_session=True)
        try:
            stdout,stderr=child.communicate(prompt,timeout=MODEL_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired as timeout:
            model_timed_out=True; os.killpg(child.pid,signal.SIGTERM)
            try: stdout,stderr=child.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid,signal.SIGKILL); stdout,stderr=child.communicate()
            stdout=(timeout.stdout or "")+stdout; stderr=(timeout.stderr or "")+stderr+"\nMODEL_WALL_TIMEOUT"
        process=subprocess.CompletedProcess(command,124 if model_timed_out else child.returncode,stdout,stderr); wall = int((time.monotonic() - started) * 1000)
        if ast_index_provenance() != ast_provenance:
            raise RuntimeError("ast-index executable provenance changed during run")
        checkout_after = git_status(isolated)
        if checkout_after:
            raise RuntimeError(f"authority checkout dirty after model: {checkout_after}")
        events_path.write_text(process.stdout, encoding="utf-8"); stderr_path.write_text(process.stderr, encoding="utf-8")
        after = source_digest(isolated)
        repository_state_after = repository_owned_state_report(workspace, dependency_seed)
        lines = process.stdout.splitlines(); metrics, _, commands = event_metrics(lines)
        request_records: list[dict[str, Any]] = []
        flags, navigation = audit(row["arm"], commands, before, after, clew, workspace, selected["compilation"], base_revision, request_records, typed_catalog)
        for record in request_records:
            record["packetBindingSha256"] = sha_bytes(compact({
                "promptSha256":prompt_sha, "taskId":public["taskId"],
                "publicManifestSha256":row["publicManifestSha256"],
                "requestSha256":record["sha256"],
            }).encode())
    model_output = None; output_errors = []
    try: model_output = load(last); output_errors = validate_model_output(model_output)
    except Exception as error: output_errors = [f"MODEL_OUTPUT_UNREADABLE:{type(error).__name__}"]
    if model_timed_out: output_errors=["MODEL_WALL_TIMEOUT"]
    if row["arm"] == "codeclew":
        flags.extend(validate_proof_model_link(model_output, request_records, typed_catalog, refusal_adapter))
    goal_bytes = len(compact(model_output["goal"]).encode()) if isinstance(model_output, dict) and model_output.get("goal") is not None else 0
    if model_timed_out: flags.append("MODEL_WALL_TIMEOUT")
    exit_code=124 if model_timed_out else process.returncode
    infrastructure_valid = exit_code == 0
    timeout_fields=model_timeout_fields(model_timed_out,events_path.is_file() and stderr_path.is_file())
    packet = {**row, "state": "FINISHED", "exitCode": exit_code, "executionStatus": "MODEL_WALL_TIMEOUT" if model_timed_out else ("OK" if infrastructure_valid else "FAILED"), **timeout_fields,"infrastructureValid": infrastructure_valid, "modelOutputValid":not output_errors, "modelOutputErrors":output_errors, "protocolValid": not flags, "wallMilliseconds": wall, "promptBytes": len(prompt.encode()), "promptSha256":prompt_sha, "contextBytes": len(prompt.encode()) + metrics["toolOutputBytes"], "goalBytes": goal_bytes, "navigationCalls": navigation, "auditFlags": sorted(set(flags)), "metrics": metrics, "modelOutput": model_output, "artifacts": {"eventsJsonl": str(events_path), "stderr": str(stderr_path), "lastMessage": str(last)}, "sourceBeforeSha256": before, "sourceAfterSha256": after, "discoveredCompilations": compilations, "selectedCompilation": selected, "testCompilation": test_compilation, "typedGoalRequests":request_records, "typedGoalCatalogSha256":typed_catalog["catalogSha256"], "refusalAdapterSha256":refusal_adapter["adapterSha256"], "astIndexExecutable":ast_provenance, "astStateAnchor":{"rootName":ast_state_anchor["rootName"], "parentIdentity":list(ast_state_anchor["parentIdentity"]), "rootIdentity":list(ast_state_anchor["rootIdentity"])}, "controllerManifestCommitment":public["controllerManifestCommitment"], "projectDiscoveryTools": discovery_observations, "checkoutCleanBeforeModel": checkout_before == "", "checkoutCleanAfterModel": checkout_after == "", "dependencySeedManifestSha256": dependency_seed["manifestSha256"], "externalResultsStateRootOutsideCheckout":not state.resolve().is_relative_to(isolated.resolve()), "repositoryOwnedMutableStateBefore":repository_state_before, "repositoryOwnedMutableStateAfter":repository_state_after}
    write_json(run_dir / "run-packet.json", packet)
    return packet


def preflight_selection(tasks: list[tuple[Path, dict[str, Any]]], max_tasks: int) -> tuple[list[tuple[Path, dict[str, Any]]], dict[str, int]]:
    selected = tasks[:max(1, max_tasks)] if max_tasks else tasks
    counts = {"GRADLE":0,"MAVEN":0}
    for _, public in selected:
        build = public.get("buildSystem")
        if not isinstance(build, str) or build.upper() not in counts:
            raise RuntimeError(f"preflight selection has unknown build system: {build!r}")
        counts[build.upper()] += 1
    return selected, counts


def preflight_aggregate_errors(
    rows: list[dict[str, Any]],
    expected_build_counts: dict[str, int],
    provenance_stable: bool,
) -> list[str]:
    actual = {build:sum(row.get("buildSystem") == build for row in rows) for build in ("GRADLE","MAVEN")}
    errors = []
    if actual != expected_build_counts:
        errors.append(f"BUILD_LAYOUT_DENOMINATOR:{actual}!={expected_build_counts}")
    if len(rows) != sum(expected_build_counts.values()):
        errors.append(f"COMPLETED_ROW_DENOMINATOR:{len(rows)}!={sum(expected_build_counts.values())}")
    if not provenance_stable:
        errors.append("FROZEN_PROVENANCE_CHANGED")
    if not all(row.get("infrastructureValid") is True for row in rows):
        errors.append("INFRASTRUCTURE_NOT_READY")
    if not all(row.get("astReady") is True for row in rows):
        errors.append("AST_NOT_READY")
    if not all(row.get("codeclewProjectReady") is True for row in rows):
        errors.append("CODECLEW_NOT_READY")
    return errors


def publish_preflight_report(output: Path, report: dict[str, Any], aggregate_errors: list[str]) -> dict[str, Any]:
    published = {
        **report,
        "status":"PREFLIGHT_PASSED" if not aggregate_errors else "PREFLIGHT_POSTCONDITION_FAILED",
        "aggregatePostconditionErrors":list(aggregate_errors),
    }
    write_canonical_json(output, published)
    if aggregate_errors:
        raise RuntimeError(f"preflight aggregate postcondition failed: {aggregate_errors}; inspect {output}")
    return published


def publish_preflight_row_failure(output: Path, rows: list[dict[str, Any]], task_id: str, errors: list[str]) -> dict[str, Any]:
    packet = {
        "schema":"semantic-editing-e04-preflight/0.2", "status":"PREFLIGHT_ROW_FAILED",
        "modelCalls":0, "tasks":len(rows), "completedRows":len(rows),
        "stoppedAt":task_id, "errors":list(errors), "rows":rows,
    }
    write_json(output, packet)
    return packet


@contextlib.contextmanager
def preflight_row_failure_guard(args: argparse.Namespace, output: Path, experiment: Path, clew: Path, rows: list[dict[str,Any]], public: dict[str,Any], readiness_gate: dict[str,Any], dependency_seed: dict[str,Any], typed_catalog: dict[str,Any], ast_provenance: dict[str,Any], started: float):
    stage=["COPY_SNAPSHOT"]
    try:
        yield lambda value: stage.__setitem__(0,value)
    except Exception as error:
        retained=load(output) if output.is_file() else None
        preserved=isinstance(retained,dict) and retained.get("status") in {"TIMEBOX_EXCEEDED","PREFLIGHT_ROW_FAILED"} and retained.get("stoppedAt")==public["taskId"]
        packet=dict(retained) if preserved else {"schema":"semantic-editing-e04-preflight/0.2","status":"PREFLIGHT_ROW_FAILED","modelCalls":0,"tasks":len(rows),"completedRows":len(rows),"stoppedAt":public["taskId"],"rows":rows}
        packet.update({"stage":stage[0],"errorCode":type(error).__name__,"errorDetailSha256":sha_bytes(str(error).encode()),"wallMilliseconds":int((time.monotonic()-started)*1000),"controllerReads":0,"diagnosticFreezeArtifactHash":readiness_gate["diagnosticFreezeArtifactHash"],"readinessRootReceiptHash":readiness_gate["receiptHash"],"codeclewBinarySha256":typed_catalog["binarySha256"],"typedGoalCatalogSha256":typed_catalog["catalogSha256"],"dependencySeedManifestSha256":dependency_seed["manifestSha256"],"astIndexBinarySha256":ast_provenance["binarySha256"]})
        write_canonical_json(output,packet); packet_sha=sha_file(output)
        readiness.issue_failed_preflight(self_module(),Path(args.readiness_graph),Path(args.readiness_store),packet,packet_sha,experiment,clew,Path(args.dependency_seed),Path(args.semantic_corpus_bin))
        raise


def preflight(args: argparse.Namespace) -> None:
    """Exercise repository/tool setup without a model or controller labels."""
    global PREFLIGHT_TIMEBOX_CONTEXT
    experiment, output = Path(args.experiment_root), Path(args.output)
    setup_started=time.monotonic()
    try:
        if getattr(args,"no_freeze_check",False): raise RuntimeError("FULL_PREFLIGHT_42 forbids --no-freeze-check")
        clew = Path(args.codeclew_bin)
        if not clew.is_absolute() or not clew.is_file(): raise RuntimeError("--codeclew-bin must be an existing absolute binary")
        if int(args.max_tasks or 0): raise RuntimeError("FULL_PREFLIGHT_42 forbids --max-tasks")
        if getattr(args,"readiness_root",None)!="DIAGNOSTIC_FULL_PREFLIGHT_START_READY" or not getattr(args,"readiness_store",None): raise RuntimeError("preflight requires DIAGNOSTIC_FULL_PREFLIGHT_START_READY")
        readiness_gate=readiness.require_root(self_module(),Path(args.readiness_graph),Path(args.readiness_store),args.readiness_root,experiment,clew,Path(args.dependency_seed),semantic_corpus=Path(args.semantic_corpus_bin))
        dependency_seed = validate_dependency_seed(Path(args.dependency_seed))
        typed_catalog = load_typed_goal_catalog(clew.resolve()); refusal_adapter = load_refusal_adapter(typed_catalog)
        ast_provenance = ast_index_provenance(); require_frozen_ast_index(ast_provenance)
        freeze_path=Path(readiness_gate["diagnosticFreezeArtifact"])
        if freeze_path.is_file():
            frozen = load(freeze_path)
            if frozen.get("codeclewBinarySha256") != typed_catalog["binarySha256"] or frozen.get("typedGoalCatalogSha256") != typed_catalog["catalogSha256"] or frozen.get("refusalAdapterSha256") != refusal_adapter["adapterSha256"]: raise RuntimeError("preflight Codeclew binary/catalog does not match freeze provenance")
        frozen_checks(True,True,dependency_seed,typed_catalog,refusal_adapter,freeze_path)
        all_tasks = discover_public(experiment)
    except Exception as error:
        write_json(output,{"schema":"semantic-editing-e04-preflight/0.2","status":"PREFLIGHT_SETUP_FAILED","modelCalls":0,"tasks":0,"completedRows":0,"stoppedAt":"SETUP","errors":[f"{type(error).__name__}:{error}"],"rows":[],"wallMilliseconds":int((time.monotonic()-setup_started)*1000)})
        raise
    tasks, expected_build_counts = preflight_selection(all_tasks, int(args.max_tasks or 0))
    rows = []; started_all = time.monotonic()
    deadline=time.monotonic()+float(getattr(args,"deadline_seconds",2700) or 2700)
    start_binary_sha = typed_catalog["binarySha256"]
    start_catalog_sha = typed_catalog["catalogSha256"]
    start_seed_sha = dependency_seed["manifestSha256"]
    for public_path, public in tasks:
        PREFLIGHT_TIMEBOX_CONTEXT={"output":output,"rows":rows,"taskId":public["taskId"]}
        task_started = time.monotonic()
        with preflight_row_failure_guard(args,output,experiment,clew,rows,public,readiness_gate,dependency_seed,typed_catalog,ast_provenance,started_all) as set_stage, tempfile.TemporaryDirectory(prefix="codeclew-e04-preflight-") as temporary, anchored_ast_state(Path(temporary)) as ast_state_anchor:
            public_build = public.get("buildSystem")
            if not isinstance(public_build, str) or public_build.upper() not in {"GRADLE", "MAVEN"}:
                raise RuntimeError(f"preflight unknown build system for {public.get('taskId')}: {public_build!r}")
            build_system = public_build.upper()
            if time.monotonic() >= deadline:
                packet={"schema":"semantic-editing-e04-preflight/0.2","status":"TIMEBOX_EXCEEDED","modelCalls":0,"tasks":len(rows),"completedRows":len(rows),"stoppedAt":public["taskId"],"errors":["TIMEBOX_EXCEEDED"],"rows":rows}
                write_json(output,packet); raise RuntimeError(f"preflight TIMEBOX_EXCEEDED; inspect {output}")
            temporary_root = Path(temporary)
            isolated = temporary_root / "repository"
            anchored_state = Path(ast_state_anchor["rootPath"])
            before, state, environment = initialize_isolated_repository(
                public_path.parent / "repository", isolated, anchored_state,
                dependency_seed, public["buildSystem"],
            )
            if before != public["sourceSnapshotSha256"]:
                raise RuntimeError(f"preflight source snapshot mismatch for {public['taskId']}")
            checkout_clean_before = git_status(isolated) == ""
            set_stage("COMPILATION_DISCOVERY")
            remaining=max(0.001,deadline-time.monotonic())
            compilations, observations = discover_compilations(isolated, public["buildSystem"], environment, remaining)
            compilation_ids = [item["compilation"] for item in compilations]
            if len(compilation_ids) != len(set(compilation_ids)) or not all(item["buildSystem"] == build_system for item in compilations):
                raise RuntimeError(f"preflight exact compilation discovery failed for {public['taskId']} ({public['buildSystem']}): {compilations}")
            set_stage("COMPILATION_SELECTION")
            selected = next(item for item in compilations if item["sourceSet"] == "main")
            matching_tests = [item for item in compilations if item["sourceSet"] == "test" and item["projectRoot"] == selected["projectRoot"] and item["projectPath"] == selected["projectPath"]]
            if len(matching_tests) != 1:
                raise RuntimeError(f"preflight expected one matching Kotlin test compilation for {public['taskId']} ({public['buildSystem']}): {compilations}")
            set_stage("PROJECT_STATE"); workspace = prepare_project_state(isolated, selected, state, dependency_seed)
            set_stage("AST_REBUILD")
            ast, ast_observation = observed_tool(
                [ast_provenance["realPath"], "rebuild", "--format", "json"], workspace, environment, isolated,
                max(0.001,deadline-time.monotonic()),
            )
            set_stage("AST_STATS"); ast_stats, ast_stats_observation = observed_tool(
                [ast_provenance["realPath"], "stats", "--format", "json"], workspace, environment, isolated,
                max(0.001,deadline-time.monotonic()),
            )
            ast_db_path = Path(environment["AST_INDEX_DB_PATH"])
            set_stage("AST_ATTESTATION"); ast_readiness = attest_ast_db_artifact(
                parse_ast_readiness(ast_stats.stdout, ast_db_path), ast_db_path, ast_state_anchor,
            )
            set_stage("CODECLEW_PROJECT_INSPECT"); project, project_observation = observed_tool(
                [str(clew.resolve()), "project", "inspect", "--repo", ".", "--compilation", selected["compilation"]],
                workspace, sanitized_clew_environment(environment), isolated,
                max(0.001,deadline-time.monotonic()),
            )
            after = source_digest(isolated)
            project_text = (project.stdout + "\n" + project.stderr).strip()
            infra_markers = ("Operation not permitted", "Permission denied", "No such file or directory", "must have a committed Git HEAD")
            network_markers = ("Could not GET", "Could not HEAD", "Could not resolve", "UnknownHostException", "Connection refused")
            set_stage("CODECLEW_PARSE")
            try:
                project_payload = json.loads(project.stdout)
            except json.JSONDecodeError:
                project_payload = None
            supported_project = (
                project.returncode == 0 and isinstance(project_payload, dict)
                and project_payload.get("schema") == "semantic-project/0.1"
                and project_payload.get("sourceSet") == selected["sourceSet"]
            )
            product_unsupported = (
                project.returncode != 0 and isinstance(project_payload, dict)
                and project_payload.get("schema") == "semantic-error/0.1"
                and (project_payload.get("error") or {}).get("code") == "UNSUPPORTED_PROJECT_CONFIGURATION"
            )
            ast_text = (ast.stdout + "\n" + ast.stderr).strip()
            set_stage("STATE_REVALIDATION"); revalidated_ast_readiness = attest_ast_db_artifact(
                ast_readiness, ast_db_path, ast_state_anchor, ast_readiness["dbSha256"],
            )
            if revalidated_ast_readiness != ast_readiness:
                raise RuntimeError("AST_DB_REVALIDATION_CHANGED")
            ast_substantive = ast.returncode == 0 and ast_stats.returncode == 0
            external_state = not state.resolve().is_relative_to(isolated.resolve())
            repository_state = repository_owned_state_report(workspace, dependency_seed)
            offline = (
                not any(key in environment for key in CLEW_ENV_DENY)
                and not any(key.startswith("ORG_GRADLE_PROJECT_") for key in environment)
                and repository_state["regularDirectories"]
                and repository_state["ignoredByGit"]
                and (build_system != "GRADLE" or observations[0]["command"][1] == "--offline")
            )
            clean_after = git_status(isolated) == ""
            set_stage("ROW_CONSTRUCTION"); row = {
                "taskId": public["taskId"],
                "publicManifestSha256":sha_file(public_path),
                "publicSourceSnapshotSha256":public["sourceSnapshotSha256"],
                "sourceBeforeSha256":before,
                "sourceAfterSha256":after,
                "buildSystem":build_system,
                "gitHead": command_output(["git", "rev-parse", "HEAD"], isolated),
                "projectRoot": selected["projectRoot"],
                "discoveredCompilations": compilations,
                "selectedCompilation": selected,
                "compilationDiscoveryEvidence":compilation_discovery_evidence(isolated,build_system,observations),
                "sourceStable": before == after,
                "checkoutCleanBeforeAllTools":checkout_clean_before,
                "checkoutCleanAfterAllTools":clean_after,
                "toolCleanliness": observations + [ast_observation, ast_stats_observation, project_observation],
                "stateRootOutsideCheckout":external_state,
                "externalResultsStateRootOutsideCheckout":external_state,
                "repositoryOwnedMutableState":repository_state,
                "dependencySeedManifestSha256": dependency_seed["manifestSha256"],
                "typedGoalCatalogSha256":start_catalog_sha,
                "codeclewBinarySha256":start_binary_sha,
                "astIndexExecutable":ast_provenance,
                "astRebuildStdoutSha256":sha_bytes(ast.stdout.encode()),
                "astStatsStdoutSha256":sha_bytes(ast_stats.stdout.encode()),
                "astReadinessSummary":ast_readiness,
                "astDbSha256":ast_readiness["dbSha256"],
                "astDbActualSizeBytes":ast_readiness["actualDbSizeBytes"],
                "astStateAnchor":{"rootName":ast_state_anchor["rootName"], "parentIdentity":list(ast_state_anchor["parentIdentity"]), "rootIdentity":list(ast_state_anchor["rootIdentity"])},
                "offlineHermetic":offline and not any(marker in project_text + ast_text for marker in network_markers),
                "astReady":ast_substantive,
                "codeclewProjectReady":supported_project or product_unsupported,
                "projectSchemaValid":supported_project or product_unsupported,
                "projectRequestCompilation":selected["compilation"],
                "infrastructureValid":checkout_clean_before and before == after and clean_after and external_state and offline and ast_substantive and (supported_project or product_unsupported) and not any(marker in project_text for marker in infra_markers + network_markers),
                "productUnsupported":product_unsupported,
                "astExitCode": ast.returncode,
                "astStatsExitCode":ast_stats.returncode,
                "codeclewExitCode": project.returncode,
                "codeclewDiagnostic": project_text[:2000],
                "wallMilliseconds":int((time.monotonic() - task_started) * 1000),
            }
            rows.append(row)
            if not row["infrastructureValid"]:
                failed = [key for key in ("sourceStable","checkoutCleanBeforeAllTools","checkoutCleanAfterAllTools","stateRootOutsideCheckout","offlineHermetic","astReady","codeclewProjectReady","projectSchemaValid") if not row.get(key)]
                if time.monotonic() >= deadline or 124 in (row["astExitCode"],row["astStatsExitCode"],row["codeclewExitCode"]):
                    write_json(output,{"schema":"semantic-editing-e04-preflight/0.2","status":"TIMEBOX_EXCEEDED","modelCalls":0,"tasks":len(rows),"completedRows":len(rows),"stoppedAt":public["taskId"],"errors":["TIMEBOX_EXCEEDED"],"rows":rows})
                    raise RuntimeError(f"preflight TIMEBOX_EXCEEDED; inspect {output}")
                publish_preflight_row_failure(output, rows, public["taskId"], failed)
                raise RuntimeError(f"preflight invariant failed at {public['taskId']} ({public['buildSystem']}): {failed}; inspect {output}")
    provenance_error = None
    try:
        end_catalog = load_typed_goal_catalog(clew.resolve())
        end_seed = validate_dependency_seed(Path(args.dependency_seed))
        end_ast_provenance = ast_index_provenance()
        require_frozen_ast_index(end_ast_provenance)
        provenance_stable = sha_file(clew.resolve()) == start_binary_sha and end_catalog["catalogSha256"] == start_catalog_sha and end_seed["manifestSha256"] == start_seed_sha and end_ast_provenance == ast_provenance
    except Exception as error:
        provenance_stable = False; provenance_error = f"{type(error).__name__}:{error}"
    build_counts = {build:sum(row["buildSystem"] == build for row in rows) for build in ("GRADLE","MAVEN")}
    aggregate_errors = preflight_aggregate_errors(rows, expected_build_counts, provenance_stable)
    report = {
        "schema": "semantic-editing-e04-preflight/0.2",
        "modelCalls": 0,
        "dependencySeed": dependency_seed,
        "typedGoalCatalog": {"catalogSha256":typed_catalog["catalogSha256"], "binarySha256":typed_catalog["binarySha256"]},
        "astIndexExecutable":ast_provenance,
        "refusalAdapterSha256":refusal_adapter["adapterSha256"],
        "diagnosticFreezeArtifactHash":readiness_gate["diagnosticFreezeArtifactHash"],
        "readinessRootReceiptHash":readiness_gate["receiptHash"],
        "tasks": len(rows),
        "allInfrastructureValid": all(row["infrastructureValid"] for row in rows),
        "allAstReady": all(row["astReady"] for row in rows),
        "allCodeclewReady": all(row["codeclewProjectReady"] for row in rows),
        "buildCounts":build_counts,
        "expectedBuildCounts":expected_build_counts,
        "selectedTaskIds":[public["taskId"] for _, public in tasks],
        "provenanceError":provenance_error,
        "productUnsupported":sum(row["productUnsupported"] for row in rows),
        "wallMilliseconds":int((time.monotonic() - started_all) * 1000),
        "rows": rows,
    }
    report = publish_preflight_report(output, report, aggregate_errors)
    diagnostic_sha=sha_file(output)
    if load(output)!=report:
        raise RuntimeError("preflight diagnostic report changed before readiness issuance")
    readiness.issue_full_preflight(self_module(),Path(args.readiness_graph),Path(args.readiness_store),output,report,diagnostic_sha,experiment,clew,Path(args.dependency_seed),Path(args.semantic_corpus_bin))
    PREFLIGHT_TIMEBOX_CONTEXT=None
    print(compact({key: report[key] for key in ("modelCalls", "tasks", "allInfrastructureValid", "allAstReady", "allCodeclewReady")}))


def circuit_breaker_packet(row: dict[str, Any], reason: str) -> dict[str, Any]:
    return {
        **row, "state": "SKIPPED", "exitCode": None, "executionStatus": "CIRCUIT_BREAKER",
        "infrastructureValid": False, "modelOutputValid": False,
        "modelOutputErrors": ["TRIPLET_CIRCUIT_BREAKER"], "protocolValid": False,
        "wallMilliseconds": 0, "promptBytes": 0, "contextBytes": 0, "goalBytes": 0,
        "navigationCalls": 0, "auditFlags": [f"TRIPLET_CIRCUIT_BREAKER:{reason}"],
        "metrics": {"turns":0,"actionCalls":0,"toolOutputBytes":0,"inputTokens":None,"cachedInputTokens":None,"outputTokens":None,"noncachedTokens":None,"nativeTokenTelemetryAvailable":False},
        "modelOutput": None,
    }


def r7_breaker_reasons(packet: dict[str, Any], expected_commitment: str | None = None) -> list[str]:
    reasons: list[str] = []
    metrics = packet.get("metrics") or {}; flags = set(packet.get("auditFlags") or [])
    if not metrics.get("nativeTokenTelemetryAvailable"):
        reasons.append("NATIVE_TELEMETRY_MISSING")
    if packet.get("sourceBeforeSha256") != packet.get("sourceAfterSha256") or "SOURCE_MUTATION" in flags:
        reasons.append("SOURCE_MUTATION")
    if not packet.get("modelOutputValid"):
        reasons.append("MODEL_SCHEMA_INVALID")
    if not packet.get("protocolValid"):
        reasons.append("PROTOCOL_INVALID")
    arm = packet.get("arm")
    unused = {"ast-index":"AST_INDEX_NOT_USED", "codeclew":"CODECLEW_PROOF_NOT_USED"}.get(arm)
    if packet.get("navigationCalls", 0) < 1 or (unused and unused in flags) or flags & {"TOOL_CALL_FAILED", "NON_SUBSTANTIVE_TOOL_OUTPUT", "INVALID_TOOL_ARGUMENTS"}:
        reasons.append("SPECIALIZED_TOOL_NOT_PROVEN")
    if not packet.get("infrastructureValid") or any(
        marker in str(packet.get("error", "")) + str(packet.get("stderr", ""))
        for marker in ("Permission denied", "No such file or directory", "committed Git HEAD", "Could not GET", "Could not resolve")
    ):
        reasons.append("INFRASTRUCTURE_INVALID")
    commitment = packet.get("controllerManifestCommitment")
    if not commitment or (expected_commitment is not None and commitment != expected_commitment):
        reasons.append("CONTROLLER_COMMITMENT_MISMATCH")
    if packet.get("arm") == "codeclew":
        requests = packet.get("typedGoalRequests") or []
        proven = False
        for request in requests:
            expected_binding = sha_bytes(compact({
                "promptSha256":packet.get("promptSha256"), "taskId":packet.get("taskId"),
                "publicManifestSha256":packet.get("publicManifestSha256"),
                "requestSha256":request.get("sha256"),
            }).encode())
            if request.get("exitCode") == 0 and request.get("packetBindingSha256") == expected_binding:
                proven = True
        if not proven:
            reasons.append("REQUEST_NOT_PACKET_BOUND")
    return sorted(set(reasons))


def r7_canary_reasons(packet: dict[str, Any], expected_commitment: str | None = None) -> list[str]:
    reasons = r7_breaker_reasons(packet, expected_commitment)
    metrics = packet.get("metrics") or {}
    if (metrics.get("turns") or 0) > R7_MAX_TURNS:
        reasons.append("TURN_CEILING_EXCEEDED")
    if (metrics.get("actionCalls") or 0) > R7_MAX_ACTION_CALLS:
        reasons.append("ACTION_CEILING_EXCEEDED")
    if (packet.get("contextBytes") or 0) > R7_MAX_CONTEXT_BYTES:
        reasons.append("CONTEXT_CEILING_EXCEEDED")
    if (packet.get("goalBytes") or 0) > R7_MAX_GOAL_BYTES:
        reasons.append("GOAL_CEILING_EXCEEDED")
    return sorted(set(reasons))


def execute_triplet(rows: list[dict[str, Any]], executor: Any, breaker: Any = None, persist: Any = None) -> list[dict[str, Any]]:
    if not rows or len({row["taskId"] for row in rows}) != 1 or len({row["arm"] for row in rows}) != len(rows):
        raise RuntimeError("circuit breaker requires one task with unique arms")
    packets: list[dict[str, Any]] = []; tripped: str | None = None
    for row in sorted(rows, key=lambda item: item["taskArmOrder"]):
        if tripped is not None:
            packet=circuit_breaker_packet(row, tripped); packets.append(packet)
            if persist is not None: persist(packet)
            continue
        try:
            packet = executor(row)
        except Exception as error:
            packet = {**circuit_breaker_packet(row, f"RUNNER_FAILURE:{type(error).__name__}"), "state":"FINISHED", "executionStatus":"FAILED", "error":str(error)}
        packets.append(packet)
        if persist is not None: persist(packet)
        reasons = breaker(packet) if breaker else ([] if packet.get("infrastructureValid", False) else [packet.get("executionStatus", "INFRASTRUCTURE_INVALID")])
        if reasons:
            tripped = "+".join(reasons)
    return packets


def preregistered_canary_triplets(grouped: dict[str, list[dict[str, Any]]]) -> list[list[dict[str, Any]]]:
    if not grouped: raise RuntimeError("diagnostic canary population is empty")
    task_id=min(grouped)
    rows=grouped[task_id]
    if len(rows)!=3 or {row["arm"] for row in rows}!=set(ARMS): raise RuntimeError("diagnostic canary requires one complete three-arm triplet")
    return [rows]


def preregistered_r1_triplets(grouped: dict[str,list[dict[str,Any]]]) -> list[list[dict[str,Any]]]:
    selected=[]
    for build_system in ("GRADLE","MAVEN"):
        candidates=[(task_id,rows) for task_id,rows in grouped.items() if load(Path(rows[0]["publicManifest"]))["buildSystem"].upper()==build_system]
        if not candidates: raise RuntimeError(f"R1 matrix has no {build_system} circuit-breaker task")
        selected.append(min(candidates,key=lambda item:item[0])[1])
    return selected


def validate_retained_canaries(
    canary_triplets: list[list[dict[str, Any]]],
    retained_packets: list[dict[str, Any]],
) -> dict[str, list[str]]:
    by_run = {packet["runId"]:packet for packet in retained_packets}
    expected_ids = {row["runId"] for triplet in canary_triplets for row in triplet}
    if set(by_run) != expected_ids:
        missing = sorted(expected_ids - set(by_run)); extra = sorted(set(by_run) - expected_ids)
        raise RuntimeError(f"retained R7 canaries are incomplete: missing={missing}, extra={extra}")
    failures: dict[str, list[str]] = {}
    for triplet in canary_triplets:
        public = load(Path(triplet[0]["publicManifest"]))
        commitment = public["controllerManifestCommitment"]
        for row in triplet:
            reasons = r7_canary_reasons(by_run[row["runId"]], commitment)
            if reasons:
                failures[row["runId"]] = reasons
    noncached = sum(((packet.get("metrics") or {}).get("noncachedTokens") or 0) for packet in retained_packets)
    actions = sum(((packet.get("metrics") or {}).get("actionCalls") or 0) for packet in retained_packets)
    aggregate = []
    if noncached > R7_CANARY_MAX_NONCACHED_TOKENS:
        aggregate.append("AGGREGATE_NONCACHED_TOKEN_CEILING_EXCEEDED")
    if actions > R7_CANARY_MAX_ACTION_CALLS:
        aggregate.append("AGGREGATE_ACTION_CEILING_EXCEEDED")
    if aggregate:
        failures["__aggregate__"] = aggregate
    return failures


def typed_goal_composition_id(request: dict[str, Any]) -> str:
    goal = request["goal"]
    operators = sorted(goal["operators"], key=compact)
    domains = {variable:goal["variables"][variable] for variable in sorted(goal["variables"])}
    return sha_bytes(compact({"operators":operators, "domains":domains}).encode())


def coverage_product_paths(product_root: Path) -> list[str]:
    tracked = command_output(["git", "ls-files"], product_root).splitlines()
    selected = []
    for path in tracked:
        if path in {"Cargo.toml", "Cargo.lock", "build.gradle.kts", "settings.gradle.kts"}:
            selected.append(path)
        elif path.startswith("crates/clew/") or path.startswith("schemas/"):
            selected.append(path)
        elif re.fullmatch(r"workers/kotlin[^/]*/build\.gradle\.kts", path) or re.match(r"workers/kotlin[^/]*/src/main/", path):
            selected.append(path)
    if not selected:
        raise RuntimeError("coverage product contour is empty")
    return sorted(selected)


def coverage_packet_digest_fields(
    binary: Path,
    catalog: dict[str, Any],
    product_root: Path,
) -> dict[str, Any]:
    product_paths = coverage_product_paths(product_root)
    if not product_paths or len(set(product_paths)) != len(product_paths):
        raise RuntimeError("coverage product path set is empty or duplicated")
    product_root = product_root.resolve()
    for relative in product_paths:
        path = (product_root / relative).resolve()
        if Path(relative).is_absolute() or not path.is_relative_to(product_root) or not path.is_file():
            raise RuntimeError(f"invalid coverage product path: {relative}")
    path_digests = {path:sha_file(product_root / path) for path in sorted(product_paths)}
    return {
        "productBinarySha256":sha_file(binary), "productCatalogSha256":catalog["catalogSha256"],
        "productPathDigests":path_digests,
        "productPathsSha256":sha_bytes(compact(path_digests).encode()),
    }


def coverage_expected_bindings(controller: dict[str, Any]) -> dict[str, str]:
    return dict(binding.split("=", 1) for binding in controller.get("requiredBindings", []))


def controller_commitment_sha(controller: dict[str, Any]) -> str:
    stable = dict(controller)
    stable["publicManifestSha256"] = ""
    stable["commitment"] = ""
    # semantic-corpus computes this with serde_json::to_vec on the ordered
    # ControllerTask fields; json.load preserves that serialized field order.
    rendered = json.dumps(stable, ensure_ascii=False, separators=(",", ":"))
    return sha_bytes(rendered.encode())


def validate_coverage_decision(value: Any, catalog: dict[str, Any]) -> str | None:
    if not isinstance(value, dict) or value.get("schema") != catalog["decisionSchema"]:
        return "invalid decision schema"
    status = value.get("status")
    if status == "BOUND":
        if set(value) != {"schema", "status", "proof"} or not isinstance(value["proof"], dict):
            return "invalid BOUND decision"
        bindings = value["proof"].get("bindings")
        if not isinstance(bindings, dict) or not bindings or not all(isinstance(key, str) and key and isinstance(symbol, str) and symbol for key, symbol in bindings.items()):
            return "invalid BOUND bindings"
    elif status == "AMBIGUOUS":
        if set(value) != {"schema", "status", "choices"} or not isinstance(value["choices"], list) or not value["choices"]:
            return "invalid AMBIGUOUS decision"
        for choice in value["choices"]:
            if not isinstance(choice, dict) or set(choice) != {"bindings"} or not isinstance(choice["bindings"], dict) or not choice["bindings"]:
                return "invalid AMBIGUOUS choices"
            if not all(isinstance(key, str) and key and isinstance(symbol, str) and symbol for key, symbol in choice["bindings"].items()):
                return "invalid AMBIGUOUS bindings"
    elif status == "REFUSED":
        if set(value) not in ({"schema", "status", "reason"}, {"schema", "status", "reason", "rejections"}) or value.get("reason") not in catalog["productRefusalReasons"]:
            return "invalid REFUSED decision"
        if "rejections" in value and not isinstance(value["rejections"], list):
            return "invalid REFUSED rejections"
    else:
        return "invalid decision status"
    return None


def make_zero_model_coverage_packet(
    packet_id: str,
    task_id: str,
    compilation: str,
    base_revision: str,
    request: dict[str, Any],
    decision: dict[str, Any],
    controller_commitment: str,
    fixture_root: Path,
    provenance: dict[str, Any],
) -> dict[str, Any]:
    canonical_request, canonical_decision = compact(request), compact(decision)
    return {
        "schema":"e04-zero-model-coverage-packet/0.1", "packetId":packet_id,
        "taskId":task_id, "compilation":compilation, "baseRevision":base_revision,
        "canonicalRequest":canonical_request,
        "requestSha256":sha_bytes(canonical_request.encode()),
        "canonicalDecision":canonical_decision,
        "decisionSha256":sha_bytes(canonical_decision.encode()),
        "compositionId":typed_goal_composition_id(request),
        "controllerCommitment":controller_commitment,
        "fixtureSourceSha256":source_digest(fixture_root),
        **provenance,
    }


def guard_zero_model_coverage(
    packets: list[dict[str, Any]],
    experiment: Path,
    binary: Path,
    product_root: Path,
) -> dict[str, Any]:
    catalog = load_typed_goal_catalog(binary.resolve()); adapter = load_refusal_adapter(catalog)
    required_product_paths = coverage_product_paths(product_root)
    actual_digests = coverage_packet_digest_fields(binary, catalog, product_root)
    if not packets or len({packet.get("packetId") for packet in packets}) != len(packets) or len({packet.get("taskId") for packet in packets}) != len(packets):
        raise RuntimeError("coverage packets are empty or duplicated")
    controllers = {}; controller_paths = sorted((experiment / "controller").glob("*/manifest.json"))
    for path in controller_paths:
        controller = load(path)
        controller_keys = {"schema","taskId","seriesId","controllerSeedCommitment","slot","seed","binderFreeze","binderTreeSha256","populationSha256","requiredBindings","requiredObligations","expectedOutcome","expectedOracleClass","ambiguousChoices","refusalReason","commitments","publicManifestSha256","commitment"}
        if not isinstance(controller, dict) or set(controller) != controller_keys or controller.get("schema") != "semantic-editing-e04-controller/0.2" or not isinstance(controller.get("seriesId"),str) or len(controller["seriesId"])!=64 or not isinstance(controller.get("controllerSeedCommitment"),str) or len(controller["controllerSeedCommitment"])!=64 or not isinstance(controller.get("slot"), dict) or set(controller["slot"]) != {"family","variant","buildSystem","ordinal"}:
            raise RuntimeError(f"invalid coverage controller manifest: {path}")
        controllers[controller["taskId"]] = controller
    if len(controller_paths) != 42 or len(controllers) != 42:
        raise RuntimeError("coverage requires 42 unique controller manifests")
    public_by_task = {value["taskId"]:(path, value) for path, value in discover_public(experiment)}
    if set(controllers) != set(public_by_task):
        raise RuntimeError("coverage controller/public population mismatch")
    if {packet.get("taskId") for packet in packets} != set(controllers) or len(packets)!=42:
        raise RuntimeError("coverage packets must cover the exact 42-task R1 denominator")
    controller_cells = {(c["slot"]["family"], c["slot"]["buildSystem"].upper()) for c in controllers.values()}
    if len(controller_cells) != 14:
        raise RuntimeError(f"coverage population must contain 14 family/build cells, found {len(controller_cells)}")
    controller_slots = {(c["slot"]["family"], c["slot"]["buildSystem"].upper(), c["slot"]["variant"]) for c in controllers.values()}
    required_slots = {(family, build, variant) for family in FAMILIES for build in ("GRADLE", "MAVEN") for variant in ("positive", "ambiguous", "must-refuse")}
    if controller_slots != required_slots:
        raise RuntimeError("coverage controller does not contain the exact 42-slot denominator")
    forbidden = set(FAMILIES)
    for task_id, controller in controllers.items():
        forbidden.add(task_id)
        for binding in controller.get("requiredBindings", []):
            symbol = binding.split("=", 1)[-1]
            forbidden.add(symbol)
            forbidden.update(part for part in re.split(r"[^A-Za-z0-9_]+", symbol) if len(part) >= 8)
    product_text = "\n".join(path + "\n" + (product_root / path).read_text(encoding="utf-8", errors="ignore") for path in required_product_paths).casefold()
    normalized_product = re.sub(r"[^a-z0-9]", "", product_text)
    leaked = sorted(token for token in forbidden if token and (token.casefold() in product_text or re.sub(r"[^a-z0-9]", "", token.casefold()) in normalized_product))
    if leaked:
        raise RuntimeError(f"family/task vocabulary leaked into product paths: {leaked[:5]}")

    packet_by_task = {}; compositions = set(); false_bound = 0; positive_cells = set(); ambiguity_cells = set()
    for packet in packets:
        required_keys = {"schema","packetId","taskId","compilation","baseRevision","canonicalRequest","requestSha256","canonicalDecision","decisionSha256","compositionId","controllerCommitment","fixtureSourceSha256",*actual_digests.keys()}
        if not isinstance(packet, dict) or set(packet) != required_keys or packet["schema"] != "e04-zero-model-coverage-packet/0.1":
            raise RuntimeError("invalid zero-model coverage packet schema")
        task_id = packet["taskId"]
        if task_id not in controllers:
            raise RuntimeError(f"coverage packet has unknown task: {task_id}")
        controller = controllers[task_id]; public_path, public = public_by_task[task_id]
        if controller["commitment"] != controller_commitment_sha(controller) or packet["controllerCommitment"] != controller["commitment"] or public["controllerManifestCommitment"] != controller["commitment"] or controller["publicManifestSha256"] != sha_file(public_path):
            raise RuntimeError(f"coverage controller commitment mismatch: {task_id}")
        for key, value in actual_digests.items():
            if packet[key] != value:
                raise RuntimeError(f"coverage provenance mismatch {key}: {task_id}")
        fixture_root = (public_path.parent / public["repository"]).resolve()
        fixture_sha = source_digest(fixture_root)
        if packet["fixtureSourceSha256"] != fixture_sha or public["sourceSnapshotSha256"] != fixture_sha or f"source: {fixture_sha}" not in controller.get("commitments", []):
            raise RuntimeError(f"coverage fixture provenance mismatch: {task_id}")
        if sha_bytes(packet["canonicalRequest"].encode()) != packet["requestSha256"]:
            raise RuntimeError(f"coverage request digest mismatch: {task_id}")
        error, _ = validate_inline_typed_request(packet["canonicalRequest"], packet["compilation"], packet["baseRevision"], catalog)
        if error:
            raise RuntimeError(f"coverage request invalid {task_id}: {error}")
        request = json.loads(packet["canonicalRequest"])
        if typed_goal_composition_id(request) != packet["compositionId"]:
            raise RuntimeError(f"coverage composition digest mismatch: {task_id}")
        compositions.add(packet["compositionId"])
        try:
            decision = json.loads(packet["canonicalDecision"])
        except (TypeError, json.JSONDecodeError) as error:
            raise RuntimeError(f"coverage decision invalid: {task_id}") from error
        if compact(decision) != packet["canonicalDecision"] or sha_bytes(packet["canonicalDecision"].encode()) != packet["decisionSha256"]:
            raise RuntimeError(f"coverage decision digest mismatch: {task_id}")
        decision_error = validate_coverage_decision(decision, catalog)
        if decision_error:
            raise RuntimeError(f"coverage decision invalid {task_id}: {decision_error}")
        expected = controller["expectedOutcome"]; actual = decision["status"]
        declared_variables = set(request["goal"]["variables"])
        if actual == "BOUND" and set(decision["proof"]["bindings"]) != declared_variables:
            raise RuntimeError(f"coverage BOUND decision does not bind the exact request variables: {task_id}")
        if actual == "AMBIGUOUS" and any(set(choice["bindings"]) != declared_variables for choice in decision["choices"]):
            raise RuntimeError(f"coverage AMBIGUOUS decision does not bind the exact request variables: {task_id}")
        if actual == "BOUND" and expected != "BOUND": false_bound += 1
        cell = (controller["slot"]["family"], controller["slot"]["buildSystem"].upper())
        correct = False
        if expected == "BOUND" and actual == "BOUND":
            correct = (decision.get("proof") or {}).get("bindings") == coverage_expected_bindings(controller)
            if correct: positive_cells.add(cell)
        elif expected == "AMBIGUOUS" and actual == "AMBIGUOUS":
            actual_choices = {frozenset(choice["bindings"].items()) for choice in decision.get("choices", [])}
            expected_choices = {frozenset(tuple(binding.split("=", 1)) for binding in choice) for choice in controller["ambiguousChoices"]}
            correct = bool(actual_choices) and len(actual_choices) == len(decision["choices"]) and actual_choices == expected_choices
            if correct: ambiguity_cells.add(cell)
        elif expected == "REFUSED" and actual == "REFUSED":
            reason = decision.get("reason")
            correct = reason not in {"INVALID_GOAL", "SNAPSHOT_MISMATCH"} and adapter["mapping"].get(reason) == controller["refusalReason"]
        packet_by_task[task_id] = {"correct":correct, "expected":expected, "cell":cell}
    claimed_cells = positive_cells
    if len(compositions) < 5: raise RuntimeError("coverage requires at least 5 distinct compositionIds")
    if len(positive_cells) < 9: raise RuntimeError(f"coverage positive cells below 9/14: {len(positive_cells)}")
    if not claimed_cells <= ambiguity_cells: raise RuntimeError("coverage ambiguity sets are missing or inexact for claimed cells")
    must_refuse = {task_id for task_id, controller in controllers.items() if controller["expectedOutcome"] == "REFUSED"}
    correct_refuse = {task_id for task_id, result in packet_by_task.items() if result["expected"] == "REFUSED" and result["correct"]}
    if correct_refuse != must_refuse: raise RuntimeError("coverage must-refuse denominator is incomplete or incorrect")
    if false_bound != 0: raise RuntimeError(f"coverage false BOUND must be zero, found {false_bound}")
    builds = {build for _, build in positive_cells}
    if builds != {"GRADLE","MAVEN"}: raise RuntimeError("coverage positives must include Gradle and Maven")
    return {"status":"COVERAGE_ACCEPTED","compositionIds":len(compositions),"positiveCells":len(positive_cells),"denominator":14,"ambiguityCells":len(ambiguity_cells),"mustRefuseCorrect":len(correct_refuse),"falseBound":false_bound,"catalogSha256":catalog["catalogSha256"],"adapterSha256":adapter["adapterSha256"]}


def run_canary(args: argparse.Namespace) -> None:
    experiment = Path(args.experiment_root) if args.experiment_root else None
    output = Path(args.output)
    if experiment is None: raise RuntimeError("diagnostic canary requires --experiment-root")
    clew = Path(args.codeclew_bin or "")
    if not args.readiness_store or args.readiness_root != "DIAGNOSTIC_CANARY_START_READY": raise RuntimeError("run-canary requires DIAGNOSTIC_CANARY_START_READY")
    readiness_gate=readiness.require_root(self_module(),Path(args.readiness_graph),Path(args.readiness_store),args.readiness_root,experiment,clew,Path(args.dependency_seed),diagnostic_output=output,diagnostic_preflight=Path(args.diagnostic_preflight_report),diagnostic_audit=Path(args.diagnostic_audit_receipt),semantic_corpus=Path(args.semantic_corpus_bin))
    dependency_seed = validate_dependency_seed(Path(args.dependency_seed))
    typed_catalog = load_typed_goal_catalog(clew.resolve()) if clew.is_absolute() and clew.is_file() else None
    refusal_adapter = load_refusal_adapter(typed_catalog) if typed_catalog else None
    freeze_path=Path(readiness_gate["diagnosticFreezeArtifact"])
    plan = plan_packets(output,experiment,True,True,dependency_seed,typed_catalog,refusal_adapter,freeze_path)
    if args.dry_run:
        print(compact({"status": "DRY_RUN", "plannedRuns": len(plan["runs"]), "output": str(output)})); return
    if not clew.is_absolute() or not clew.is_file(): raise RuntimeError("--codeclew-bin must be an existing absolute frozen binary")
    if not freeze_path.is_file(): raise RuntimeError("live run requires an explicit freeze manifest")
    if sha_file(clew) != load(freeze_path).get("codeclewBinarySha256"):
        raise RuntimeError("Codeclew binary does not match the E04 freeze manifest")
    frozen_seed = load(freeze_path).get("dependencySeedManifestSha256")
    if dependency_seed["manifestSha256"] != frozen_seed:
        raise RuntimeError("dependency seed does not match the E04 freeze manifest")
    results_path = output / "runs.jsonl"
    existing = [json.loads(line) for line in results_path.read_text(encoding="utf-8").splitlines() if line] if results_path.exists() else []
    existing_ids = {row["runId"] for row in existing}
    if len(existing_ids) != len(existing): raise RuntimeError("duplicate retained run IDs")
    all_grouped: dict[str, list[dict[str, Any]]] = {}
    for row in plan["runs"]:
        all_grouped.setdefault(row["taskId"], []).append(row)
    canary_triplets = preregistered_canary_triplets(all_grouped)
    canary_ids = {rows[0]["taskId"] for rows in canary_triplets}
    canary_run_ids = {row["runId"] for triplet in canary_triplets for row in triplet}
    retained_canaries = [packet for packet in existing if packet["runId"] in canary_run_ids]
    if set(packet["runId"] for packet in retained_canaries) != canary_run_ids:
        if retained_canaries: raise RuntimeError("diagnostic canary triplet is partial and cannot resume")
        rows=canary_triplets[0]; public=load(Path(rows[0]["publicManifest"])); expected_commitment=public["controllerManifestCommitment"]
        def persist(packet: dict[str,Any]) -> None:
            write_json(output/"runs"/packet["runId"]/"run-packet.json",packet); append_jsonl(results_path,packet)
        retained_canaries=execute_triplet(rows,lambda row:execute_one(row,experiment,output,clew,dependency_seed,typed_catalog,refusal_adapter),lambda packet:r7_canary_reasons(packet,expected_commitment),persist)
    canary_failures = validate_retained_canaries(canary_triplets, retained_canaries)
    if canary_failures:
        raise RuntimeError(f"diagnostic canary rejected completion: {compact(canary_failures)}")
    readiness.issue_authority_completion(self_module(),Path(args.readiness_graph),Path(args.readiness_store),"DIAGNOSTIC_CANARY_3_COMPLETE","DIAGNOSTIC_CANARY_START_READY",{"taskId":next(iter(canary_ids)),"runIds":sorted(canary_run_ids),"runsJsonlSha256":sha_file(results_path)},experiment,clew,Path(args.dependency_seed),diagnostic_output=output,diagnostic_preflight=Path(args.diagnostic_preflight_report),diagnostic_audit=Path(args.diagnostic_audit_receipt),semantic_corpus=Path(args.semantic_corpus_bin))
    print(compact({"status":"CANARY_3_COMPLETE","canaryTaskId":next(iter(canary_ids)),"retainedRuns":3}))


def run_all(args: argparse.Namespace) -> None:
    experiment=Path(args.experiment_root); diagnostic=Path(args.diagnostic_experiment_root); output=Path(args.output); clew=Path(args.codeclew_bin or "")
    if experiment.resolve()==diagnostic.resolve(): raise RuntimeError("final run requires a distinct R1 corpus")
    if not args.readiness_store or args.readiness_root!="FINAL_MATRIX_START_READY": raise RuntimeError("run requires FINAL_MATRIX_START_READY")
    gate=readiness.require_root(self_module(),Path(args.readiness_graph),Path(args.readiness_store),args.readiness_root,experiment,clew,Path(args.dependency_seed),diagnostic,Path(args.diagnostic_output_root),output,Path(args.diagnostic_preflight_report),Path(args.diagnostic_audit_receipt),Path(args.semantic_corpus_bin))
    dependency_seed=validate_dependency_seed(Path(args.dependency_seed)); catalog=load_typed_goal_catalog(clew.resolve()); adapter=load_refusal_adapter(catalog); freeze_path=Path(gate["diagnosticFreezeArtifact"])
    plan=plan_packets(output,experiment,True,True,dependency_seed,catalog,adapter,freeze_path,True)
    results_path=output/"runs.jsonl"; existing=[json.loads(line) for line in results_path.read_text().splitlines() if line] if results_path.exists() else []
    planned={row["runId"]:row for row in plan["runs"]}
    if len({packet.get("runId") for packet in existing})!=len(existing) or any(packet.get("runId") not in planned or packet.get("publicManifest")!=planned[packet["runId"]]["publicManifest"] or packet.get("publicManifestSha256")!=planned[packet["runId"]]["publicManifestSha256"] for packet in existing): raise RuntimeError("final run contains foreign diagnostic/R1 packets")
    all_grouped={}
    for row in plan["runs"]: all_grouped.setdefault(row["taskId"],[]).append(row)
    r1_triplets=preregistered_r1_triplets(all_grouped); existing_by_id={packet["runId"]:packet for packet in existing}
    def persist(packet: dict[str,Any]) -> None:
        write_json(output/"runs"/packet["runId"]/"run-packet.json",packet)
        with RUNS_LOCK: append_jsonl(results_path,packet)
    for rows in r1_triplets:
        ids={row["runId"] for row in rows}; retained=ids & set(existing_by_id)
        if retained and retained!=ids: raise RuntimeError("R1 circuit-breaker triplet is partial")
        public=load(Path(rows[0]["publicManifest"])); commitment=public["controllerManifestCommitment"]
        if not retained:
            packets=execute_triplet(rows,lambda row:execute_one(row,experiment,output,clew,dependency_seed,catalog,adapter),lambda packet:r7_canary_reasons(packet,commitment),persist)
            existing.extend(packets); existing_by_id.update({packet["runId"]:packet for packet in packets})
        failures=validate_retained_canaries([rows],[existing_by_id[row["runId"]] for row in rows])
        if failures: raise RuntimeError(f"R1 circuit breaker failed:{compact(failures)}")
    r1_ids={row["runId"] for rows in r1_triplets for row in rows}; pending=[row for row in plan["runs"] if row["runId"] not in set(existing_by_id) and row["runId"] not in r1_ids]; grouped={}
    for row in pending: grouped.setdefault(row["taskId"],[]).append(row)
    with ThreadPoolExecutor(max_workers=max(1,min(int(args.max_workers),4))) as pool:
        futures=[pool.submit(execute_triplet,rows,lambda row:execute_one(row,experiment,output,clew,dependency_seed,catalog,adapter),None,persist) for rows in grouped.values()]
        for future in as_completed(futures): future.result()
    packets=[json.loads(line) for line in results_path.read_text().splitlines() if line]
    if len(packets)!=126 or {packet["runId"] for packet in packets}!=set(planned): raise RuntimeError("model matrix is partial; completion refused")
    readiness.issue_authority_completion(self_module(),Path(args.readiness_graph),Path(args.readiness_store),"FINAL_MATRIX_126_COMPLETE","FINAL_MATRIX_START_READY",{"runs":126,"runsJsonlSha256":sha_file(results_path)},experiment,clew,Path(args.dependency_seed),diagnostic,Path(args.diagnostic_output_root),output,Path(args.diagnostic_preflight_report),Path(args.diagnostic_audit_receipt),Path(args.semantic_corpus_bin))
    print(compact({"status":"MODEL_MATRIX_126_COMPLETE","runs":126}))


def binding_set(strings: list[str]) -> set[str]:
    return {str(item) for item in strings}


def actual_bindings(output: dict[str, Any]) -> set[str]:
    return {f"{item['role']}={item['symbol']}" for item in output["goal"]["bindings"]}


def judge(args: argparse.Namespace) -> None:
    experiment, output = Path(args.experiment_root), Path(args.output)
    if args.readiness_root!="JUDGE_START_READY": raise RuntimeError("judge requires JUDGE_START_READY")
    diagnostic=Path(args.diagnostic_experiment_root); readiness.require_root(self_module(),Path(args.readiness_graph),Path(args.readiness_store),args.readiness_root,experiment,Path(args.codeclew_bin),Path(args.dependency_seed),diagnostic,Path(args.diagnostic_output_root),output,Path(args.diagnostic_preflight_report),Path(args.diagnostic_audit_receipt),Path(args.semantic_corpus_bin))
    _judge_authorized(args,experiment,output)


def _judge_authorized(args: argparse.Namespace, experiment: Path, output: Path) -> None:
    packets = [json.loads(line) for line in (output / "runs.jsonl").read_text(encoding="utf-8").splitlines() if line]
    if len(packets) != 126: raise RuntimeError(f"judge requires all 126 retained runs, found {len(packets)}")
    judged = output / "judgments.jsonl"
    if judged.exists(): judged.unlink()
    for packet in packets:
        controller = load(experiment / "controller" / packet["taskId"] / "manifest.json")
        public_path = experiment / "agent" / packet["taskId"] / "task-manifest.json"
        public = load(public_path)
        if controller.get("schema") != "semantic-editing-e04-controller/0.2" or controller.get("taskId") != packet["taskId"] or not isinstance(controller.get("seriesId"),str) or len(controller["seriesId"])!=64 or not isinstance(controller.get("controllerSeedCommitment"),str) or len(controller["controllerSeedCommitment"])!=64 or controller.get("binderFreeze") != BASE or controller.get("populationSha256") != POP_SHA or public.get("controllerManifestCommitment") != controller.get("commitment") or controller.get("publicManifestSha256") != sha_file(public_path):
            raise RuntimeError(f"controller/public commitment mismatch for {packet['taskId']}")
        expected, model = controller["expectedOutcome"], packet.get("modelOutput") or {}
        infrastructure_valid = packet.get("infrastructureValid", packet["executionStatus"] == "OK")
        output_valid = packet.get("modelOutputValid", not validate_model_output(model))
        protocol_valid = packet.get("protocolValid", not packet["auditFlags"])
        actual_status = model.get("status"); semantic_correct = False; tp = fp = 0
        fn = len(controller["requiredBindings"]) if expected == "BOUND" else 0
        if output_valid and expected == "BOUND" and actual_status == "BOUND":
            expected_bindings = binding_set(controller["requiredBindings"]); actual = actual_bindings(model)
            tp, fp, fn = len(actual & expected_bindings), len(actual - expected_bindings), len(expected_bindings - actual)
            bound_symbols = [binding.split("=", 1)[1] for binding in controller["requiredBindings"]]
            evidence = "\n".join(model["goal"]["evidenceAnchors"])
            semantic_correct = (
                model.get("inferredFamily") == controller["slot"]["family"]
                and fp == 0 and fn == 0
                and set(model["goal"]["obligations"]) == set(controller["requiredObligations"])
                and model["goal"]["oracleClass"] == controller["expectedOracleClass"]
                and all(symbol in evidence for symbol in bound_symbols)
            )
        elif output_valid and expected == "AMBIGUOUS" and actual_status == "AMBIGUOUS":
            choices = {
                frozenset(f"{binding['role']}={binding['symbol']}" for binding in choice["bindings"])
                for choice in model["ambiguity"]["choices"]
            }
            expected_choices = {frozenset(choice) for choice in controller["ambiguousChoices"]}
            semantic_correct = model.get("inferredFamily") == controller["slot"]["family"] and choices == expected_choices
        elif output_valid and expected == "REFUSED" and actual_status == "REFUSED":
            semantic_correct = model.get("inferredFamily") == controller["slot"]["family"] and model["refusal"]["code"] == controller["refusalReason"]
        accepted = infrastructure_valid and output_valid and protocol_valid and semantic_correct
        judgment = {"schema": "semantic-editing-e04-judgment/0.2", "runId": packet["runId"], "taskId": packet["taskId"], "arm": packet["arm"], "family": controller["slot"]["family"], "variant": controller["slot"]["variant"], "expectedOutcome": expected, "actualStatus": actual_status, "infrastructureValid":infrastructure_valid, "modelOutputValid":output_valid, "protocolValid":protocol_valid, "semanticCorrect":semantic_correct, "accepted":accepted, "falseComplete": output_valid and actual_status == "BOUND" and expected != "BOUND", "bindingTp": tp, "bindingFp": fp, "bindingFn": fn, "packet": packet}
        append_jsonl(judged, judgment)
    if not getattr(args,"_skip_authority_for_selftest",False): readiness.issue_authority_completion(self_module(),Path(args.readiness_graph),Path(args.readiness_store),"JUDGE_COMPLETE","JUDGE_START_READY",{"judgments":126,"judgmentsSha256":sha_file(judged)},experiment,Path(args.codeclew_bin),Path(args.dependency_seed),diagnostic,Path(args.diagnostic_output_root),output,Path(args.diagnostic_preflight_report),Path(args.diagnostic_audit_receipt),Path(args.semantic_corpus_bin))
    print(compact({"status": "JUDGED", "runs": 126, "output": str(judged)}))


def summarize(args: argparse.Namespace) -> None:
    experiment=Path(args.experiment_root)
    if args.readiness_root!="SUMMARIZE_START_READY": raise RuntimeError("summarize requires SUMMARIZE_START_READY")
    diagnostic=Path(args.diagnostic_experiment_root); readiness.require_root(self_module(),Path(args.readiness_graph),Path(args.readiness_store),args.readiness_root,experiment,Path(args.codeclew_bin),Path(args.dependency_seed),diagnostic,Path(args.diagnostic_output_root),Path(args.output),Path(args.diagnostic_preflight_report),Path(args.diagnostic_audit_receipt),Path(args.semantic_corpus_bin))
    _summarize_authorized(args,experiment)


def _summarize_authorized(args: argparse.Namespace, experiment: Path) -> None:
    rows = [json.loads(line) for line in (Path(args.output) / "judgments.jsonl").read_text(encoding="utf-8").splitlines() if line]
    if len(rows) != 126: raise RuntimeError("summary requires 126 judgments")
    result = {"schema": "semantic-editing-e04-summary/0.2", "population": "NARROW_POPULATION", "arms": {}}
    for arm in ARMS:
        selected = [row for row in rows if row["arm"] == arm]; positives = [r for r in selected if r["expectedOutcome"] == "BOUND"]; ambiguous = [r for r in selected if r["expectedOutcome"] == "AMBIGUOUS"]; refused = [r for r in selected if r["expectedOutcome"] == "REFUSED"]
        tp, fp, fn = sum(r["bindingTp"] for r in positives), sum(r["bindingFp"] for r in positives), sum(r["bindingFn"] for r in positives)
        packets = [r["packet"] for r in selected]
        total = lambda path: sum((path(p) or 0) for p in packets)
        median = lambda values: statistics.median(values) if values else 0
        family_breakdown = {}
        for family in FAMILIES:
            family_rows = [r for r in selected if r["family"] == family]; family_positive = [r for r in family_rows if r["expectedOutcome"] == "BOUND"]
            ftp, ffp, ffn = sum(r["bindingTp"] for r in family_positive), sum(r["bindingFp"] for r in family_positive), sum(r["bindingFn"] for r in family_positive)
            family_breakdown[family] = {"positiveCorrect":sum(r["accepted"] for r in family_positive),"positiveSemanticCorrect":sum(r["semanticCorrect"] for r in family_positive),"positiveDenominator":2,"applicability":sum(r["accepted"] for r in family_positive)/2,"bindingPrecision":ftp/(ftp+ffp) if ftp+ffp else 0,"bindingRecall":ftp/(ftp+ffn) if ftp+ffn else 0,"ambiguityCorrect":sum(r["accepted"] for r in family_rows if r["expectedOutcome"]=="AMBIGUOUS"),"ambiguitySemanticCorrect":sum(r["semanticCorrect"] for r in family_rows if r["expectedOutcome"]=="AMBIGUOUS"),"ambiguityDenominator":2,"mustRefuseCorrect":sum(r["accepted"] for r in family_rows if r["expectedOutcome"]=="REFUSED"),"mustRefuseSemanticCorrect":sum(r["semanticCorrect"] for r in family_rows if r["expectedOutcome"]=="REFUSED"),"mustRefuseDenominator":2,"falseComplete":sum(r["falseComplete"] for r in family_rows)}
        result["arms"][arm] = {"runs": 42, "acceptedRuns":sum(r["accepted"] for r in selected), "semanticCorrectRuns":sum(r["semanticCorrect"] for r in selected), "infrastructureInvalidRuns":sum(not r["infrastructureValid"] for r in selected), "modelOutputInvalidRuns":sum(not r["modelOutputValid"] for r in selected), "protocolInvalidRuns":sum(not r["protocolValid"] for r in selected), "failedOrAuditedRuns":sum(not r["infrastructureValid"] or not r["modelOutputValid"] or not r["protocolValid"] for r in selected), "applicablePositiveBound": sum(r["accepted"] for r in positives), "semanticPositiveBound":sum(r["semanticCorrect"] for r in positives), "applicabilityDenominator": 14, "applicability": sum(r["accepted"] for r in positives) / 14, "bindingPrecision": tp / (tp + fp) if tp + fp else 0, "bindingRecall": tp / (tp + fn) if tp + fn else 0, "ambiguityCorrect": sum(r["accepted"] for r in ambiguous), "ambiguitySemanticCorrect":sum(r["semanticCorrect"] for r in ambiguous), "ambiguityDenominator": 14, "ambiguityAccuracy":sum(r["accepted"] for r in ambiguous)/14, "mustRefuseCorrect": sum(r["accepted"] for r in refused), "mustRefuseSemanticCorrect":sum(r["semanticCorrect"] for r in refused), "mustRefuseDenominator": 14, "mustRefuseAccuracy":sum(r["accepted"] for r in refused)/14, "falseComplete": sum(r["falseComplete"] for r in selected), "wallMilliseconds": total(lambda p: p["wallMilliseconds"]), "medianWallMilliseconds":median([p["wallMilliseconds"] for p in packets]), "contextBytes": total(lambda p: p["contextBytes"]), "medianContextBytes":median([p["contextBytes"] for p in packets]), "goalBytes": total(lambda p: p["goalBytes"]), "medianGoalBytes":median([p["goalBytes"] for p in packets if isinstance(p.get("modelOutput"), dict) and p["modelOutput"].get("status") == "BOUND"]), "medianClarificationTurns":0, "turns": total(lambda p: p["metrics"]["turns"]), "actionCalls": total(lambda p: p["metrics"]["actionCalls"]), "navigationCalls": total(lambda p: p["navigationCalls"]), "inputTokens": total(lambda p: p["metrics"]["inputTokens"]), "cachedInputTokens": total(lambda p: p["metrics"]["cachedInputTokens"]), "outputTokens": total(lambda p: p["metrics"]["outputTokens"]), "noncachedTokens": total(lambda p: p["metrics"]["noncachedTokens"]), "nativeTokenRuns": sum(p["metrics"]["nativeTokenTelemetryAvailable"] for p in packets), "families":family_breakdown}
    summary_path=Path(args.output)/"summary.json"; write_json(summary_path,result)
    if not getattr(args,"_skip_authority_for_selftest",False): readiness.issue_authority_completion(self_module(),Path(args.readiness_graph),Path(args.readiness_store),"SUMMARY_COMPLETE","SUMMARIZE_START_READY",{"summarySha256":sha_file(summary_path)},experiment,Path(args.codeclew_bin),Path(args.dependency_seed),diagnostic,Path(args.diagnostic_output_root),Path(args.output),Path(args.diagnostic_preflight_report),Path(args.diagnostic_audit_receipt),Path(args.semantic_corpus_bin))
    print(compact(result))


def self_test_zero_model_coverage(base: Path, binary: Path, catalog: dict[str, Any]) -> dict[str, Any]:
    experiment = base / "coverage-experiment"
    product_root = base / "coverage-product"; product_root.mkdir()
    product_file = product_root / "crates/clew/src/neutral_kernel.rs"; product_file.parent.mkdir(parents=True)
    product_file.write_text("pub fn execute() -> bool { true }\n", encoding="utf-8")
    command_output(["git", "init", "-q"], product_root)
    command_output(["git", "add", "."], product_root)
    subprocess.run(["git", "-c", "user.name=E04", "-c", "user.email=e04@example.invalid", "commit", "-qm", "fixture"], cwd=product_root, check=True)
    provenance = coverage_packet_digest_fields(binary, catalog, product_root)
    cells = [(family, build) for family in FAMILIES for build in ("GRADLE", "MAVEN")]
    claimed_cells = set(cells[:9])
    packets: list[dict[str, Any]] = []
    task_index = 0
    optional_operators = (None, "PRESERVE_ORDER", "PRESERVE_CARDINALITY", "PRESERVE_EFFECTS", "PRESERVE_NULLABILITY")
    for family, build in cells:
        roles = FAMILY_CONTRACTS[family]["roles"]
        domains = {roles[0]:"CALLABLE", roles[1]:"CALLABLE", roles[2]:"VALUE_EDGE"}
        operators = [{"operator":"MAP_EDGE", "operands":roles}]
        optional = optional_operators[len(packets) % len(optional_operators)]
        if optional:
            operators.append({"operator":optional, "operands":[roles[2]]})
        bindings = {role:f"org.example.unit{task_index}.{role.lower()}" for role in roles}
        alternatives = [
            {role:f"org.example.unit{task_index}.a.{role.lower()}" for role in roles},
            {role:f"org.example.unit{task_index}.b.{role.lower()}" for role in roles},
        ]
        for variant in ("positive", "ambiguous", "must-refuse"):
            task_id = f"e04-{task_index:016x}"; task_index += 1
            public_dir = experiment / "agent" / task_id
            fixture_root = public_dir / "repository"; fixture_root.mkdir(parents=True)
            (fixture_root / "Unit.kt").write_text("package org.example\nclass Unit\n", encoding="utf-8")
            fixture_sha = source_digest(fixture_root)
            slot = {"family":family, "variant":variant, "buildSystem":build.lower(), "ordinal":0}
            expected = {"positive":"BOUND", "ambiguous":"AMBIGUOUS", "must-refuse":"REFUSED"}[variant]
            controller = {
                "schema":"semantic-editing-e04-controller/0.2", "taskId":task_id,"seriesId":"a"*64,"controllerSeedCommitment":"b"*64,
                "slot":slot, "seed":task_index, "binderFreeze":"coverage-freeze",
                "binderTreeSha256":"1"*64, "populationSha256":POP_SHA,
                "requiredBindings":[f"{role}={symbol}" for role, symbol in bindings.items()],
                "requiredObligations":FAMILY_CONTRACTS[family]["obligations"],
                "expectedOutcome":expected,
                "expectedOracleClass":"EXTERNAL_SPEC" if expected == "BOUND" else None,
                "ambiguousChoices":[[f"{role}={symbol}" for role, symbol in choice.items()] for choice in alternatives] if expected == "AMBIGUOUS" else [],
                "refusalReason":"INCOMPLETE_SEMANTIC_EVIDENCE" if expected == "REFUSED" else None,
                "commitments":[f"slot:{family}:{build}:{variant}", f"source: {fixture_sha}"],
                "publicManifestSha256":"", "commitment":"",
            }
            controller["commitment"] = controller_commitment_sha(controller)
            public = {
                "schema":"semantic-editing-e04-public-task/0.1", "taskId":task_id,
                "buildSystem":build, "kotlinVersion":"2.1.21", "task":"Resolve the requested semantic relationship.",
                "repository":"repository", "sourceSnapshotSha256":fixture_sha,
                "buildCommand":[], "controllerManifestCommitment":controller["commitment"],
            }
            public_path = public_dir / "task-manifest.json"; write_json(public_path, public)
            controller["publicManifestSha256"] = sha_file(public_path)
            assert controller["commitment"] == controller_commitment_sha(controller)
            write_json(experiment / "controller" / task_id / "manifest.json", controller)
            request = {
                "schema":catalog["requestSchema"], "compilation":":unit/main", "hints":[],
                "goal":{"schema":catalog["goalSchema"], "baseRevision":"coverage-base", "variables":domains, "operators":operators},
            }
            if expected != "REFUSED" and (family, build) not in claimed_cells:
                decision = {"schema":catalog["decisionSchema"], "status":"REFUSED", "reason":"NO_COMPATIBLE_BINDINGS"}
            elif expected == "BOUND":
                decision = {"schema":catalog["decisionSchema"], "status":"BOUND", "proof":{"bindings":bindings}}
            elif expected == "AMBIGUOUS":
                decision = {"schema":catalog["decisionSchema"], "status":"AMBIGUOUS", "choices":[{"bindings":choice} for choice in alternatives]}
            else:
                decision = {"schema":catalog["decisionSchema"], "status":"REFUSED", "reason":"NO_COMPATIBLE_BINDINGS"}
            packets.append(make_zero_model_coverage_packet(
                f"coverage-{task_id}", task_id, ":unit/main", "coverage-base", request,
                decision, controller["commitment"], fixture_root, provenance,
            ))
    report = guard_zero_model_coverage(packets, experiment, binary, product_root)
    assert report["status"] == "COVERAGE_ACCEPTED" and report["positiveCells"] == 9 and report["mustRefuseCorrect"] == 14

    def rejected(candidate: list[dict[str, Any]], text: str) -> None:
        try:
            guard_zero_model_coverage(candidate, experiment, binary, product_root)
            raise AssertionError(f"coverage counterexample accepted: {text}")
        except RuntimeError as error:
            assert text in str(error), (text, str(error))

    refused_index = next(index for index, packet in enumerate(packets) if json.loads(packet["canonicalDecision"])["status"] == "REFUSED")
    wrong_refusal=json.loads(json.dumps(packets)); refusal=json.loads(wrong_refusal[refused_index]["canonicalDecision"]); refusal["reason"]="INVALID_GOAL"; wrong_refusal[refused_index]["canonicalDecision"]=compact(refusal); wrong_refusal[refused_index]["decisionSha256"]=sha_bytes(wrong_refusal[refused_index]["canonicalDecision"].encode())
    rejected(wrong_refusal, "must-refuse denominator")
    forged = json.loads(json.dumps(packets)); forged[0]["productCatalogSha256"] = "0" * 64
    rejected(forged, "provenance mismatch")
    forged_commitment = json.loads(json.dumps(packets)); forged_commitment[0]["controllerCommitment"] = "0" * 64
    rejected(forged_commitment, "controller commitment mismatch")
    rejected(packets + [dict(packets[0])], "duplicated")
    positive_index = next(index for index, packet in enumerate(packets) if json.loads(packet["canonicalDecision"])["status"] == "BOUND")
    below=json.loads(json.dumps(packets)); refused={"schema":catalog["decisionSchema"],"status":"REFUSED","reason":"NO_COMPATIBLE_BINDINGS"}; below[positive_index]["canonicalDecision"]=compact(refused); below[positive_index]["decisionSha256"]=sha_bytes(below[positive_index]["canonicalDecision"].encode())
    rejected(below, "positive cells below 9/14")
    return {"packets":len(packets), "counterexamples":5, **report}


def self_test() -> None:
    spec = population(); assert len(matrix(None)) == 126
    self_test_clew = ROOT / "target/debug/clew"
    typed_catalog = load_typed_goal_catalog(self_test_clew)
    assert typed_catalog["schema"] == "typed-goal-language-schema/0.1"
    assert typed_catalog["derivedCapabilities"]["issuableRoots"] == list(TYPED_GOAL_ISSUABLE_ROOTS)
    assert typed_catalog["derivedCapabilities"]["auxiliaryOperators"] == ["BIND_UNIQUE", "VALUE_FLOWS_TO"]
    assert {"NULLABLE_CONSTRUCTION", "PROJECTION"} <= set(typed_catalog["derivedCapabilities"]["nonExecutableDomains"])
    refusal_adapter = load_refusal_adapter(typed_catalog)
    frozen_ast = ast_index_provenance(); require_frozen_ast_index(frozen_ast)
    ast_fixture_path = ROOT / "scripts/fixtures/e04-ast-readiness.json"
    ast_fixture_cases = load(ast_fixture_path)
    ast_artifact_counterexamples = 8
    with tempfile.TemporaryDirectory(prefix="e04-ast-readiness-fixture-") as ast_temporary, anchored_ast_state(Path(ast_temporary)) as fixture_ast_anchor:
        ast_root = Path(fixture_ast_anchor["rootPath"])
        fixture_ast_db = ast_root / "ast-index.db"
        canonical_fixture_ast_db, _ = ast_db_identity(fixture_ast_db, fixture_ast_anchor)
        fixture_bytes = b"SQLite format 3\0" + b"\0" * (225280 - 16)
        fixture_ast_db.write_bytes(fixture_bytes)
        positive_ast_summary = None; positive_parsed_summary = None
        for case in ast_fixture_cases:
            raw = case["stdout"] if isinstance(case["stdout"], str) else compact(case["stdout"])
            raw = raw.replace("${AST_DB}", str(fixture_ast_db))
            try:
                summary = parse_ast_readiness(raw, fixture_ast_db)
                if case["expectedError"] is not None:
                    raise AssertionError(f"AST readiness counterexample accepted: {case['name']}")
                positive_parsed_summary = summary
                positive_ast_summary = attest_ast_db_artifact(summary, fixture_ast_db, fixture_ast_anchor)
            except RuntimeError as error:
                assert case["expectedError"] == str(error), (case["name"], case["expectedError"], str(error))
        assert positive_ast_summary is not None and positive_parsed_summary is not None
        for field, wrong in (("schema","wrong/0.1"), ("status","OK"), ("dbPath","/tmp/wrong.db")):
            try:
                validate_ast_readiness_summary({**positive_ast_summary, field:wrong}, canonical_fixture_ast_db)
                raise AssertionError(f"normalized AST readiness accepted wrong {field}")
            except RuntimeError:
                pass
        fixture_ast_db.unlink()
        try:
            attest_ast_db_artifact(positive_parsed_summary, fixture_ast_db, fixture_ast_anchor)
            raise AssertionError("absent AST DB accepted")
        except RuntimeError as error:
            assert str(error) == "AST_DB_MISSING"
        symlink_target = ast_root / "target.db"; symlink_target.write_bytes(fixture_bytes)
        fixture_ast_db.symlink_to(symlink_target)
        try:
            attest_ast_db_artifact(positive_parsed_summary, fixture_ast_db, fixture_ast_anchor)
            raise AssertionError("symlink AST DB accepted")
        except RuntimeError as error:
            assert str(error) == "AST_DB_SYMLINK"
        fixture_ast_db.unlink(); symlink_target.unlink(); fixture_ast_db.write_bytes(fixture_bytes[:-1])
        try:
            attest_ast_db_artifact(positive_parsed_summary, fixture_ast_db, fixture_ast_anchor)
            raise AssertionError("wrong-size AST DB accepted")
        except RuntimeError as error:
            assert str(error) == "AST_DB_SIZE_MISMATCH"
        fixture_ast_db.write_bytes(fixture_bytes)
        first_attestation = attest_ast_db_artifact(positive_parsed_summary, fixture_ast_db, fixture_ast_anchor)
        fixture_ast_db.write_bytes(fixture_bytes[:-1] + b"x")
        try:
            attest_ast_db_artifact(first_attestation, fixture_ast_db, fixture_ast_anchor, first_attestation["dbSha256"])
            raise AssertionError("post-summary AST DB mutation accepted")
        except RuntimeError as error:
            assert str(error) == "AST_DB_SHA256_MISMATCH"
        nested = ast_root / "nested"; nested.mkdir()
        nested_db = nested / "ast-index.db"; nested_db.write_bytes(fixture_bytes)
        nested_summary = {**positive_parsed_summary, "dbPath":str(nested_db)}
        saved_nested = ast_root / "nested-saved"
        def swap_parent(stage: str, _: dict[str, Any]) -> None:
            if stage == "before_parent_open" and nested.exists() and not nested.is_symlink():
                nested.rename(saved_nested); nested.symlink_to(saved_nested, target_is_directory=True)
        try:
            attest_ast_db_artifact(nested_summary, nested_db, fixture_ast_anchor, test_hook=swap_parent)
            raise AssertionError("parent directory swap accepted")
        except RuntimeError as error:
            assert str(error) == "AST_DB_SYMLINKED_PARENT"
        finally:
            if nested.is_symlink(): nested.unlink()
            if saved_nested.exists(): saved_nested.rename(nested)
        fixture_ast_db.write_bytes(fixture_bytes)
        swap_target = ast_root / "swap-target.db"; swap_target.write_bytes(fixture_bytes)
        def swap_database(stage: str, _: dict[str, Any]) -> None:
            if stage == "before_db_open" and fixture_ast_db.exists() and not fixture_ast_db.is_symlink():
                fixture_ast_db.unlink(); fixture_ast_db.symlink_to(swap_target)
        try:
            attest_ast_db_artifact(positive_parsed_summary, fixture_ast_db, fixture_ast_anchor, test_hook=swap_database)
            raise AssertionError("regular-to-symlink DB swap accepted")
        except RuntimeError as error:
            assert str(error) == "AST_DB_SYMLINK"
        if fixture_ast_db.is_symlink(): fixture_ast_db.unlink()
        if swap_target.exists(): swap_target.unlink()
        fixture_ast_db.write_bytes(fixture_bytes)
        saved_root = Path(ast_temporary) / "state-saved"
        ast_root.rename(saved_root); ast_root.symlink_to(saved_root, target_is_directory=True)
        try:
            attest_ast_db_artifact(positive_parsed_summary, fixture_ast_db, fixture_ast_anchor)
            raise AssertionError("root replacement symlink accepted")
        except RuntimeError as error:
            assert str(error) == "AST_STATE_ROOT_SYMLINK_OR_MISSING"
        finally:
            if ast_root.is_symlink(): ast_root.unlink()
            saved_root.rename(ast_root)
        ast_root.rename(saved_root); ast_root.mkdir(); fixture_ast_db.write_bytes(fixture_bytes)
        try:
            attest_ast_db_artifact(positive_parsed_summary, fixture_ast_db, fixture_ast_anchor)
            raise AssertionError("root rename plus new real directory accepted")
        except RuntimeError as error:
            assert str(error) == "AST_STATE_ROOT_IDENTITY_CHANGED"
        finally:
            shutil.rmtree(ast_root); saved_root.rename(ast_root)
    sample = ['{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":40,"output_tokens":7}}', '{"type":"item.completed","item":{"id":"x","type":"command_execution","command":"/bin/zsh -lc \'rg foo .\'","aggregated_output":"abc"}}']
    metrics, _, commands = event_metrics(sample); assert metrics["noncachedTokens"] == 67 and metrics["actionCalls"] == 1
    frozen_clew = Path("/opt/frozen/clew")
    flags, navigation = audit("codeclew", commands, "a", "a", frozen_clew); assert "FALLBACK_SEARCH" in flags and navigation == 1
    fixture_path = ROOT / "scripts/fixtures/e04-s0-command-events.json"
    for case in load(fixture_path):
        _, _, fixture_commands = event_metrics([compact(case["event"])])
        flags, _ = audit(
            case["arm"], fixture_commands, "a", "a", frozen_clew, ROOT,
            case.get("selectedCompilation"), case.get("baseRevision"), None, typed_catalog,
        )
        assert flags == case["expectedFlags"], (case["name"], flags)
    flags, _ = audit(
        "ast-index",
        [
            {"command": "/bin/zsh -lc 'ast-index search DefinitelyUnrelated'", "output": "src/Other.kt:1", "exitCode":0},
            {"command": "/bin/zsh -lc \"sed -n '1,40p' README.md\"", "output": "readme", "exitCode":0},
        ],
        "a",
        "a",
        repository=ROOT,
    )
    assert "FALLBACK_SEARCH" in flags
    flags, _ = audit(
        "ast-index",
        [
            {"command": "/bin/zsh -lc 'ast-index search README'", "output": "README.md:1", "exitCode":0},
            {"command": "/bin/zsh -lc \"sed -n '1,40p' README.md\"", "output": "readme", "exitCode":0},
        ],
        "a",
        "a",
        repository=ROOT,
    )
    assert "FALLBACK_SEARCH" not in flags
    project_fixture_path = ROOT / "scripts/fixtures/e04-s0-project-model-fixtures.json"
    project_fixtures = load(project_fixture_path)
    for case in project_fixtures:
        assert case["kind"] == "gradle-task-output"
        assert parse_gradle_compilations(case["input"]) == case["expected"], case["name"]
    bound = {"schema":"semantic-editing-e04-model-output/0.1","status":"BOUND","inferredFamily":FAMILIES[0],"goal":{"bindings":[{"role":"TRANSFORMER","symbol":"p.f"}],"obligations":[obligations_catalog(spec)[0]],"evidenceAnchors":["a"],"oracleClass":"EXTERNAL_SPEC"},"ambiguity":None,"refusal":None}
    assert not validate_model_output(bound)
    prompt = common_prompt(spec, typed_catalog); assert "must-refuse" not in prompt and "positive" not in prompt
    with tempfile.TemporaryDirectory(prefix="e04-runner-self-test-") as temporary:
        base = Path(temporary); dry = base / "dry"
        maven_repository = base / "maven-leaf-repository"
        fixture_artifact = maven_repository / "org/example/library/1/library-1.jar"
        fixture_artifact.parent.mkdir(parents=True)
        fixture_artifact.write_bytes(b"fixture jar")
        flat_reactor = base / "flat-reactor"; flat_reactor.mkdir()
        (flat_reactor / "pom.xml").write_text(
            "<project><modelVersion>4.0.0</modelVersion><groupId>fixture</groupId>"
            "<artifactId>flat</artifactId><version>1</version><dependencies><dependency>"
            "<groupId>org.example</groupId><artifactId>library</artifactId><version>1</version>"
            "</dependency></dependencies></project>", encoding="utf-8",
        )
        module_reactor = base / "module-reactor"; (module_reactor / "child").mkdir(parents=True)
        (module_reactor / "pom.xml").write_text(
            "<project><modelVersion>4.0.0</modelVersion><groupId>fixture</groupId>"
            "<artifactId>aggregator</artifactId><version>1</version><packaging>pom</packaging>"
            "<modules><module>child</module></modules></project>", encoding="utf-8",
        )
        (module_reactor / "child/pom.xml").write_text(
            "<project><modelVersion>4.0.0</modelVersion><groupId>fixture</groupId>"
            "<artifactId>child</artifactId><version>1</version><dependencies><dependency>"
            "<groupId>org.example</groupId><artifactId>library</artifactId><version>1</version>"
            "</dependency></dependencies></project>", encoding="utf-8",
        )
        flat_leaves = maven_reactor_leaves(flat_reactor)
        module_leaves = maven_reactor_leaves(module_reactor)
        assert [(leaf["relativePom"], leaf["gav"], leaf["dependencyBearing"]) for leaf in flat_leaves] == [("pom.xml", "fixture:flat:1", True)]
        assert [(leaf["relativePom"], leaf["gav"], leaf["dependencyBearing"]) for leaf in module_leaves] == [("child/pom.xml", "fixture:child:1", True)]
        used_classpath_outputs: set[Path] = set()
        flat_classpath = flat_reactor / "target/e04-classpath-00.txt"; flat_classpath.parent.mkdir()
        module_classpath = module_reactor / "target/e04-classpath-00.txt"; module_classpath.parent.mkdir()
        flat_classpath.write_text(str(fixture_artifact.resolve()), encoding="utf-8")
        module_classpath.write_text(str(fixture_artifact.resolve()), encoding="utf-8")
        flat_classpath_result = validate_maven_leaf_classpath(flat_classpath, flat_reactor, maven_repository, True, used_classpath_outputs)
        module_classpath_result = validate_maven_leaf_classpath(module_classpath, module_reactor, maven_repository, True, used_classpath_outputs)
        assert flat_classpath_result["artifacts"] and module_classpath_result["artifacts"]
        try:
            validate_maven_leaf_classpath(flat_classpath, flat_reactor, maven_repository, True, used_classpath_outputs)
            raise AssertionError("reused Maven leaf classpath output was accepted")
        except RuntimeError as error:
            assert str(error) == "MAVEN_LEAF_CLASSPATH_OUTPUT_COLLISION"
        publisher_root = base / "publisher"; publisher_root.mkdir()
        publisher_staging_parent = publisher_root / "staging"; publisher_staging_parent.mkdir(mode=0o700)
        publisher_staged = publisher_staging_parent / "snapshot"; publisher_staged.mkdir()
        for directory, payload in (("gradle-modules", b"g"), ("gradle-wrapper-dists", b"w"), ("maven-repository", b"m")):
            target = publisher_staged / directory; target.mkdir(); (target / "artifact.bin").write_bytes(payload)
        publisher_records = {}
        for name, directory in (("gradle",publisher_staged/"gradle-modules"),("gradleWrapper",publisher_staged/"gradle-wrapper-dists"),("maven",publisher_staged/"maven-repository")):
            tree_sha, files, size = tree_sha256(directory); publisher_records[name] = {"treeSha256":tree_sha,"files":files,"bytes":size}
        write_json(publisher_staged / "manifest.json", {"schema":"semantic-editing-e04-dependency-seed/0.1",**publisher_records})
        publisher_output = publisher_root / "published"
        published = atomic_publish_dependency_seed(publisher_staged, publisher_output)
        publisher_marker = Path(str(publisher_output) + ".complete")
        assert published["manifestSha256"] == sha_file(publisher_output / "payload/manifest.json"), "sealed-publisher: published manifest"
        assert publisher_staging_parent.lstat().st_mode & stat.S_IWUSR, "sealed-publisher: staging parent writable"
        validate_read_only_regular_tree(publisher_output)
        publisher_marker.rename(publisher_root / "saved.complete")
        try:
            validate_dependency_seed(publisher_output); raise AssertionError("unsealed dependency seed was accepted")
        except RuntimeError as error:
            assert "completion marker is missing" in str(error), f"sealed-publisher: missing marker diagnostic: {error}"
        (publisher_root / "saved.complete").rename(publisher_marker)
        (publisher_output / "payload").chmod(0o755)
        try:
            validate_dependency_seed(publisher_output); raise AssertionError("writable dependency seed payload was accepted")
        except RuntimeError as error:
            assert "entry is writable" in str(error), f"sealed-publisher: writable payload diagnostic: {error}"
        (publisher_output / "payload").chmod(0o555)
        publisher_output.chmod(0o755)
        try:
            validate_dependency_seed(publisher_output); raise AssertionError("writable dependency seed envelope was accepted")
        except RuntimeError as error:
            assert "entry is writable" in str(error), f"sealed-publisher: writable envelope diagnostic: {error}"
        (publisher_output / "extra").write_bytes(b"extra"); publisher_output.chmod(0o555)
        try:
            validate_dependency_seed(publisher_output); raise AssertionError("dependency seed envelope with extra entry was accepted")
        except RuntimeError as error:
            assert "envelope entries mismatch" in str(error), f"sealed-publisher: extra entry diagnostic: {error}"
        publisher_output.chmod(0o755); (publisher_output / "extra").unlink(); publisher_output.chmod(0o555)
        publisher_seal = publisher_output / "SEAL.json"; original_seal = publisher_seal.read_bytes(); publisher_seal.chmod(0o644); publisher_seal.write_bytes(b"{}\n"); publisher_seal.chmod(0o444)
        try:
            validate_dependency_seed(publisher_output); raise AssertionError("dependency seed seal mismatch was accepted")
        except RuntimeError as error:
            assert "seal mismatch" in str(error), f"sealed-publisher: seal mismatch diagnostic: {error}"
        publisher_seal.chmod(0o644); publisher_seal.write_bytes(original_seal); publisher_seal.chmod(0o444)
        assert validate_dependency_seed(publisher_output)["manifestSha256"] == published["manifestSha256"], "sealed-publisher: restored seal"
        collision_parent = publisher_root / "collision-staging"; collision_parent.mkdir(mode=0o700)
        collision_staged = collision_parent / "snapshot"; clone_tree(publisher_output, collision_staged)
        published_before_collision = validate_dependency_seed(publisher_output)
        try:
            atomic_publish_dependency_seed(collision_staged, publisher_output)
            raise AssertionError("dependency seed destination collision was accepted")
        except RuntimeError as error:
            assert str(error) == f"dependency seed output already exists: {publisher_output}", f"sealed-publisher: collision diagnostic: {error}"
        assert validate_dependency_seed(publisher_output)["manifestSha256"] == published_before_collision["manifestSha256"], "sealed-publisher: collision mutated output"
        assert collision_staged.exists(), "sealed-publisher: collision consumed staged source"
        make_tree_writable(publisher_output); make_tree_writable(collision_staged); publisher_marker.chmod(0o644); publisher_marker.unlink()
        gradle_seed_source = base / "gradle-source"; gradle_seed_source.mkdir()
        gradle_wrapper_source = base / "gradle-wrapper"; gradle_wrapper_source.mkdir()
        maven_seed_source = base / "maven-source"; maven_seed_source.mkdir()
        (gradle_seed_source / "artifact.bin").write_bytes(b"gradle")
        (gradle_wrapper_source / "distribution.zip.ok").write_bytes(b"wrapper")
        (maven_seed_source / "artifact.pom").write_bytes(b"maven")
        dependency_seed = freeze_dependency_seed(base / "dependency-seed", gradle_seed_source, gradle_wrapper_source, maven_seed_source)
        assert validate_dependency_seed(base / "dependency-seed")["manifestSha256"] == dependency_seed["manifestSha256"], "sealed-freeze: initial validation"
        (gradle_seed_source / "artifact.bin").write_bytes(b"changed")
        assert validate_dependency_seed(base / "dependency-seed")["manifestSha256"] == dependency_seed["manifestSha256"], "sealed-freeze: host mutation isolation"
        snapshot_artifact = Path(dependency_seed["root"]) / "gradle-modules/artifact.bin"
        snapshot_artifact.chmod(0o644)
        snapshot_artifact.write_bytes(b"changed")
        try:
            validate_dependency_seed(base / "dependency-seed")
            raise AssertionError("changed dependency seed was accepted")
        except RuntimeError as error:
            assert "published dependency seed entry is writable" in str(error), f"sealed-freeze: writable payload mutation diagnostic: {error}"
        snapshot_artifact.chmod(0o444)
        try:
            validate_dependency_seed(base / "dependency-seed")
            raise AssertionError("same-mode changed dependency seed was accepted")
        except RuntimeError as error:
            assert "dependency seed mismatch: gradle" in str(error), f"sealed-freeze: same-mode payload digest diagnostic: {error}"
        snapshot_artifact.chmod(0o644); snapshot_artifact.write_bytes(b"gradle"); snapshot_artifact.chmod(0o444)
        source = base / "source"; source.mkdir(); (source / "App.kt").write_text("fun app() = 1\n", encoding="utf-8")
        (source / "module").mkdir(); (source / "module/App.kt").write_text("fun moduleApp() = 1\n", encoding="utf-8")
        isolated = base / "isolated"
        state = base / "state"
        snapshot, state, environment = initialize_isolated_repository(
            source, isolated, state, dependency_seed, "GRADLE",
        )
        assert snapshot == source_digest(isolated)
        assert command_output(["git", "status", "--porcelain"], isolated) == ""
        assert command_output(["git", "rev-parse", "--verify", "HEAD"], isolated)
        assert not state.resolve().is_relative_to(isolated.resolve())
        assert (isolated / ".semantic-thread").is_dir() and not (isolated / ".semantic-thread").is_symlink()
        assert (isolated / ".gradle").is_dir() and not (isolated / ".gradle").is_symlink()
        assert environment["E04_DEPENDENCY_SEED_SHA256"] == dependency_seed["manifestSha256"]
        assert (isolated / ".gradle/caches/modules-2/artifact.bin").read_bytes() == b"gradle"
        assert (isolated / ".gradle/wrapper/dists/distribution.zip.ok").read_bytes() == b"wrapper"
        assert (isolated / ".semantic-thread/maven-repository/artifact.pom").read_bytes() == b"maven"
        repository_state = repository_owned_state_report(isolated, dependency_seed)
        assert repository_state["insideCheckout"] and repository_state["ignoredByGit"] and repository_state["regularDirectories"]
        assert repository_state["seedCloneSha256"] == {
            "gradleModules":dependency_seed["gradle"]["treeSha256"],
            "gradleWrapper":dependency_seed["gradleWrapper"]["treeSha256"],
            "mavenRepository":dependency_seed["maven"]["treeSha256"],
        }
        assert repository_state["currentTreeSha256"] == repository_state["seedCloneSha256"]
        assert not CLEW_ENV_DENY & set(environment)
        tainted_environment = {**environment, "GRADLE_OPTS":"forbidden", "MAVEN_ARGS":"forbidden", "JAVA_TOOL_OPTIONS":"forbidden", "ORG_GRADLE_PROJECT_probe":"forbidden"}
        clean_environment = sanitized_clew_environment(tainted_environment)
        assert set(clean_environment) <= CLEW_ENV_ALLOWLIST and not any(key.startswith("ORG_GRADLE_PROJECT_") for key in clean_environment)
        assert not CLEW_ENV_DENY & set(clean_environment)
        discovery_command = gradle_discovery_command(isolated)
        assert discovery_command[1:4] == ["--offline", "--gradle-user-home", str((isolated / ".gradle").resolve())]
        assert snapshot == source_digest(isolated) and git_status(isolated) == ""
        nested_workspace = prepare_project_state(isolated, {"projectRoot":"module","buildSystem":"MAVEN"}, state, dependency_seed)
        nested_state = repository_owned_state_report(nested_workspace, dependency_seed)
        assert nested_state["insideCheckout"] and nested_state["ignoredByGit"] and nested_state["regularDirectories"]
        assert nested_state["currentTreeSha256"] == nested_state["seedCloneSha256"]
        assert snapshot == source_digest(isolated) and git_status(isolated) == ""
        inline_request = compact({
            "schema":"typed-goal-binding-request/0.1", "compilation":":unit/main", "hints":[],
            "goal":{"schema":"typed-semantic-goal/0.1","baseRevision":"revision",
                "variables":{"context":"CALLABLE","transformer":"CALLABLE","edge":"VALUE_EDGE"},
                "operators":[{"operator":"MAP_EDGE","operands":["context","transformer","edge"]}]},
        })
        positive_error, _ = validate_inline_typed_request(inline_request, ":unit/main", "revision", typed_catalog)
        assert positive_error is None
        unused_variable_request = json.loads(inline_request)
        unused_variable_request["goal"]["variables"]["unused"] = "CALLABLE"
        unused_error, _ = validate_inline_typed_request(compact(unused_variable_request), ":unit/main", "revision", typed_catalog)
        assert unused_error == "INVALID_INLINE_REQUEST"
        typed_body = f"/opt/frozen/clew prove typed-goal --repo . --compilation :unit/main --request-json {shlex.quote(inline_request)}"
        typed_command = [{
            "command":f"/bin/zsh -lc {shlex.quote(typed_body)}",
            "output":'{"schema":"typed-goal-binding-decision/0.1","status":"REFUSED"}', "exitCode":0,
        }]
        request_records: list[dict[str, Any]] = []
        flags, _ = audit("codeclew", typed_command, "a", "a", frozen_clew, isolated, ":unit/main", "revision", request_records, typed_catalog)
        assert flags == []
        assert request_records[0]["sha256"] == sha_bytes(inline_request.encode())
        invalid_goal_command = [{**typed_command[0], "output":'{"reason":"INVALID_GOAL","schema":"typed-goal-binding-decision/0.1","status":"REFUSED"}'}]
        flags, _ = audit("codeclew", invalid_goal_command, "a", "a", frozen_clew, isolated, ":unit/main", "revision", [], typed_catalog)
        assert flags == ["CODECLEW_PROOF_NOT_USED", "NON_SUBSTANTIVE_TOOL_OUTPUT"]
        unsupported_request = compact({
            "schema":typed_catalog["requestSchema"], "compilation":":unit/main", "hints":[],
            "goal":{"schema":typed_catalog["goalSchema"], "baseRevision":"revision",
                "variables":{"DECLARATION":"DECLARATION","OVERRIDE":"DECLARATION","CALL_SITE":"DECLARATION"},
                "operators":[{"operator":"NULL_HANDLES","operands":["DECLARATION","OVERRIDE","CALL_SITE"]}]},
        })
        unsupported_error, _ = validate_inline_typed_request(unsupported_request, ":unit/main", "revision", typed_catalog)
        assert unsupported_error is None
        unsupported_body = f"/opt/frozen/clew prove typed-goal --repo . --compilation :unit/main --request-json {shlex.quote(unsupported_request)}"
        unsupported_command = [{"command":f"/bin/zsh -lc {shlex.quote(unsupported_body)}","output":'{"reason":"UNSUPPORTED_CONSTRAINT_DOMAIN","schema":"typed-goal-binding-decision/0.1","status":"REFUSED"}',"exitCode":0}]
        unsupported_records: list[dict[str, Any]] = []
        flags, _ = audit("codeclew", unsupported_command, "a", "a", frozen_clew, isolated, ":unit/main", "revision", unsupported_records, typed_catalog)
        assert flags == [] and unsupported_records[0]["decision"]["reason"] == "UNSUPPORTED_CONSTRAINT_DOMAIN"
        unsupported_model = {"schema":"semantic-editing-e04-model-output/0.1","status":"REFUSED","inferredFamily":FAMILIES[0],"goal":None,"ambiguity":None,"refusal":{"code":"UNSUPPORTED_FAMILY"}}
        assert validate_proof_model_link(unsupported_model, unsupported_records, typed_catalog, refusal_adapter) == []
        malformed_map = compact({
            "schema":"typed-goal-binding-request/0.1", "compilation":":unit/main", "hints":[],
            "goal":{"schema":"typed-semantic-goal/0.1","baseRevision":"revision",
                "variables":{"left":"CALLABLE"},
                "operators":[{"operator":"MAP_EDGE","operands":["left"]}]},
        })
        error, _ = validate_inline_typed_request(malformed_map, ":unit/main", "revision", typed_catalog)
        assert error == "INVALID_INLINE_REQUEST"
        malformed_body = "/opt/frozen/clew prove typed-goal --repo . --compilation :unit/main --request-json '{}'"
        malformed = [{"command":f"/bin/zsh -lc {shlex.quote(malformed_body)}","output":"invalid","exitCode":2}]
        flags, _ = audit("codeclew", malformed, "a", "a", frozen_clew, isolated, ":unit/main", "revision", [], typed_catalog)
        assert flags == ["CODECLEW_PROOF_NOT_USED", "INVALID_INLINE_REQUEST"]
        dual_body = typed_body + f" --request-json {shlex.quote(inline_request)}"
        dual = [{"command":f"/bin/zsh -lc {shlex.quote(dual_body)}","output":"invalid","exitCode":2}]
        flags, _ = audit("codeclew", dual, "a", "a", frozen_clew, isolated, ":unit/main", "revision", [], typed_catalog)
        assert flags == ["CODECLEW_PROOF_NOT_USED", "INVALID_TOOL_ARGUMENTS"]
        refusal_record = {**request_records[0], "decision":{"schema":"typed-goal-binding-decision/0.1","status":"REFUSED","reason":"NO_COMPATIBLE_BINDINGS"}}
        refusal_model = {"schema":"semantic-editing-e04-model-output/0.1","status":"REFUSED","inferredFamily":FAMILIES[0],"goal":None,"ambiguity":None,"refusal":{"code":"INCOMPLETE_SEMANTIC_EVIDENCE"}}
        assert validate_proof_model_link(refusal_model, [refusal_record], typed_catalog, refusal_adapter) == []
        unrelated_refusal = {**refusal_model, "refusal":{"code":"BUSINESS_ORACLE_ABSENT"}}
        assert validate_proof_model_link(unrelated_refusal, [refusal_record], typed_catalog, refusal_adapter) == ["PROOF_MODEL_REFUSAL_MISMATCH"]
        status_mismatch = {**refusal_model, "status":"BOUND", "goal":{"bindings":[],"obligations":[],"evidenceAnchors":[],"oracleClass":"DERIVED"}, "refusal":None}
        assert validate_proof_model_link(status_mismatch, [refusal_record], typed_catalog, refusal_adapter) == ["PROOF_MODEL_STATUS_MISMATCH"]
        bound_request_json = compact({
            "schema":"typed-goal-binding-request/0.1", "compilation":":unit/main", "hints":[],
            "goal":{"schema":"typed-semantic-goal/0.1","baseRevision":"revision",
                "variables":{"VALUE_EDGE":"VALUE_EDGE"},
                "operators":[{"operator":"BIND_UNIQUE","operands":["VALUE_EDGE"]}]},
        })
        bound_record = {"exitCode":0,"canonicalJson":bound_request_json,"decision":{"schema":"typed-goal-binding-decision/0.1","status":"BOUND","proof":{"bindings":{"VALUE_EDGE":"p.edge"}}}}
        bound_model = {"schema":"semantic-editing-e04-model-output/0.1","status":"BOUND","inferredFamily":FAMILIES[0],"goal":{"bindings":[{"role":"VALUE_EDGE","symbol":"p.other"}],"obligations":["x"],"evidenceAnchors":["x"],"oracleClass":"DERIVED"},"ambiguity":None,"refusal":None}
        assert validate_proof_model_link(bound_model, [bound_record], typed_catalog, refusal_adapter) == ["PROOF_MODEL_BINDINGS_MISMATCH"]
        path_body = "/opt/frozen/clew prove typed-goal --repo . --compilation :unit/main --request request.json"
        path_call = [{"command":f"/bin/zsh -lc {shlex.quote(path_body)}","output":"invalid","exitCode":2}]
        flags, _ = audit("codeclew", path_call, "a", "a", frozen_clew, isolated, ":unit/main", "revision", [], typed_catalog)
        assert flags == ["CODECLEW_PROOF_NOT_USED", "INVALID_TOOL_ARGUMENTS"]
        planned = plan_packets(dry, None, False, False)
        assert len(planned["runs"]) == 126
        assert len(list((dry / "planned").glob("*/run-packet.json"))) == 126
        experiment, results = base / "experiment", base / "results"
        for index, slot in enumerate(spec["slots"]):
            task_id = f"e04-{index:016x}"; commitment = f"commitment-{index}"
            public_dir = experiment / "agent" / task_id; public_dir.mkdir(parents=True)
            (public_dir / "repository").mkdir()
            if slot["buildSystem"].upper() == "MAVEN":
                (public_dir / "repository/pom.xml").write_text("""<project><modelVersion>4.0.0</modelVersion><groupId>fixture</groupId><artifactId>unit</artifactId><version>1</version><properties><kotlin.version>2.1.21</kotlin.version></properties><dependencies><dependency><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-test-junit5</artifactId><version>${kotlin.version}</version><scope>test</scope></dependency></dependencies><build><plugins><plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>${kotlin.version}</version></plugin></plugins></build></project>""", encoding="utf-8")
            public = {"schema":"semantic-editing-e04-public-task/0.1","taskId":task_id,"buildSystem":slot["buildSystem"].upper(),"kotlinVersion":"2.1.21","task":"Update the named target.","repository":"repository","sourceSnapshotSha256":"0"*64,"buildCommand":[],"controllerManifestCommitment":commitment}
            public_path = public_dir / "task-manifest.json"; write_json(public_path, public)
            family_spec = next(item for item in spec["families"] if item["id"] == slot["family"])
            role, symbol = "TRANSFORMER", f"p{index}.target"; bindings = [f"{role}={symbol}"]
            expected = {"positive":"BOUND","ambiguous":"AMBIGUOUS","must-refuse":"REFUSED"}[slot["variant"]]
            alternatives = [[f"{role}=p{index}.a"], [f"{role}=p{index}.b"]]
            controller = {"schema":"semantic-editing-e04-controller/0.2","taskId":task_id,"seriesId":"a"*64,"controllerSeedCommitment":"b"*64,"slot":slot,"seed":index,"binderFreeze":BASE,"binderTreeSha256":"1"*64,"populationSha256":POP_SHA,"requiredBindings":bindings,"requiredObligations":family_spec["requiredObligations"],"expectedOutcome":expected,"ambiguousChoices":alternatives if expected=="AMBIGUOUS" else [],"refusalReason":"UNSUPPORTED_FAMILY" if expected=="REFUSED" else None,"commitments":[],"publicManifestSha256":sha_file(public_path),"commitment":commitment}
            controller["expectedOracleClass"] = "EXTERNAL_SPEC" if expected == "BOUND" else None
            write_json(experiment / "controller" / task_id / "manifest.json", controller)
            for arm in ARMS:
                if expected == "BOUND": model = {"schema":"semantic-editing-e04-model-output/0.1","status":"BOUND","inferredFamily":slot["family"],"goal":{"bindings":[{"role":role,"symbol":symbol}],"obligations":family_spec["requiredObligations"],"evidenceAnchors":[symbol],"oracleClass":"EXTERNAL_SPEC"},"ambiguity":None,"refusal":None}
                elif expected == "AMBIGUOUS": model = {"schema":"semantic-editing-e04-model-output/0.1","status":"AMBIGUOUS","inferredFamily":slot["family"],"goal":None,"ambiguity":{"choices":[{"bindings":[{"role":role,"symbol":choice[0].split("=",1)[1]}]} for choice in alternatives]},"refusal":None}
                else: model = {"schema":"semantic-editing-e04-model-output/0.1","status":"REFUSED","inferredFamily":slot["family"],"goal":None,"ambiguity":None,"refusal":{"code":"UNSUPPORTED_FAMILY"}}
                protocol_valid = not (index == 2 and arm == "default")
                packet = {"runId":f"{task_id}--{arm}","taskId":task_id,"arm":arm,"executionStatus":"OK","infrastructureValid":True,"protocolValid":protocol_valid,"auditFlags":[] if protocol_valid else ["COMPOUND_COMMAND"],"modelOutput":model,"wallMilliseconds":1,"contextBytes":1,"goalBytes":1 if expected=="BOUND" else 0,"navigationCalls":1,"metrics":{"turns":1,"actionCalls":1,"inputTokens":2,"cachedInputTokens":1,"outputTokens":1,"noncachedTokens":2,"nativeTokenTelemetryAvailable":True}}
                append_jsonl(results / "runs.jsonl", packet)
        synthetic_tasks = discover_public(experiment)
        synthetic_maven_plan = public_maven_seed_plan(experiment)
        assert len(synthetic_maven_plan["tasks"]) == 21
        assert synthetic_maven_plan["declaredDependencies"] == ["org.jetbrains.kotlin:kotlin-test-junit5:2.1.21"]
        assert synthetic_maven_plan["declaredPlugins"] == ["org.jetbrains.kotlin:kotlin-maven-plugin:2.1.21"]
        selected_one, denominator_one = preflight_selection(synthetic_tasks, 1)
        assert [public["taskId"] for _, public in selected_one] == ["e04-0000000000000000"]
        assert denominator_one == {"GRADLE":1,"MAVEN":0}
        selected_two, denominator_two = preflight_selection(synthetic_tasks, 2)
        assert [public["taskId"] for _, public in selected_two] == ["e04-0000000000000000", "e04-0000000000000001"]
        assert denominator_two == {"GRADLE":1,"MAVEN":1}
        selected_full, denominator_full = preflight_selection(synthetic_tasks, 0)
        assert len(selected_full) == 42 and denominator_full == {"GRADLE":21,"MAVEN":21}
        diagnostic_path = base / "preflight-postcondition-failure.json"
        diagnostic_rows = [{"taskId":"e04-0000000000000000","buildSystem":"GRADLE","infrastructureValid":True,"astReady":True,"codeclewProjectReady":True}]
        diagnostic_errors = preflight_aggregate_errors(diagnostic_rows, {"GRADLE":1,"MAVEN":1}, True)
        try:
            publish_preflight_report(diagnostic_path, {"schema":"semantic-editing-e04-preflight/0.2","tasks":1,"rows":diagnostic_rows}, diagnostic_errors)
            raise AssertionError("aggregate failure was published as success")
        except RuntimeError as error:
            assert "preflight aggregate postcondition failed" in str(error)
        diagnostic_packet = load(diagnostic_path)
        assert diagnostic_packet["status"] == "PREFLIGHT_POSTCONDITION_FAILED"
        assert diagnostic_packet["rows"] == diagnostic_rows and diagnostic_packet["tasks"] == 1
        assert diagnostic_packet["aggregatePostconditionErrors"] == diagnostic_errors
        row_failure_path = base / "preflight-row-failure.json"
        row_failure = publish_preflight_row_failure(row_failure_path, diagnostic_rows, diagnostic_rows[0]["taskId"], ["offlineHermetic"])
        assert row_failure["status"] == "PREFLIGHT_ROW_FAILED" and row_failure["completedRows"] == 1
        assert load(row_failure_path) == row_failure and row_failure["errors"] == ["offlineHermetic"]
        self_test_semantic_corpus=base/"semantic-corpus-fixture"; self_test_semantic_corpus.write_bytes(b"semantic-corpus-fixture\n")
        setup_failure_path=base/"preflight-setup-failure.json"
        try:
            preflight(argparse.Namespace(experiment_root=str(experiment),output=str(setup_failure_path),codeclew_bin=str(base/"missing-clew"),dependency_seed=str(base/"dependency-seed"),semantic_corpus_bin=str(self_test_semantic_corpus),max_tasks=0,no_freeze_check=False,diagnostic_freeze=None,full_readiness=True,deadline_seconds=1))
            raise AssertionError("preflight setup failure was accepted")
        except RuntimeError:
            setup_packet=load(setup_failure_path); assert setup_packet["status"]=="PREFLIGHT_SETUP_FAILED" and setup_packet["tasks"]==0
        missing_root_path=base/"preflight-missing-root.json"
        try:
            preflight(argparse.Namespace(experiment_root=str(experiment),output=str(missing_root_path),codeclew_bin=str(self_test_clew),dependency_seed=str(base/"dependency-seed"),semantic_corpus_bin=str(self_test_semantic_corpus),max_tasks=0,no_freeze_check=False,deadline_seconds=1,readiness_store=str(base/"missing-readiness-store"),readiness_root="DIAGNOSTIC_FULL_PREFLIGHT_START_READY",readiness_graph=str(READINESS_GRAPH)))
            raise AssertionError("preflight tools reached without readiness root")
        except RuntimeError:
            missing_root_packet=load(missing_root_path); assert missing_root_packet["status"]=="PREFLIGHT_SETUP_FAILED" and missing_root_packet["tasks"]==0
        bypass_path=base/"preflight-freeze-bypass.json"; bypass_store=base/"freeze-bypass-readiness-store"
        try:
            preflight(argparse.Namespace(experiment_root=str(experiment),output=str(bypass_path),codeclew_bin=str(self_test_clew),dependency_seed=str(base/"dependency-seed"),semantic_corpus_bin=str(self_test_semantic_corpus),max_tasks=0,no_freeze_check=True,deadline_seconds=1,readiness_store=str(bypass_store),readiness_root="DIAGNOSTIC_FULL_PREFLIGHT_START_READY",readiness_graph=str(READINESS_GRAPH)))
            raise AssertionError("preflight freeze bypass was accepted")
        except RuntimeError as error:
            bypass_packet=load(bypass_path); assert "forbids --no-freeze-check" in str(error)
            assert bypass_packet["status"]=="PREFLIGHT_SETUP_FAILED" and bypass_packet["tasks"]==0
            assert not (bypass_store/"current"/"DIAGNOSTIC_FULL_PREFLIGHT_42.json").exists()
        captured_failures=[]; original_failed_issuer=readiness.issue_failed_preflight
        try:
            readiness.issue_failed_preflight=lambda *values: captured_failures.append(values[3]) or "failed-receipt"
            failure_args=argparse.Namespace(readiness_graph=str(READINESS_GRAPH),readiness_store=str(base/"failure-store"),dependency_seed=str(base/"dependency-seed"),semantic_corpus_bin=str(self_test_semantic_corpus))
            failure_gate={"diagnosticFreezeArtifactHash":"1"*64,"receiptHash":"2"*64}; failure_seed={"manifestSha256":"3"*64}; failure_catalog={"binarySha256":"4"*64,"catalogSha256":"5"*64}; failure_ast={"binarySha256":"6"*64}; failure_public={"taskId":"failure-task"}
            for injected_stage in ("COPY_SNAPSHOT","AST_STATS"):
                failure_output=base/f"preflight-injected-{injected_stage}.json"
                try:
                    with preflight_row_failure_guard(failure_args,failure_output,experiment,self_test_clew,[],failure_public,failure_gate,failure_seed,failure_catalog,failure_ast,time.monotonic()) as set_stage:
                        set_stage(injected_stage); raise RuntimeError(f"injected-{injected_stage}")
                    raise AssertionError("injected preflight row failure was accepted")
                except RuntimeError as error:
                    assert str(error)==f"injected-{injected_stage}"
                failure_packet=load(failure_output)
                assert failure_packet["status"]=="PREFLIGHT_ROW_FAILED" and failure_packet["stage"]==injected_stage and failure_packet["stoppedAt"]=="failure-task"
                assert failure_packet["modelCalls"]==0 and failure_packet["controllerReads"]==0 and failure_packet["errorDetailSha256"]==sha_bytes(f"injected-{injected_stage}".encode())
            assert [packet["stage"] for packet in captured_failures]==["COPY_SNAPSHOT","AST_STATS"]
        finally:
            readiness.issue_failed_preflight=original_failed_issuer
        readiness_regressions=readiness.synthetic_regressions(base/"readiness-regressions")
        try:
            run_canary(argparse.Namespace(experiment_root=str(experiment),output=str(base/"forbidden-run"),codeclew_bin=str(self_test_clew),dependency_seed=str(base/"dependency-seed"),semantic_corpus_bin=str(self_test_semantic_corpus),max_workers=1,dry_run=False,readiness_store="",readiness_root="DIAGNOSTIC_CANARY_START_READY",readiness_graph=str(READINESS_GRAPH),freeze_manifest=str(FREEZE_MANIFEST)))
            raise AssertionError("model run reached executor without readiness root")
        except RuntimeError as error:
            assert "run-canary requires DIAGNOSTIC_CANARY_START_READY" in str(error)
        with contextlib.redirect_stdout(io.StringIO()):
            internal_args=argparse.Namespace(experiment_root=str(experiment),output=str(results),semantic_corpus_bin=str(self_test_semantic_corpus),_skip_authority_for_selftest=True)
            _judge_authorized(internal_args,experiment,results)
            _summarize_authorized(internal_args,experiment)
        summary = load(results / "summary.json")
        assert all(summary["arms"][arm]["applicability"] == 1 for arm in ARMS)
        assert all(summary["arms"][arm]["semanticCorrectRuns"] == 42 for arm in ARMS)
        assert summary["arms"]["default"]["acceptedRuns"] == 41
        assert summary["arms"]["default"]["protocolInvalidRuns"] == 1
        assert summary["arms"]["default"]["infrastructureInvalidRuns"] == 0
        assert summary["arms"]["default"]["modelOutputInvalidRuns"] == 0
        triplet_rows = [{"taskId":"t","arm":arm,"taskArmOrder":index,"runId":f"t--{arm}"} for index, arm in enumerate(ARMS)]
        calls: list[str] = []
        def failing_executor(row: dict[str, Any]) -> dict[str, Any]:
            calls.append(row["arm"])
            return {**row, "infrastructureValid":False, "executionStatus":"FAILED"}
        persisted_packets=[]
        circuit_packets = execute_triplet(triplet_rows, failing_executor,persist=persisted_packets.append)
        assert calls == [triplet_rows[0]["arm"]]
        assert [packet["executionStatus"] for packet in circuit_packets] == ["FAILED", "CIRCUIT_BREAKER", "CIRCUIT_BREAKER"]
        assert persisted_packets==circuit_packets
        timeout_contract=model_timeout_fields(True,True)
        assert timeout_contract=={"modelTimedOut":True,"timeoutSeconds":600,"partialArtifactsPresent":True}
        bound_request = {**request_records[0]}
        r7_base = {
            **triplet_rows[0], "arm":"codeclew", "infrastructureValid":True,
            "modelOutputValid":True, "protocolValid":True, "navigationCalls":1,
            "auditFlags":[], "sourceBeforeSha256":"same", "sourceAfterSha256":"same",
            "controllerManifestCommitment":"commitment", "promptSha256":"prompt",
            "publicManifestSha256":"public", "contextBytes":100, "goalBytes":100,
            "metrics":{"nativeTokenTelemetryAvailable":True,"turns":1,"actionCalls":1},
        }
        bound_request["packetBindingSha256"] = sha_bytes(compact({"promptSha256":"prompt","taskId":"t","publicManifestSha256":"public","requestSha256":bound_request["sha256"]}).encode())
        r7_base["typedGoalRequests"] = [bound_request]
        assert r7_breaker_reasons(r7_base, "commitment") == []
        missing_telemetry = {**r7_base, "metrics":{"nativeTokenTelemetryAvailable":False}}
        assert r7_breaker_reasons(missing_telemetry, "commitment") == ["NATIVE_TELEMETRY_MISSING"]
        invalid_schema = {**r7_base, "modelOutputValid":False}
        assert r7_breaker_reasons(invalid_schema, "commitment") == ["MODEL_SCHEMA_INVALID"]
        unused_tool = {**r7_base, "auditFlags":["CODECLEW_PROOF_NOT_USED"]}
        assert r7_breaker_reasons(unused_tool, "commitment") == ["SPECIALIZED_TOOL_NOT_PROVEN"]
        action_overage = {**r7_base, "metrics":{**r7_base["metrics"], "actionCalls":R7_MAX_ACTION_CALLS + 1}}
        assert r7_canary_reasons(action_overage, "commitment") == ["ACTION_CEILING_EXCEEDED"]
        r7_calls: list[str] = []
        def r7_executor(row: dict[str, Any]) -> dict[str, Any]:
            r7_calls.append(row["arm"]); return {**missing_telemetry, **row}
        r7_packets = execute_triplet(triplet_rows, r7_executor, lambda packet: r7_breaker_reasons(packet, "commitment"))
        assert len(r7_calls) == 1 and [packet["executionStatus"] for packet in r7_packets[1:]] == ["CIRCUIT_BREAKER", "CIRCUIT_BREAKER"]
        retained_manifest = experiment / "agent/e04-0000000000000000/task-manifest.json"
        retained_rows = [{**row, "publicManifest":str(retained_manifest)} for row in triplet_rows]
        retained_packets = [{**r7_base, **row, "controllerManifestCommitment":"commitment-0"} for row in retained_rows]
        for packet in retained_packets:
            request = {**bound_request}
            request["packetBindingSha256"] = sha_bytes(compact({"promptSha256":"prompt","taskId":"t","publicManifestSha256":"public","requestSha256":request["sha256"]}).encode())
            packet["typedGoalRequests"] = [request]
        assert validate_retained_canaries([retained_rows], retained_packets) == {}
        try:
            validate_retained_canaries([retained_rows], retained_packets[:-1])
            raise AssertionError("incomplete retained canary was accepted")
        except RuntimeError as error:
            assert "incomplete" in str(error)
        retained_rows_2 = [{**row, "taskId":"u", "runId":f"u--{row['arm']}"} for row in retained_rows]
        retained_packets_2 = []
        for row in retained_rows_2:
            packet = {**r7_base, **row, "controllerManifestCommitment":"commitment-0", "metrics":{**r7_base["metrics"],"noncachedTokens":8_000}}
            request = {**bound_request}
            request["packetBindingSha256"] = sha_bytes(compact({"promptSha256":"prompt","taskId":"u","publicManifestSha256":"public","requestSha256":request["sha256"]}).encode())
            packet["typedGoalRequests"] = [request]; retained_packets_2.append(packet)
        token_packets = [{**packet, "metrics":{**packet["metrics"],"noncachedTokens":8_000}} for packet in retained_packets + retained_packets_2]
        token_ceiling = validate_retained_canaries([retained_rows, retained_rows_2], token_packets)
        assert token_ceiling["__aggregate__"] == ["AGGREGATE_NONCACHED_TOKEN_CEILING_EXCEEDED"]
        action_packets = []
        for packet in retained_packets + retained_packets_2:
            action_packets.append({**packet, "metrics":{**packet["metrics"],"noncachedTokens":1,"actionCalls":3}})
        action_ceiling = validate_retained_canaries([retained_rows, retained_rows_2], action_packets)
        assert action_ceiling["__aggregate__"] == ["AGGREGATE_ACTION_CEILING_EXCEEDED"]
        coverage_report = self_test_zero_model_coverage(base, self_test_clew, typed_catalog)
        assert coverage_report["compositionIds"] >= 5
        make_tree_writable(base / "dependency-seed")
    print(compact({"status":"SELF_TEST_PASSED","matrixRuns":126,"noncachedTokens":67,"eventFixtures":len(load(fixture_path)),"astReadinessFixtures":len(ast_fixture_cases),"astArtifactCounterexamples":ast_artifact_counterexamples,"projectModelFixtures":len(project_fixtures),"coveragePackets":coverage_report["packets"],"coverageCounterexamples":coverage_report["counterexamples"],"readinessCounterexamples":readiness_regressions["counterexamples"]}))


def main() -> None:
    parser = argparse.ArgumentParser(); sub = parser.add_subparsers(dest="command", required=True)
    seed = sub.add_parser("freeze-dependency-seed"); seed.add_argument("--output", required=True); seed.add_argument("--gradle-cache", required=True); seed.add_argument("--gradle-wrapper", required=True); seed.add_argument("--maven-repo", required=True)
    augment = sub.add_parser("augment-dependency-seed"); augment.add_argument("--base-seed", required=True); augment.add_argument("--experiment-root", required=True); augment.add_argument("--output", required=True); augment.add_argument("--repository", default="https://repo.maven.apache.org/maven2")
    freeze = sub.add_parser("freeze-check"); freeze.add_argument("--no-tool-check", action="store_true"); freeze.add_argument("--dependency-seed"); freeze.add_argument("--codeclew-bin", required=True)
    plan = sub.add_parser("plan"); plan.add_argument("--experiment-root"); plan.add_argument("--output", required=True); plan.add_argument("--no-tool-check", action="store_true"); plan.add_argument("--dependency-seed"); plan.add_argument("--codeclew-bin", required=True); plan.add_argument("--diagnostic-freeze")
    preflight_parser = sub.add_parser("preflight"); preflight_parser.add_argument("--experiment-root", required=True); preflight_parser.add_argument("--output", required=True); preflight_parser.add_argument("--codeclew-bin", required=True); preflight_parser.add_argument("--dependency-seed", required=True); preflight_parser.add_argument("--max-tasks", type=int, default=0); preflight_parser.add_argument("--no-freeze-check", action="store_true")
    preflight_parser.add_argument("--deadline-seconds", type=float, default=2700); preflight_parser.add_argument("--readiness-store",required=True); preflight_parser.add_argument("--readiness-root",required=True); preflight_parser.add_argument("--readiness-graph",default=str(READINESS_GRAPH)); preflight_parser.add_argument("--semantic-corpus-bin",required=True)
    def add_run_arguments(command: argparse.ArgumentParser) -> None:
        command.add_argument("--experiment-root",required=True); command.add_argument("--output",required=True); command.add_argument("--diagnostic-preflight-report",required=True); command.add_argument("--diagnostic-audit-receipt",required=True); command.add_argument("--codeclew-bin",required=True); command.add_argument("--dependency-seed",required=True); command.add_argument("--semantic-corpus-bin",required=True); command.add_argument("--max-workers",type=int,default=3); command.add_argument("--dry-run",action="store_true"); command.add_argument("--readiness-store",required=True); command.add_argument("--readiness-root",required=True); command.add_argument("--readiness-graph",default=str(READINESS_GRAPH))
    canary=sub.add_parser("run-canary"); add_run_arguments(canary)
    run = sub.add_parser("run"); add_run_arguments(run); run.add_argument("--diagnostic-experiment-root",required=True); run.add_argument("--diagnostic-output-root",required=True)
    def add_downstream_arguments(command: argparse.ArgumentParser) -> None:
        command.add_argument("--experiment-root",required=True); command.add_argument("--diagnostic-experiment-root",required=True); command.add_argument("--diagnostic-output-root",required=True); command.add_argument("--diagnostic-preflight-report",required=True); command.add_argument("--diagnostic-audit-receipt",required=True); command.add_argument("--output",required=True); command.add_argument("--codeclew-bin",required=True); command.add_argument("--dependency-seed",required=True); command.add_argument("--semantic-corpus-bin",required=True); command.add_argument("--readiness-store",required=True); command.add_argument("--readiness-root",required=True); command.add_argument("--readiness-graph",default=str(READINESS_GRAPH))
    judge_parser = sub.add_parser("judge"); add_downstream_arguments(judge_parser)
    summary = sub.add_parser("summarize"); add_downstream_arguments(summary)
    readiness_parser=sub.add_parser("readiness"); readiness_parser.add_argument("--readiness-store",required=True); readiness_parser.add_argument("--diagnostic-experiment-root",required=True); readiness_parser.add_argument("--r1-experiment-root"); readiness_parser.add_argument("--diagnostic-output-root",required=True); readiness_parser.add_argument("--r1-output-root"); readiness_parser.add_argument("--diagnostic-preflight-report"); readiness_parser.add_argument("--diagnostic-audit-receipt"); readiness_parser.add_argument("--codeclew-bin",required=True); readiness_parser.add_argument("--dependency-seed",required=True); readiness_parser.add_argument("--semantic-corpus-bin"); readiness_parser.add_argument("--agent-seed-file"); readiness_parser.add_argument("--controller-seed-file"); readiness_parser.add_argument("--series-nonce-file"); readiness_parser.add_argument("--graph",default=str(READINESS_GRAPH))
    readiness_actions=readiness_parser.add_subparsers(dest="readiness_action",required=True)
    readiness_actions.add_parser("plan"); readiness_actions.add_parser("explain")
    prepare_readiness=readiness_actions.add_parser("prepare"); prepare_readiness.add_argument("--node",required=True)
    verify_readiness=readiness_actions.add_parser("verify"); verify_readiness.add_argument("--node",required=True); verify_readiness.add_argument("--preflight-report")
    audit_readiness=readiness_actions.add_parser("import-audit"); audit_readiness.add_argument("--audit-receipt",required=True)
    annotation_readiness=readiness_actions.add_parser("import-annotation"); annotation_readiness.add_argument("--annotation",required=True); annotation_readiness.add_argument("--annotator-id",required=True,choices=(readiness.ANNOTATOR_A_ID,readiness.ANNOTATOR_B_ID))
    coverage_audit_readiness=readiness_actions.add_parser("import-coverage-audit"); coverage_audit_readiness.add_argument("--coverage-audit",required=True)
    readiness_actions.add_parser("product-coverage")
    root_readiness=readiness_actions.add_parser("root"); root_readiness.add_argument("--root",required=True)
    sub.add_parser("self-test")
    args = parser.parse_args()
    if args.command == "freeze-dependency-seed": print(compact(freeze_dependency_seed(Path(args.output), Path(args.gradle_cache), Path(args.gradle_wrapper), Path(args.maven_repo))))
    elif args.command == "augment-dependency-seed": print(compact(build_augmented_dependency_seed(Path(args.base_seed), Path(args.experiment_root), Path(args.output), args.repository)))
    elif args.command == "freeze-check":
        dependency_seed = validate_dependency_seed(Path(args.dependency_seed)) if args.dependency_seed else None
        typed_catalog = load_typed_goal_catalog(Path(args.codeclew_bin).resolve())
        refusal_adapter = load_refusal_adapter(typed_catalog)
        print(compact(frozen_checks(not args.no_tool_check, True, dependency_seed, typed_catalog, refusal_adapter)))
    elif args.command == "plan":
        dependency_seed = validate_dependency_seed(Path(args.dependency_seed)) if args.dependency_seed else None
        typed_catalog = load_typed_goal_catalog(Path(args.codeclew_bin).resolve())
        refusal_adapter = load_refusal_adapter(typed_catalog)
        print(compact({"status":"PLANNED","runs":len(plan_packets(Path(args.output),Path(args.experiment_root) if args.experiment_root else None,not args.no_tool_check,True,dependency_seed,typed_catalog,refusal_adapter,Path(args.diagnostic_freeze) if args.diagnostic_freeze else None)["runs"])}))
    elif args.command == "preflight": preflight(args)
    elif args.command == "run-canary": run_canary(args)
    elif args.command == "run": run_all(args)
    elif args.command == "judge": judge(args)
    elif args.command == "summarize": summarize(args)
    elif args.command == "readiness": print(compact(readiness.run_command(args,self_module())))
    else: self_test()


if __name__ == "__main__":
    try: main()
    except Exception as error:
        print(compact({"status":"ERROR","error":str(error)}), file=sys.stderr); raise SystemExit(2)
