#!/usr/bin/env python3
"""Run the private G1K Kotlin descriptor readiness gate without leaking locators."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import re
import signal
import stat
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import verify_thread_kotlin_descriptor_gate as checked_verifier


PRIVATE_CORPUS_SCHEMA = "codeclew-private-thread-contract-corpus/1.0"
PRIVATE_BENCHMARK_SCHEMA = "codeclew-private-kotlin-descriptor-benchmark/1.0"
PRIVATE_OUTPUT_SCHEMA = "codeclew-private-thread-kotlin-descriptor-gate/2.0"
COMPILER_AUTHORITY_SCHEMA = "codeclew-kotlin-descriptor-compiler-authority/1.0"
REVISION_AUTHORITY_SCHEMA = "codeclew-kotlin-descriptor-revision-authority/1.0"
FAILURE_AUTHORITY_SCHEMA = "codeclew-kotlin-descriptor-failure-authority/1.0"
FROZEN_AT = checked_verifier.FROZEN_AT
COMPILATION = ":/main"
INTENT = "verify compiler-backed Kotlin descriptor readiness"
KOTLIN_FACT_DOMAIN = "analysis:kotlin-semantic-facts"
CAS_OBJECT_SCHEMA = "codeclew-cas-object/2.0"
FACT_PAYLOAD_SCHEMA = "codeclew-kotlin-semantic-fact/3.0"
SUPPORTED_COMPILERS = {"2.3.0", "2.4.10"}
EXPECTED_CORPUS_DIGEST = checked_verifier.EXPECTED_PRIVATE_CORPUS_DIGEST
EXPECTED_BENCHMARK_DIGEST = checked_verifier.EXPECTED_BENCHMARK_DIGEST
EXPECTED_RESOURCE_BUDGETS = {
    "maxColdWallMsPerTask": 900_000,
    "maxContextCreatesPerTask": 1,
    "maxContextExpansionsPerTask": 1,
    "maxEvidenceBytesPerTask": 8_388_608,
    "maxQueryTermsPerTask": 16,
    "maxReturnedFactsPerTask": 128,
    "maxSelectedFilesPerTask": 12,
    "maxSourceWindowsPerTask": 24,
    "maxStdoutBytesPerTask": 65_536,
    "maxWarmWallMsPerTask": 60_000,
}
MAX_PRIVATE_CORPUS_BYTES = 256 * 1024
MAX_PRIVATE_BENCHMARK_BYTES = 4 * 1024 * 1024
MAX_CLEW_BYTES = 256 * 1024 * 1024
MAX_CLEW_STDOUT_BYTES = 256 * 1024
MAX_GIT_STDOUT_BYTES = 256 * 1024
GIT_OID = re.compile(r"^[0-9a-f]{40}(?:[0-9a-f]{24})?$")
SHA256 = checked_verifier.SHA256
SESSION_ID = re.compile(r"^session:(?:sha256:)?[0-9A-Za-z_-]{1,128}$")
CONTEXT_ID = re.compile(r"^context:sha256:[0-9a-f]{64}$")
SAFE_FAILURE = re.compile(r"^[A-Z][A-Z0-9_]{0,63}$")
SAFE_DECLARATION = re.compile(r"^[A-Za-z_][A-Za-z0-9_]{0,255}$")
GIT_BLOB_OID = re.compile(r"^[0-9a-f]{40}(?:[0-9a-f]{24})?$")
KOTLIN_FACT_KEY = re.compile(
    r"^kotlin:(metadata|file|descriptor|descriptor-boundary|relation|relation-boundary):"
    r"[0-9a-f]{64}$"
)
REPOSITORY_KEY = re.compile(r"^[0-9a-f]{64}$")


class GateError(RuntimeError):
    """A deliberately path-free gate failure."""

    def __init__(self, code: str):
        if SAFE_FAILURE.fullmatch(code) is None:
            code = "INTERNAL_GATE_FAILURE"
        super().__init__(code)
        self.code = code


@dataclass(frozen=True)
class Service:
    alias: str
    service_id: str
    repository: Path
    revision: str


@dataclass(frozen=True)
class Task:
    task_id: str
    pair_id: str
    provider: str
    consumer: str
    scenario: str


@dataclass(frozen=True)
class Corpus:
    frozen_at: str
    services: tuple[Service, ...]
    tasks: tuple[Task, ...]


@dataclass(frozen=True)
class OracleDeclaration:
    kind: str
    name: str


@dataclass(frozen=True)
class OracleNavigation:
    relative_file: str
    blob_oid: str
    declarations: tuple[OracleDeclaration, ...]


@dataclass(frozen=True)
class OracleSide:
    task_id: str
    role: str
    service_alias: str
    revision: str
    minimum_approved_files: int
    minimum_callable_descriptors: int
    minimum_type_descriptors: int
    navigations: tuple[OracleNavigation, ...]

    @property
    def key(self) -> str:
        return f"{self.task_id}:{self.role}"


@dataclass(frozen=True)
class Benchmark:
    authority_digest: str
    sides: tuple[OracleSide, ...]
    max_query_terms: int
    max_roots: int
    max_cold_seconds: int


@dataclass(frozen=True)
class SideResult:
    task_id: str
    role: str
    alias: str
    context_authority: str
    evidence_authority: str
    compiler_authority: str
    approved_file_count: int
    minimum_approved_files: int
    callable_descriptor_count: int
    type_descriptor_count: int
    descriptor_evidence: bool
    relation_evidence: bool
    boundary_evidence: bool
    k2_ready: bool
    failure_code: str | None

    @property
    def key(self) -> str:
        return f"{self.task_id}:{self.role}"


@dataclass(frozen=True)
class UnitResult:
    alias: str
    revision_authority: str
    session_authority: str
    context_authority: str
    evidence_authority: str
    compiler_authority: str
    analysis_authority: str
    descriptor_evidence: bool
    relation_evidence: bool
    boundary_evidence: bool
    syntax_fallback: bool
    k2_ready: bool
    failure_code: str | None
    task_sides: tuple[SideResult, ...]


def canonical_bytes(value: Any) -> bytes:
    return checked_verifier.canonical_bytes(value)


def authority_digest(value: Any) -> str:
    return checked_verifier.authority_digest(value)


def load_json_bytes(raw: bytes, label: str) -> Any:
    try:
        return json.loads(raw, object_pairs_hook=checked_verifier.reject_duplicate_keys)
    except (json.JSONDecodeError, UnicodeDecodeError, checked_verifier.EvidenceError) as error:
        raise GateError(f"INVALID_{label}") from error


def private_input(path: Path, maximum: int, label: str) -> tuple[Path, bytes]:
    try:
        absolute = path if path.is_absolute() else Path.cwd() / path
        metadata = os.lstat(absolute)
        resolved = absolute.resolve(strict=True)
    except OSError as error:
        raise GateError(f"INVALID_{label}") from error
    if (
        stat.S_ISLNK(metadata.st_mode)
        or not stat.S_ISREG(metadata.st_mode)
        or stat.S_IMODE(metadata.st_mode) != 0o600
        or metadata.st_uid != os.geteuid()
        or resolved != absolute.absolute()
        or metadata.st_size <= 0
        or metadata.st_size > maximum
    ):
        raise GateError(f"INVALID_{label}")
    try:
        raw = resolved.read_bytes()
    except OSError as error:
        raise GateError(f"INVALID_{label}") from error
    if len(raw) != metadata.st_size:
        raise GateError(f"INVALID_{label}")
    return resolved, raw


def parse_corpus(value: Any, *, validate_paths: bool = True) -> Corpus:
    checked_verifier.require_keys(
        value,
        {"schema", "frozenAt", "selectionRule", "services", "tasks", "topologyAuthorities"},
        "private corpus",
    )
    if value["schema"] != PRIVATE_CORPUS_SCHEMA or value["frozenAt"] != FROZEN_AT:
        raise GateError("INVALID_PRIVATE_CORPUS")
    if not isinstance(value["selectionRule"], str) or not value["selectionRule"]:
        raise GateError("INVALID_PRIVATE_CORPUS")
    topology = value["topologyAuthorities"]
    if not isinstance(topology, list) or len(topology) != 3:
        raise GateError("INVALID_PRIVATE_CORPUS")
    for authority in topology:
        checked_verifier.require_keys(authority, {"path", "revision"}, "topology authority")
        if (
            not isinstance(authority["path"], str)
            or not authority["path"].startswith("/")
            or not isinstance(authority["revision"], str)
            or GIT_OID.fullmatch(authority["revision"]) is None
        ):
            raise GateError("INVALID_PRIVATE_CORPUS")

    rows = value["services"]
    if not isinstance(rows, list) or len(rows) != 11:
        raise GateError("INVALID_PRIVATE_CORPUS")
    services: list[Service] = []
    for index, row in enumerate(rows, 1):
        checked_verifier.require_keys(
            row,
            {"serviceAlias", "serviceId", "repositoryPath", "revision"},
            "private service",
        )
        alias = row["serviceAlias"]
        expected = f"service-{index:02}"
        service_id = row["serviceId"]
        revision = row["revision"]
        repository_raw = row["repositoryPath"]
        if (
            alias != expected
            or not isinstance(service_id, str)
            or not service_id
            or len(service_id.encode("utf-8")) > 1024
            or not isinstance(revision, str)
            or GIT_OID.fullmatch(revision) is None
            or not isinstance(repository_raw, str)
            or not repository_raw.startswith("/")
        ):
            raise GateError("INVALID_PRIVATE_CORPUS")
        repository = Path(repository_raw)
        if validate_paths:
            try:
                resolved = repository.resolve(strict=True)
                metadata = os.lstat(repository)
            except OSError as error:
                raise GateError("INVALID_PRIVATE_CORPUS") from error
            if resolved != repository or stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
                raise GateError("INVALID_PRIVATE_CORPUS")
        services.append(Service(alias, service_id, repository, revision))

    if len({service.service_id for service in services}) != 11 or len(
        {service.repository for service in services}
    ) != 11:
        raise GateError("INVALID_PRIVATE_CORPUS")

    task_rows = value["tasks"]
    if not isinstance(task_rows, list) or len(task_rows) != 10:
        raise GateError("INVALID_PRIVATE_CORPUS")
    service_aliases = {service.alias for service in services}
    pair_bindings: dict[str, tuple[str, str]] = {}
    tasks: list[Task] = []
    for index, row in enumerate(task_rows):
        checked_verifier.require_keys(
            row,
            {"taskId", "pairId", "provider", "consumer", "scenario"},
            "private task",
        )
        task_id = row["taskId"]
        pair_id = row["pairId"]
        provider = row["provider"]
        consumer = row["consumer"]
        scenario = row["scenario"]
        if (
            task_id != checked_verifier.EXPECTED_TASKS[index]
            or pair_id != checked_verifier.EXPECTED_PAIRS[index]
            or provider not in service_aliases
            or consumer not in service_aliases
            or provider == consumer
            or (provider, consumer) != checked_verifier.EXPECTED_BINDINGS[index]
            or scenario
            not in {
                "PROVIDER_CONTRACT_CHANGE",
                "CONSUMER_REQUEST_SHAPE",
                "PROVIDER_RESPONSE_SHAPE",
            }
        ):
            raise GateError("INVALID_PRIVATE_CORPUS")
        binding = (provider, consumer)
        if pair_id in pair_bindings and pair_bindings[pair_id] != binding:
            raise GateError("INVALID_PRIVATE_CORPUS")
        pair_bindings[pair_id] = binding
        tasks.append(Task(task_id, pair_id, provider, consumer, scenario))
    if len(pair_bindings) != 8:
        raise GateError("INVALID_PRIVATE_CORPUS")
    return Corpus(value["frozenAt"], tuple(services), tuple(tasks))


def safe_relative_kotlin_file(value: Any) -> str:
    if (
        not isinstance(value, str)
        or not value.endswith((".kt", ".kts"))
        or len(value.encode("utf-8")) > 4096
        or value.startswith("/")
        or "\\" in value
        or "\0" in value
    ):
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    parts = value.split("/")
    if not parts or any(part in {"", ".", ".."} for part in parts):
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    return value


def parse_benchmark(value: Any, raw: bytes, corpus: Corpus, corpus_digest: str) -> Benchmark:
    checked_verifier.require_keys(
        value,
        {
            "authorityDigest",
            "binaryRubric",
            "frozenAt",
            "manualVerificationProfiles",
            "promptProfiles",
            "resourceBudgets",
            "schema",
            "scope",
            "scoring",
            "selfCheck",
            "sourceAuthority",
            "tasks",
        },
        "private benchmark",
    )
    if raw != canonical_bytes(value) + b"\n":
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    unsigned = dict(value)
    declared_authority = unsigned.pop("authorityDigest", None)
    if (
        value["schema"] != PRIVATE_BENCHMARK_SCHEMA
        or value["frozenAt"] != FROZEN_AT
        or declared_authority != authority_digest(unsigned)
        or declared_authority != EXPECTED_BENCHMARK_DIGEST
        or corpus_digest != EXPECTED_CORPUS_DIGEST
    ):
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    if value["scope"] != {
        "absolutePathsStored": False,
        "exactRevisionRequired": True,
        "httpEndpointEquivalenceScored": False,
        "language": "KOTLIN",
        "profile": "CALLABLE_TYPE_DATA_CLASS_NAVIGATION",
        "sourceBodiesStored": False,
    }:
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    if value["scoring"] != {
        "aggregateResult": "PASS_IFF_ALL_TEN_TASKS_PASS",
        "httpEndpointEquivalence": "NOT_SCORED",
        "partialCredit": False,
        "taskResult": "PASS_IFF_ALL_EIGHT_CRITERIA_TRUE_ELSE_FAIL",
    } or value["resourceBudgets"] != EXPECTED_RESOURCE_BUDGETS:
        raise GateError("INVALID_PRIVATE_BENCHMARK")

    source = checked_verifier.require_keys(
        value["sourceAuthority"],
        {"canonicalDigest", "pairIds", "schema", "taskIds"},
        "benchmark source authority",
    )
    if source != {
        "canonicalDigest": corpus_digest,
        "pairIds": [f"pair-{index:02}" for index in range(1, 9)],
        "schema": PRIVATE_CORPUS_SCHEMA,
        "taskIds": checked_verifier.EXPECTED_TASKS,
    }:
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    self_check = checked_verifier.require_keys(
        value["selfCheck"],
        {
            "canonicalization",
            "determinismChecks",
            "expectedPairCount",
            "expectedTaskCount",
            "relevanceChecks",
        },
        "benchmark self-check",
    )
    if (
        self_check["expectedPairCount"] != 8
        or self_check["expectedTaskCount"] != 10
        or not isinstance(self_check["canonicalization"], str)
        or not isinstance(self_check["determinismChecks"], list)
        or not self_check["determinismChecks"]
        or not isinstance(self_check["relevanceChecks"], list)
        or not self_check["relevanceChecks"]
    ):
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    profiles = {"CONTRACT", "REQUEST", "RESPONSE"}
    prompts = value["promptProfiles"]
    manual = value["manualVerificationProfiles"]
    if (
        not isinstance(prompts, dict)
        or set(prompts) != profiles
        or any(not isinstance(text, str) or not text for text in prompts.values())
        or not isinstance(manual, dict)
        or set(manual) != profiles
        or any(
            not isinstance(items, list)
            or not items
            or any(not isinstance(item, str) or not item for item in items)
            for items in manual.values()
        )
    ):
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    rubric = value["binaryRubric"]
    expected_rubric = [
        "EXACT_AUTHORITY",
        "BOTH_SIDES_FILE_NAVIGATION",
        "CALLABLE_DESCRIPTOR_NAVIGATION",
        "TYPE_DESCRIPTOR_NAVIGATION",
        "BOUNDED_SOURCE_EVIDENCE",
        "MANUAL_VERIFICATION_COMPLETE",
        "NO_ENDPOINT_EQUIVALENCE_CLAIM",
        "RESOURCE_BUDGET",
    ]
    if not isinstance(rubric, list) or len(rubric) != len(expected_rubric):
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    for expected_id, row in zip(expected_rubric, rubric, strict=True):
        checked_verifier.require_keys(row, {"id", "passWhen"}, "benchmark rubric")
        if row["id"] != expected_id or not isinstance(row["passWhen"], str) or not row["passWhen"]:
            raise GateError("INVALID_PRIVATE_BENCHMARK")

    tasks = value["tasks"]
    if not isinstance(tasks, list) or len(tasks) != 10:
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    corpus_tasks = {task.task_id: task for task in corpus.tasks}
    revisions = {service.alias: service.revision for service in corpus.services}
    sides: list[OracleSide] = []
    expected_profiles = ["CONTRACT"] * 8 + ["REQUEST", "RESPONSE"]
    for index, row in enumerate(tasks):
        checked_verifier.require_keys(
            row,
            {
                "manualCategoryProfile",
                "oracle",
                "pairId",
                "promptProfile",
                "scenario",
                "taskId",
            },
            "benchmark task",
        )
        task_id = checked_verifier.EXPECTED_TASKS[index]
        corpus_task = corpus_tasks[task_id]
        if (
            row["taskId"] != task_id
            or row["pairId"] != corpus_task.pair_id
            or row["scenario"] != corpus_task.scenario
            or row["promptProfile"] != expected_profiles[index]
            or row["manualCategoryProfile"] != expected_profiles[index]
        ):
            raise GateError("INVALID_PRIVATE_BENCHMARK")
        oracle = checked_verifier.require_keys(
            row["oracle"],
            {"minimumCallableDescriptors", "minimumTypeDescriptors", "sides"},
            "benchmark oracle",
        )
        if (
            oracle["minimumCallableDescriptors"] != 1
            or oracle["minimumTypeDescriptors"] != 1
            or not isinstance(oracle["sides"], list)
            or len(oracle["sides"]) != 2
        ):
            raise GateError("INVALID_PRIVATE_BENCHMARK")
        for role_index, side_value in enumerate(oracle["sides"]):
            checked_verifier.require_keys(
                side_value,
                {
                    "approvedNavigation",
                    "minimumApprovedFiles",
                    "revision",
                    "role",
                    "serviceAlias",
                },
                "benchmark oracle side",
            )
            role = ("provider", "consumer")[role_index]
            alias = (corpus_task.provider, corpus_task.consumer)[role_index]
            navigation_values = side_value["approvedNavigation"]
            if (
                side_value["role"] != role
                or side_value["serviceAlias"] != alias
                or side_value["revision"] != revisions[alias]
                or type(side_value["minimumApprovedFiles"]) is not int
                or side_value["minimumApprovedFiles"] < 1
                or not isinstance(navigation_values, list)
                or not navigation_values
                or side_value["minimumApprovedFiles"] > len(navigation_values)
            ):
                raise GateError("INVALID_PRIVATE_BENCHMARK")
            navigations: list[OracleNavigation] = []
            seen_files: set[str] = set()
            for navigation in navigation_values:
                checked_verifier.require_keys(
                    navigation,
                    {"allowedDeclarations", "blobOid", "relativeFile"},
                    "benchmark navigation",
                )
                relative_file = safe_relative_kotlin_file(navigation["relativeFile"])
                blob_oid = navigation["blobOid"]
                declarations_value = navigation["allowedDeclarations"]
                if (
                    relative_file in seen_files
                    or not isinstance(blob_oid, str)
                    or GIT_BLOB_OID.fullmatch(blob_oid) is None
                    or not isinstance(declarations_value, list)
                    or not declarations_value
                    or len(declarations_value) > 64
                ):
                    raise GateError("INVALID_PRIVATE_BENCHMARK")
                seen_files.add(relative_file)
                declarations: list[OracleDeclaration] = []
                seen_declarations: set[tuple[str, str]] = set()
                for declaration in declarations_value:
                    checked_verifier.require_keys(
                        declaration, {"kind", "name"}, "benchmark declaration"
                    )
                    binding = (declaration["kind"], declaration["name"])
                    if (
                        binding[0] not in {"CLASS", "DATA_CLASS", "ENUM_CLASS", "FUN", "INTERFACE"}
                        or not isinstance(binding[1], str)
                        or SAFE_DECLARATION.fullmatch(binding[1]) is None
                        or binding in seen_declarations
                    ):
                        raise GateError("INVALID_PRIVATE_BENCHMARK")
                    seen_declarations.add(binding)
                    declarations.append(OracleDeclaration(*binding))
                navigations.append(
                    OracleNavigation(relative_file, blob_oid, tuple(declarations))
                )
            sides.append(
                OracleSide(
                    task_id,
                    role,
                    alias,
                    revisions[alias],
                    side_value["minimumApprovedFiles"],
                    oracle["minimumCallableDescriptors"],
                    oracle["minimumTypeDescriptors"],
                    tuple(navigations),
                )
            )

    max_query_terms = EXPECTED_RESOURCE_BUDGETS["maxQueryTermsPerTask"]
    for side in sides:
        terms = {
            declaration.name
            for navigation in side.navigations
            for declaration in navigation.declarations
        }
        if not terms or len(terms) > max_query_terms:
            raise GateError("INVALID_PRIVATE_BENCHMARK")
    if {side.service_alias for side in sides} != set(checked_verifier.EXPECTED_SERVICES):
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    return Benchmark(
        declared_authority,
        tuple(sides),
        max_query_terms,
        EXPECTED_RESOURCE_BUDGETS["maxSelectedFilesPerTask"],
        EXPECTED_RESOURCE_BUDGETS["maxColdWallMsPerTask"] // 1000,
    )


def side_query_terms(side: OracleSide) -> tuple[str, ...]:
    return tuple(
        sorted(
            {
                declaration.name
                for navigation in side.navigations
                for declaration in navigation.declarations
            }
        )
    )


def side_oracles(benchmark: Benchmark) -> dict[str, tuple[OracleSide, ...]]:
    output: dict[str, tuple[OracleSide, ...]] = {}
    for alias in checked_verifier.EXPECTED_SERVICES:
        sides = tuple(
            sorted(
                (side for side in benchmark.sides if side.service_alias == alias),
                key=lambda side: (side.task_id, side.role),
            )
        )
        if not sides or any(
            not side_query_terms(side)
            or len(side_query_terms(side)) > benchmark.max_query_terms
            for side in sides
        ):
            raise GateError("INVALID_PRIVATE_BENCHMARK")
        output[alias] = sides
    return output


def executable_authority(path: Path) -> tuple[Path, str]:
    try:
        absolute = path if path.is_absolute() else Path.cwd() / path
        resolved = absolute.resolve(strict=True)
        metadata = os.stat(resolved)
    except OSError as error:
        raise GateError("INVALID_CLEW_EXECUTABLE") from error
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_size <= 0
        or metadata.st_size > MAX_CLEW_BYTES
        or not os.access(resolved, os.X_OK)
    ):
        raise GateError("INVALID_CLEW_EXECUTABLE")
    digest = hashlib.sha256()
    try:
        with resolved.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                digest.update(chunk)
    except OSError as error:
        raise GateError("INVALID_CLEW_EXECUTABLE") from error
    return resolved, f"sha256:{digest.hexdigest()}"


def git_output(repository: Path, arguments: list[str]) -> str:
    try:
        completed = subprocess.run(
            ["git", "-C", os.fspath(repository), *arguments],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            timeout=30,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise GateError("GIT_AUTHORITY_UNAVAILABLE") from error
    if completed.returncode != 0 or len(completed.stdout) > MAX_GIT_STDOUT_BYTES:
        raise GateError("GIT_AUTHORITY_UNAVAILABLE")
    try:
        return completed.stdout.decode("utf-8").strip()
    except UnicodeDecodeError as error:
        raise GateError("GIT_AUTHORITY_UNAVAILABLE") from error


def git_output_optional(repository: Path, arguments: list[str]) -> str | None:
    try:
        completed = subprocess.run(
            ["git", "-C", os.fspath(repository), *arguments],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            timeout=30,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise GateError("GIT_AUTHORITY_UNAVAILABLE") from error
    if len(completed.stdout) > MAX_GIT_STDOUT_BYTES:
        raise GateError("GIT_AUTHORITY_UNAVAILABLE")
    if completed.returncode != 0:
        return None
    try:
        return completed.stdout.decode("utf-8").strip()
    except UnicodeDecodeError as error:
        raise GateError("GIT_AUTHORITY_UNAVAILABLE") from error


def pinned_target_ref(service: Service) -> str:
    commit = git_output(service.repository, ["rev-parse", "--verify", f"{service.revision}^{{commit}}"])
    head = git_output(service.repository, ["rev-parse", "--verify", "HEAD^{commit}"])
    if commit != service.revision or head != service.revision:
        raise GateError("HEAD_NOT_PINNED")
    symbolic = git_output_optional(service.repository, ["symbolic-ref", "-q", "HEAD"])
    candidates = []
    if symbolic is not None and symbolic.startswith("refs/heads/"):
        candidates.append(symbolic)
    listed = git_output(
        service.repository,
        ["for-each-ref", "--format=%(refname)", "--points-at", service.revision, "refs/heads"],
    )
    candidates.extend(line for line in listed.splitlines() if line.startswith("refs/heads/"))
    for candidate in sorted(set(candidates)):
        if git_output(service.repository, ["rev-parse", "--verify", candidate]) == service.revision:
            return candidate
    raise GateError("PINNED_BRANCH_UNAVAILABLE")


def validate_oracle_files(corpus: Corpus, benchmark: Benchmark) -> None:
    services = {service.alias: service for service in corpus.services}
    checked: set[tuple[str, str, str]] = set()
    for side in benchmark.sides:
        service = services.get(side.service_alias)
        if service is None or service.revision != side.revision:
            raise GateError("INVALID_PRIVATE_BENCHMARK")
        for navigation in side.navigations:
            key = (side.service_alias, navigation.relative_file, navigation.blob_oid)
            if key in checked:
                continue
            checked.add(key)
            observed = git_output(
                service.repository,
                ["rev-parse", "--verify", f"{service.revision}:{navigation.relative_file}"],
            )
            if observed != navigation.blob_oid or git_output(
                service.repository, ["cat-file", "-t", observed]
            ) != "blob":
                raise GateError("ORACLE_BLOB_AUTHORITY_INVALID")


def run_json_process(
    command: list[str],
    timeout_seconds: int,
    max_stdout_bytes: int = MAX_CLEW_STDOUT_BYTES,
) -> dict[str, Any]:
    if not 1 <= max_stdout_bytes <= MAX_CLEW_STDOUT_BYTES:
        raise GateError("RESOURCE_BUDGET_INVALID")
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        try:
            process = subprocess.Popen(
                command,
                stdin=subprocess.DEVNULL,
                stdout=stdout,
                stderr=stderr,
                start_new_session=True,
                close_fds=True,
            )
        except OSError as error:
            raise GateError("CLEW_START_FAILED") from error
        try:
            return_code = process.wait(timeout=timeout_seconds)
        except subprocess.TimeoutExpired as error:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except OSError:
                process.kill()
            process.wait()
            raise GateError("CLEW_TIMEOUT") from error
        stdout.seek(0, os.SEEK_END)
        size = stdout.tell()
        if return_code != 0:
            raise GateError("CLEW_COMMAND_FAILED")
        if size <= 0 or size > max_stdout_bytes:
            raise GateError("CLEW_OUTPUT_INVALID")
        stdout.seek(0)
        raw = stdout.read(max_stdout_bytes + 1)
    try:
        value = json.loads(raw)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise GateError("CLEW_OUTPUT_INVALID") from error
    if not isinstance(value, dict):
        raise GateError("CLEW_OUTPUT_INVALID")
    return value


def require_digest(value: Any, code: str) -> str:
    if not isinstance(value, str) or SHA256.fullmatch(value) is None:
        raise GateError(code)
    return value


def require_cas(value: Any, object_schema: str, code: str) -> str:
    if (
        not isinstance(value, dict)
        or set(value) != {"schema", "objectSchema", "digest", "size"}
        or value.get("schema") != CAS_OBJECT_SCHEMA
        or value.get("objectSchema") != object_schema
        or type(value.get("size")) is not int
        or value["size"] <= 0
    ):
        raise GateError(code)
    return require_digest(value.get("digest"), code)


def parse_session_open(
    value: dict[str, Any], service: Service, target_ref: str
) -> dict[str, Any]:
    session = value.get("session")
    if (
        set(value) != {"schema", "status", "session"}
        or value.get("schema") != "codeclew-session-open/4.0"
        or value.get("status") != "OPEN"
        or not isinstance(session, dict)
        or session.get("schema") != "codeclew-session/5.0"
        or session.get("language") != "KOTLIN"
        or session.get("compilations") != [COMPILATION]
        or session.get("baseRevision") != service.revision
        or session.get("targetRef") != target_ref
        or session.get("targetOid") != service.revision
        or session.get("runtimeMode") not in {"DEVELOPMENT", "RELEASE"}
        or session.get("generationJobs") != 1
        or not isinstance(session.get("sessionId"), str)
        or SESSION_ID.fullmatch(session["sessionId"]) is None
        or not isinstance(session.get("repositoryKey"), str)
        or REPOSITORY_KEY.fullmatch(session["repositoryKey"]) is None
        or not isinstance(session.get("runtimeKey"), str)
        or SHA256.fullmatch(session["runtimeKey"]) is None
    ):
        raise GateError("SESSION_AUTHORITY_INVALID")
    require_digest(session.get("authorityDigest"), "SESSION_AUTHORITY_INVALID")
    return session


def safe_match_file(value: Any) -> str:
    try:
        return safe_relative_kotlin_file(value)
    except GateError as error:
        raise GateError("K2_MATCH_INVALID") from error


def valid_source_range(payload: dict[str, Any]) -> bool:
    start = payload.get("start")
    end = payload.get("end")
    return type(start) is int and type(end) is int and 0 <= start <= end


def symbol_tokens(*values: Any) -> set[str]:
    output: set[str] = set()
    for value in values:
        if isinstance(value, str):
            output.update(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", value))
    return output


def validate_k2_match(match: Any) -> tuple[str, str, set[str], str | None] | None:
    if not isinstance(match, dict):
        raise GateError("K2_MATCH_INVALID")
    fact_key = match.get("factKey")
    fact_match = KOTLIN_FACT_KEY.fullmatch(fact_key) if isinstance(fact_key, str) else None
    if (
        fact_match is None
        or match.get("compilation") != COMPILATION
        or match.get("domainUri") != KOTLIN_FACT_DOMAIN
        or not isinstance(match.get("payload"), dict)
    ):
        raise GateError("K2_MATCH_INVALID")
    require_cas(match.get("payloadRef"), FACT_PAYLOAD_SCHEMA, "K2_MATCH_INVALID")
    category = fact_match.group(1)
    if category in {"metadata", "file"}:
        return None
    payload = match["payload"]
    relative_file = safe_match_file(payload.get("file"))
    if not valid_source_range(payload):
        raise GateError("K2_MATCH_INVALID")
    if category.endswith("-boundary") and payload.get("code") == "SYNTAX_ONLY":
        raise GateError("SYNTAX_FALLBACK_REJECTED")
    declaration_kind: str | None = None
    if category == "descriptor":
        declaration_kind = payload.get("declarationKind")
        if (
            payload.get("schema") != "declaration-descriptor/0.1"
            or payload.get("resolution") != "PROVEN"
            or payload.get("provider") != "K2_FIR"
            or payload.get("sourceProvenance") != "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
            or payload.get("compilerAuthority") != "fir-facts-extractor/0.6"
            or declaration_kind
            not in {"FUNCTION", "CONSTRUCTOR", "PROPERTY", "MUTABLE_PROPERTY", "CLASS"}
        ):
            raise GateError("K2_MATCH_INVALID")
        names = symbol_tokens(
            payload.get("compilerCallableId"),
            payload.get("compilerClassId"),
            payload.get("symbolIdentity"),
        )
    elif category == "relation":
        if (
            payload.get("schema") != "declaration-relation/0.1"
            or payload.get("resolution") != "PROVEN"
            or payload.get("provider") != "K2_FIR"
            or payload.get("sourceProvenance") != "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
            or not isinstance(payload.get("owner"), str)
            or not isinstance(payload.get("target"), str)
        ):
            raise GateError("K2_MATCH_INVALID")
        names = symbol_tokens(payload.get("owner"), payload.get("target"))
    elif category == "descriptor-boundary":
        if (
            payload.get("schema") != "declaration-descriptor-boundary/0.1"
            or payload.get("resolution") != "UNKNOWN"
            or payload.get("provider")
            not in {"K2_FIR", "COMPILER_DESCRIPTOR_NORMALIZER", "WORKER"}
            or payload.get("compilerAuthority") != "fir-facts-extractor/0.6"
        ):
            raise GateError("K2_MATCH_INVALID")
        names = symbol_tokens(
            payload.get("symbolIdentity"),
            payload.get("compilerCallableId"),
            payload.get("compilerClassId"),
        )
    else:
        if (
            payload.get("schema") != "declaration-relation-boundary/0.1"
            or payload.get("resolution") != "UNKNOWN"
            or payload.get("provider")
            not in {
                "K2_FIR",
                "K2_FIR_CFG",
                "COMPILER_RELATION_NORMALIZER",
                "CODECLEW_RELATION_NORMALIZER",
                "WORKER",
            }
        ):
            raise GateError("K2_MATCH_INVALID")
        names = symbol_tokens(
            payload.get("owner"),
            payload.get("target"),
            payload.get("symbol"),
        )
    if not names:
        raise GateError("K2_MATCH_INVALID")
    return category, relative_file, names, declaration_kind


def declaration_matches(
    category: str,
    names: set[str],
    declaration_kind: str | None,
    declaration: OracleDeclaration,
) -> bool:
    if declaration.name not in names:
        return False
    if category != "descriptor":
        return True
    if declaration.kind == "FUN":
        return declaration_kind == "FUNCTION"
    return declaration_kind == "CLASS"


def parse_context(
    value: dict[str, Any],
    session: dict[str, Any],
    side: OracleSide,
) -> SideResult:
    context = value.get("context")
    completeness = value.get("completeness")
    expected_terms = sorted(term.lower() for term in side_query_terms(side))
    if (
        value.get("schema") != "codeclew-context-result/2.0"
        or value.get("sessionId") != session["sessionId"]
        or not isinstance(value.get("contextId"), str)
        or CONTEXT_ID.fullmatch(value["contextId"]) is None
        or not isinstance(context, dict)
        or context.get("schema") != "codeclew-bounded-context-projection/4.0"
        or context.get("language") != "language:kotlin"
        or context.get("compilations") != [COMPILATION]
        or context.get("task", {}).get("terms") != expected_terms
        or context.get("completeness") != completeness
    ):
        raise GateError("CONTEXT_AUTHORITY_INVALID")
    generation_authority = context.get("generationAuthority")
    compiler_versions = context.get("compilerVersions")
    snapshot = context.get("snapshot")
    if (
        not isinstance(generation_authority, dict)
        or (
            generation_authority.get("coverage"), generation_authority.get("certainty")
        )
        not in {("COMPLETE", "VERIFIED"), ("PARTIAL", "UNSURE")}
        or not isinstance(compiler_versions, dict)
        or set(compiler_versions) != {COMPILATION}
        or not isinstance(compiler_versions[COMPILATION], str)
        or compiler_versions[COMPILATION] not in SUPPORTED_COMPILERS
        or not isinstance(snapshot, dict)
        or snapshot.get("baseRevision") != session["baseRevision"]
        or not isinstance(snapshot.get("compilations"), list)
        or len(snapshot["compilations"]) != 1
    ):
        raise GateError("COMPILER_WORKER_NOT_VERIFIED")
    compilation = snapshot["compilations"][0]
    if (
        not isinstance(compilation, dict)
        or compilation.get("compilation") != COMPILATION
        or compilation.get("compilerVersion") != compiler_versions[COMPILATION]
    ):
        raise GateError("COMPILER_WORKER_NOT_VERIFIED")
    generation_digest = require_cas(
        compilation.get("generation"),
        "codeclew-generation-manifest/2.0",
        "COMPILER_WORKER_NOT_VERIFIED",
    )
    query_index_digest = require_cas(
        compilation.get("queryIndex"),
        "codeclew-query-index/3.0",
        "COMPILER_WORKER_NOT_VERIFIED",
    )
    unmatched = completeness.get("unmatchedTerms") if isinstance(completeness, dict) else None
    if (
        not isinstance(unmatched, list)
        or any(term not in expected_terms for term in unmatched)
        or len(set(unmatched)) != len(unmatched)
    ):
        raise GateError("CONTEXT_AUTHORITY_INVALID")
    obligations = context.get("verificationObligations")
    if not isinstance(obligations, list):
        raise GateError("CONTEXT_AUTHORITY_INVALID")
    obligation_ids: set[str] = set()
    for obligation in obligations:
        if not isinstance(obligation, dict):
            raise GateError("CONTEXT_AUTHORITY_INVALID")
        code = obligation.get("code")
        identifier = obligation.get("id")
        binding = (identifier, code)
        if binding not in {
            ("VERIFY_PARTIAL_KOTLIN_BOUNDARIES", "UNSURE_GENERATION_AUTHORITY"),
            ("verify-query-selection", "VERIFY_QUERY_SELECTION"),
        } or identifier in obligation_ids:
            raise GateError("SYNTAX_FALLBACK_REJECTED")
        obligation_ids.add(identifier)
    partial_projection = generation_authority == {"coverage": "PARTIAL", "certainty": "UNSURE"}
    if partial_projection != ("VERIFY_PARTIAL_KOTLIN_BOUNDARIES" in obligation_ids):
        raise GateError("COMPILER_WORKER_NOT_VERIFIED")

    matches = context.get("matches")
    if (
        not isinstance(matches, list)
        or len(matches) > EXPECTED_RESOURCE_BUDGETS["maxReturnedFactsPerTask"]
    ):
        raise GateError("CONTEXT_AUTHORITY_INVALID")
    validated_matches: list[tuple[str, str, set[str], str | None, str]] = []
    for match in matches:
        validated = validate_k2_match(match)
        if validated is not None:
            category, relative_file, names, declaration_kind = validated
            if category.endswith("-boundary"):
                # Open boundaries remain verification obligations. They never
                # qualify a frozen side or become readiness evidence.
                continue
            payload_digest = require_cas(
                match.get("payloadRef"), FACT_PAYLOAD_SCHEMA, "K2_MATCH_INVALID"
            )
            validated_matches.append(
                (category, relative_file, names, declaration_kind, payload_digest)
            )
    approved_files: set[str] = set()
    bound_matches: set[tuple[str, str]] = set()
    callable_descriptors: set[str] = set()
    type_descriptors: set[str] = set()
    for navigation in side.navigations:
        for category, relative_file, names, declaration_kind, payload_digest in validated_matches:
            if relative_file != navigation.relative_file:
                continue
            if not any(
                declaration_matches(category, names, declaration_kind, declaration)
                for declaration in navigation.declarations
            ):
                continue
            bound_matches.add((category, payload_digest))
            if category != "descriptor":
                continue
            approved_files.add(relative_file)
            if declaration_kind in {
                "FUNCTION",
                "CONSTRUCTOR",
                "PROPERTY",
                "MUTABLE_PROPERTY",
            }:
                callable_descriptors.add(payload_digest)
            elif declaration_kind == "CLASS":
                type_descriptors.add(payload_digest)
    descriptor = bool(callable_descriptors or type_descriptors)
    relation = any(category == "relation" for category, _ in bound_matches)
    evidence_authority = require_digest(value.get("evidenceDigest"), "CONTEXT_AUTHORITY_INVALID")
    context_authority = value["contextId"].removeprefix("context:")
    compiler_authority = authority_digest(
        {
            "schema": COMPILER_AUTHORITY_SCHEMA,
            "taskId": side.task_id,
            "role": side.role,
            "runtimeKey": session["runtimeKey"],
            "runtimeMode": session["runtimeMode"],
            "sessionAuthority": session["authorityDigest"],
            "compilation": COMPILATION,
            "compilerVersion": compiler_versions[COMPILATION],
            "generation": generation_digest,
            "queryIndex": query_index_digest,
            "matchedK2Facts": authority_digest(sorted(set(bound_matches))),
            "analysisAuthority": "COMPILER_WORKER",
        }
    )
    return SideResult(
        task_id=side.task_id,
        role=side.role,
        alias=side.service_alias,
        context_authority=context_authority,
        evidence_authority=evidence_authority,
        compiler_authority=compiler_authority,
        approved_file_count=len(approved_files),
        minimum_approved_files=side.minimum_approved_files,
        callable_descriptor_count=len(callable_descriptors),
        type_descriptor_count=len(type_descriptors),
        descriptor_evidence=descriptor,
        relation_evidence=relation,
        boundary_evidence=False,
        k2_ready=True,
        failure_code=None,
    )


def side_authority_row(side: SideResult) -> dict[str, Any]:
    return {
        "taskId": side.task_id,
        "role": side.role,
        "contextAuthority": side.context_authority,
        "evidenceAuthority": side.evidence_authority,
        "compilerAuthority": side.compiler_authority,
    }


def aggregate_unit_authority(
    kind: str, alias: str, sides: tuple[SideResult, ...]
) -> str:
    return checked_verifier.unit_aggregate_authority(
        kind, alias, [side_authority_row(side) for side in sides]
    )


def failure_result(
    alias: str,
    corpus_digest: str,
    code: str,
    oracle_sides: tuple[OracleSide, ...],
) -> UnitResult:
    def failure_authority(kind: str) -> str:
        return authority_digest(
            {
                "schema": FAILURE_AUTHORITY_SCHEMA,
                "privateCorpusDigest": corpus_digest,
                "serviceAlias": alias,
                "kind": kind,
                "failureCode": code,
            }
        )

    failed_sides = tuple(
        SideResult(
            task_id=side.task_id,
            role=side.role,
            alias=side.service_alias,
            context_authority=authority_digest(
                {
                    "schema": FAILURE_AUTHORITY_SCHEMA,
                    "privateCorpusDigest": corpus_digest,
                    "serviceAlias": alias,
                    "taskId": side.task_id,
                    "role": side.role,
                    "kind": "CONTEXT",
                    "failureCode": code,
                }
            ),
            evidence_authority=authority_digest(
                {
                    "schema": FAILURE_AUTHORITY_SCHEMA,
                    "privateCorpusDigest": corpus_digest,
                    "serviceAlias": alias,
                    "taskId": side.task_id,
                    "role": side.role,
                    "kind": "EVIDENCE",
                    "failureCode": code,
                }
            ),
            compiler_authority=authority_digest(
                {
                    "schema": FAILURE_AUTHORITY_SCHEMA,
                    "privateCorpusDigest": corpus_digest,
                    "serviceAlias": alias,
                    "taskId": side.task_id,
                    "role": side.role,
                    "kind": "COMPILER",
                    "failureCode": code,
                }
            ),
            approved_file_count=0,
            minimum_approved_files=side.minimum_approved_files,
            callable_descriptor_count=0,
            type_descriptor_count=0,
            descriptor_evidence=False,
            relation_evidence=False,
            boundary_evidence=False,
            k2_ready=False,
            failure_code=code,
        )
        for side in oracle_sides
    )
    return UnitResult(
        alias=alias,
        revision_authority=failure_authority("REVISION"),
        session_authority=failure_authority("SESSION"),
        context_authority=aggregate_unit_authority("CONTEXT", alias, failed_sides),
        evidence_authority=aggregate_unit_authority("EVIDENCE", alias, failed_sides),
        compiler_authority=aggregate_unit_authority("COMPILER", alias, failed_sides),
        analysis_authority="UNAVAILABLE",
        descriptor_evidence=False,
        relation_evidence=False,
        boundary_evidence=False,
        syntax_fallback=code == "SYNTAX_FALLBACK_REJECTED",
        k2_ready=False,
        failure_code=code,
        task_sides=failed_sides,
    )


def run_unit(
    service: Service,
    oracle_sides: tuple[OracleSide, ...],
    corpus_digest: str,
    clew: Path,
    max_roots: int,
    timeout_seconds: int,
) -> UnitResult:
    session_id: str | None = None
    terminal = False
    try:
        target_ref = pinned_target_ref(service)
        opened = run_json_process(
            [
                os.fspath(clew),
                "--json",
                "session",
                "open",
                "--repo",
                os.fspath(service.repository),
                "--target-ref",
                target_ref,
                "--language",
                "kotlin",
                "--compilation",
                COMPILATION,
                "--generation-jobs",
                "1",
            ],
            timeout_seconds,
        )
        session = parse_session_open(opened, service, target_ref)
        session_id = session["sessionId"]
        task_sides: list[SideResult] = []
        for side in oracle_sides:
            context_value = run_json_process(
                [
                    os.fspath(clew),
                    "--json",
                    "context",
                    "create",
                    "--session",
                    session_id,
                    "--intent",
                    INTENT,
                    *[
                        argument
                        for term in side_query_terms(side)
                        for argument in ("--term", term)
                    ],
                    "--max-roots",
                    str(max_roots),
                ],
                timeout_seconds,
                EXPECTED_RESOURCE_BUDGETS["maxStdoutBytesPerTask"],
            )
            task_sides.append(parse_context(context_value, session, side))
        run_json_process(
            [os.fspath(clew), "--json", "session", "close", "--session", session_id],
            min(timeout_seconds, 120),
        )
        terminal = True
        revision_authority = authority_digest(
            {
                "schema": REVISION_AUTHORITY_SCHEMA,
                "privateCorpusDigest": corpus_digest,
                "serviceAlias": service.alias,
                "repositoryKey": session["repositoryKey"],
                "baseRevision": session["baseRevision"],
            }
        )
        frozen_sides = tuple(sorted(task_sides, key=lambda side: (side.task_id, side.role)))
        return UnitResult(
            alias=service.alias,
            revision_authority=revision_authority,
            session_authority=session["authorityDigest"],
            context_authority=aggregate_unit_authority(
                "CONTEXT", service.alias, frozen_sides
            ),
            evidence_authority=aggregate_unit_authority(
                "EVIDENCE", service.alias, frozen_sides
            ),
            compiler_authority=aggregate_unit_authority(
                "COMPILER", service.alias, frozen_sides
            ),
            analysis_authority="COMPILER_WORKER",
            descriptor_evidence=any(side.descriptor_evidence for side in frozen_sides),
            relation_evidence=any(side.relation_evidence for side in frozen_sides),
            boundary_evidence=False,
            syntax_fallback=False,
            k2_ready=True,
            failure_code=None,
            task_sides=frozen_sides,
        )
    except GateError as error:
        code = error.code
    except Exception:
        code = "INTERNAL_UNIT_FAILURE"
    finally:
        if session_id is not None and not terminal:
            try:
                run_json_process(
                    [os.fspath(clew), "--json", "session", "abort", "--session", session_id],
                    min(timeout_seconds, 120),
                )
            except GateError:
                if "code" in locals() and code != "SESSION_CLEANUP_FAILED":
                    code = "SESSION_CLEANUP_FAILED"
    return failure_result(service.alias, corpus_digest, code, oracle_sides)


def checked_unit(result: UnitResult) -> dict[str, Any]:
    unit: dict[str, Any] = {
        "serviceAlias": result.alias,
        "revisionAuthority": result.revision_authority,
        "sessionAuthority": result.session_authority,
        "contextAuthority": result.context_authority,
        "evidenceAuthority": result.evidence_authority,
        "compilerAuthority": result.compiler_authority,
        "taskSideCount": len(result.task_sides),
        "analysisAuthority": result.analysis_authority,
        "descriptorEvidence": result.descriptor_evidence,
        "relationEvidence": result.relation_evidence,
        "boundaryEvidence": result.boundary_evidence,
        "syntaxFallback": result.syntax_fallback,
        "k2Ready": result.k2_ready,
        "failureCode": result.failure_code,
    }
    unit["unitAuthority"] = authority_digest(checked_verifier.unit_authority_payload(unit))
    return unit


def checked_side(result: SideResult) -> dict[str, Any]:
    side: dict[str, Any] = {
        "contextAuthority": result.context_authority,
        "evidenceAuthority": result.evidence_authority,
        "compilerAuthority": result.compiler_authority,
        "approvedFileCount": result.approved_file_count,
        "minimumApprovedFiles": result.minimum_approved_files,
        "callableDescriptorCount": result.callable_descriptor_count,
        "typeDescriptorCount": result.type_descriptor_count,
        "descriptorEvidence": result.descriptor_evidence,
        "relationEvidence": result.relation_evidence,
        "boundaryEvidence": False,
        "k2Ready": result.k2_ready,
    }
    side["sideAuthority"] = authority_digest(
        checked_verifier.side_authority_payload(
            result.task_id, result.role, result.alias, side
        )
    )
    return side


def build_checked_evidence(
    corpus: Corpus,
    benchmark: Benchmark,
    corpus_digest: str,
    benchmark_digest: str,
    clew_authority: str,
    max_parallelism: int,
    results: list[UnitResult],
) -> dict[str, Any]:
    units = [checked_unit(result) for result in sorted(results, key=lambda item: item.alias)]
    by_alias = {unit["serviceAlias"]: unit for unit in units}
    result_by_side: dict[str, SideResult] = {}
    for result in results:
        for side in result.task_sides:
            if side.key in result_by_side:
                raise GateError("DUPLICATE_TASK_SIDE_RESULT")
            result_by_side[side.key] = side
    benchmark_by_side = {side.key: side for side in benchmark.sides}
    if set(result_by_side) != set(benchmark_by_side):
        raise GateError("TASK_SIDE_RESULT_SET_INVALID")
    tasks = []
    for source in corpus.tasks:
        provider = by_alias[source.provider]
        consumer = by_alias[source.consumer]
        provider_result = result_by_side[f"{source.task_id}:provider"]
        consumer_result = result_by_side[f"{source.task_id}:consumer"]
        provider_oracle = benchmark_by_side[provider_result.key]
        consumer_oracle = benchmark_by_side[consumer_result.key]
        if (
            provider_result.alias != source.provider
            or consumer_result.alias != source.consumer
            or provider_oracle.service_alias != source.provider
            or consumer_oracle.service_alias != source.consumer
            or provider_result.minimum_approved_files
            != provider_oracle.minimum_approved_files
            or consumer_result.minimum_approved_files
            != consumer_oracle.minimum_approved_files
            or provider_oracle.minimum_callable_descriptors
            != consumer_oracle.minimum_callable_descriptors
            or provider_oracle.minimum_type_descriptors
            != consumer_oracle.minimum_type_descriptors
        ):
            raise GateError("INVALID_PRIVATE_BENCHMARK")
        provider_side = checked_side(provider_result)
        consumer_side = checked_side(consumer_result)
        callable_count = (
            provider_result.callable_descriptor_count
            + consumer_result.callable_descriptor_count
        )
        type_count = (
            provider_result.type_descriptor_count + consumer_result.type_descriptor_count
        )
        minimum_callable = provider_oracle.minimum_callable_descriptors
        minimum_type = provider_oracle.minimum_type_descriptors
        covered = (
            provider["k2Ready"] is True
            and consumer["k2Ready"] is True
            and provider_result.k2_ready
            and consumer_result.k2_ready
            and provider_result.approved_file_count
            >= provider_result.minimum_approved_files
            and consumer_result.approved_file_count
            >= consumer_result.minimum_approved_files
        )
        task: dict[str, Any] = {
            "taskId": source.task_id,
            "pairId": source.pair_id,
            "provider": source.provider,
            "consumer": source.consumer,
            "providerUnitAuthority": provider["unitAuthority"],
            "consumerUnitAuthority": consumer["unitAuthority"],
            "providerSide": provider_side,
            "consumerSide": consumer_side,
            "relationshipAuthority": "DECLARED_TOPOLOGY",
            "twoMemberCoverage": covered,
            "callableDescriptorCount": callable_count,
            "minimumCallableDescriptors": minimum_callable,
            "callableDescriptorNavigation": callable_count >= minimum_callable,
            "typeDescriptorCount": type_count,
            "minimumTypeDescriptors": minimum_type,
            "typeDescriptorNavigation": type_count >= minimum_type,
            "manualVerificationProfileBound": True,
            "resourceBudgetAuthorityBound": True,
            "httpEquivalenceClaims": 0,
        }
        task["taskAuthority"] = authority_digest(checked_verifier.task_authority_payload(task))
        tasks.append(task)
    ready_units = sum(unit["k2Ready"] is True for unit in units)
    covered_tasks = sum(
        task["twoMemberCoverage"] is True
        and task["callableDescriptorNavigation"] is True
        and task["typeDescriptorNavigation"] is True
        for task in tasks
    )
    callable_tasks = sum(task["callableDescriptorNavigation"] is True for task in tasks)
    type_tasks = sum(task["typeDescriptorNavigation"] is True for task in tasks)
    pair_count = len({task["pairId"] for task in tasks})
    passed = (
        ready_units == 11
        and covered_tasks == 10
        and callable_tasks == 10
        and type_tasks == 10
        and pair_count == 8
    )
    return {
        "schema": checked_verifier.SCHEMA,
        "frozenAt": corpus.frozen_at,
        "selectionAuthority": {
            "kind": "PINNED_KOTLIN_DESCRIPTOR_CORPUS",
            "ruleId": "REUSE_G1_TASKS_AND_PAIRS_V1",
            "privateCorpusDigest": corpus_digest,
            "benchmarkDigest": benchmark_digest,
            "unitCount": 11,
            "taskCount": 10,
            "pairCount": 8,
        },
        "executionAuthority": {
            "clewAuthority": clew_authority,
            "compilationAuthority": authority_digest(COMPILATION),
            "maxParallelism": max_parallelism,
        },
        "units": units,
        "tasks": tasks,
        "summary": {
            "unitCount": 11,
            "readyUnits": ready_units,
            "taskCount": 10,
            "taskSideCount": len(result_by_side),
            "coveredTasks": covered_tasks,
            "callableNavigableTasks": callable_tasks,
            "typeNavigableTasks": type_tasks,
            "distinctServicePairs": pair_count,
            "declaredTopologyTasks": 10,
            "manualVerificationProfilesBound": 10,
            "resourceBudgetAuthoritiesBound": 10,
            "httpEquivalenceClaims": 0,
            "result": "PASS" if passed else "STOP_PROFILE_SELECTION",
        },
        "privacy": {
            "absolutePaths": False,
            "repositoryNames": False,
            "sourceBodies": False,
            "packageNames": False,
            "credentials": False,
        },
    }


def build_private_output(
    checked: dict[str, Any],
    results: list[UnitResult],
) -> dict[str, Any]:
    failures = [
        {"serviceAlias": result.alias, "failureCode": result.failure_code}
        for result in sorted(results, key=lambda item: item.alias)
        if result.failure_code is not None
    ]
    return {
        "schema": PRIVATE_OUTPUT_SCHEMA,
        "frozenAt": checked["frozenAt"],
        "privateCorpusDigest": checked["selectionAuthority"]["privateCorpusDigest"],
        "benchmarkDigest": checked["selectionAuthority"]["benchmarkDigest"],
        "checkedEvidenceDigest": authority_digest(checked),
        "failures": failures,
        "summary": checked["summary"],
    }


def output_target(path: Path) -> Path:
    absolute = path if path.is_absolute() else Path.cwd() / path
    try:
        parent = absolute.parent.resolve(strict=True)
    except OSError as error:
        raise GateError("OUTPUT_WRITE_FAILED") from error
    target = parent / absolute.name
    try:
        metadata = os.lstat(target)
    except FileNotFoundError:
        return target
    except OSError as error:
        raise GateError("OUTPUT_WRITE_FAILED") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise GateError("OUTPUT_WRITE_FAILED")
    return target


def atomic_write(path: Path, value: Any, mode: int) -> None:
    target = output_target(path)
    raw = canonical_bytes(value)
    temporary: str | None = None
    try:
        descriptor, temporary = tempfile.mkstemp(prefix=f".{target.name}.", dir=target.parent)
        with os.fdopen(descriptor, "wb", closefd=True) as stream:
            os.fchmod(stream.fileno(), mode)
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, target)
        temporary = None
        directory_fd = os.open(target.parent, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
        if stat.S_IMODE(os.stat(target).st_mode) != mode:
            raise GateError("OUTPUT_WRITE_FAILED")
    except (OSError, GateError) as error:
        if temporary is not None:
            try:
                os.unlink(temporary)
            except OSError:
                pass
        if isinstance(error, GateError):
            raise
        raise GateError("OUTPUT_WRITE_FAILED") from error


def run_gate(args: argparse.Namespace) -> dict[str, Any]:
    corpus_path, corpus_raw = private_input(
        args.private_corpus, MAX_PRIVATE_CORPUS_BYTES, "PRIVATE_CORPUS"
    )
    benchmark_path, benchmark_raw = private_input(
        args.private_benchmark, MAX_PRIVATE_BENCHMARK_BYTES, "PRIVATE_BENCHMARK"
    )
    output_paths = [output_target(args.private_output), output_target(args.checked_output)]
    if len(set(output_paths + [corpus_path, benchmark_path])) != 4:
        raise GateError("OUTPUT_PATH_COLLISION")
    corpus_value = load_json_bytes(corpus_raw, "PRIVATE_CORPUS")
    benchmark_value = load_json_bytes(benchmark_raw, "PRIVATE_BENCHMARK")
    if not isinstance(benchmark_value, dict):
        raise GateError("INVALID_PRIVATE_BENCHMARK")
    corpus = parse_corpus(corpus_value)
    corpus_digest = authority_digest(corpus_value)
    if corpus_digest != EXPECTED_CORPUS_DIGEST:
        raise GateError("INVALID_PRIVATE_CORPUS")
    benchmark = parse_benchmark(benchmark_value, benchmark_raw, corpus, corpus_digest)
    if args.timeout_seconds > benchmark.max_cold_seconds:
        raise GateError("RESOURCE_BUDGET_INVALID")
    oracles = side_oracles(benchmark)
    validate_oracle_files(corpus, benchmark)
    benchmark_digest = benchmark.authority_digest
    clew, clew_authority = executable_authority(args.clew)

    results: list[UnitResult] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.max_parallelism) as executor:
        futures = {
            executor.submit(
                run_unit,
                service,
                oracles[service.alias],
                corpus_digest,
                clew,
                benchmark.max_roots,
                args.timeout_seconds,
            ): service.alias
            for service in corpus.services
        }
        for future in concurrent.futures.as_completed(futures):
            alias = futures[future]
            try:
                result = future.result()
            except Exception:
                result = failure_result(
                    alias,
                    corpus_digest,
                    "INTERNAL_UNIT_FAILURE",
                    oracles[alias],
                )
            results.append(result)

    checked = build_checked_evidence(
        corpus,
        benchmark,
        corpus_digest,
        benchmark_digest,
        clew_authority,
        args.max_parallelism,
        results,
    )
    private = build_private_output(checked, results)
    if checked["summary"]["result"] == "PASS":
        checked_verifier.verify_value(checked)
    atomic_write(args.private_output, private, 0o600)
    atomic_write(args.checked_output, checked, 0o644)
    return checked["summary"]


def synthetic_corpus() -> Corpus:
    services = tuple(
        Service(f"service-{index:02}", f"private-{index}", Path(f"/private/{index}"), "a" * 40)
        for index in range(1, 12)
    )
    tasks = tuple(
        Task(task_id, pair_id, provider, consumer, "PROVIDER_CONTRACT_CHANGE")
        for task_id, pair_id, (provider, consumer) in zip(
            checked_verifier.EXPECTED_TASKS,
            checked_verifier.EXPECTED_PAIRS,
            checked_verifier.EXPECTED_BINDINGS,
            strict=True,
        )
    )
    return Corpus(FROZEN_AT, services, tasks)


def synthetic_context() -> tuple[dict[str, Any], dict[str, Any], OracleSide]:
    digest = lambda label: authority_digest(label)
    session = {
        "sessionId": "session:self-test",
        "baseRevision": "a" * 40,
        "runtimeKey": digest("runtime"),
        "runtimeMode": "DEVELOPMENT",
        "authorityDigest": digest("session"),
    }
    navigation = OracleNavigation(
        "src/Sample.kt",
        "b" * 40,
        (OracleDeclaration("CLASS", "Sample"),),
    )
    side = OracleSide(
        "task-01",
        "provider",
        "service-01",
        "a" * 40,
        1,
        1,
        1,
        (navigation,),
    )
    completeness = {"unmatchedTerms": []}
    context = {
        "schema": "codeclew-context-result/2.0",
        "sessionId": session["sessionId"],
        "contextId": f"context:{digest('context')}",
        "evidenceDigest": digest("evidence"),
        "context": {
            "schema": "codeclew-bounded-context-projection/4.0",
            "language": "language:kotlin",
            "compilations": [COMPILATION],
            "task": {"terms": ["sample"]},
            "generationAuthority": {"certainty": "UNSURE", "coverage": "PARTIAL"},
            "compilerVersions": {COMPILATION: "2.4.10"},
            "snapshot": {
                "baseRevision": session["baseRevision"],
                "compilations": [
                    {
                        "compilation": COMPILATION,
                        "compilerVersion": "2.4.10",
                        "generation": {
                            "schema": CAS_OBJECT_SCHEMA,
                            "objectSchema": "codeclew-generation-manifest/2.0",
                            "digest": digest("generation"),
                            "size": 1,
                        },
                        "queryIndex": {
                            "schema": CAS_OBJECT_SCHEMA,
                            "objectSchema": "codeclew-query-index/3.0",
                            "digest": digest("index"),
                            "size": 1,
                        },
                    }
                ],
            },
            "matches": [
                {
                    "compilation": COMPILATION,
                    "factKey": f"kotlin:descriptor:{'c' * 64}",
                    "domainUri": KOTLIN_FACT_DOMAIN,
                    "payloadRef": {
                        "schema": CAS_OBJECT_SCHEMA,
                        "objectSchema": FACT_PAYLOAD_SCHEMA,
                        "digest": digest("payload"),
                        "size": 1,
                    },
                    "payload": {
                        "schema": "declaration-descriptor/0.1",
                        "file": "src/Sample.kt",
                        "start": 0,
                        "end": 10,
                        "declarationKind": "CLASS",
                        "compilerClassId": "sample/Sample",
                        "symbolIdentity": "class:sample/Sample",
                        "resolution": "PROVEN",
                        "provider": "K2_FIR",
                        "sourceProvenance": "COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
                        "compilerAuthority": "fir-facts-extractor/0.6",
                    },
                }
            ],
            "completeness": completeness,
            "verificationObligations": [
                {
                    "id": "VERIFY_PARTIAL_KOTLIN_BOUNDARIES",
                    "code": "UNSURE_GENERATION_AUTHORITY",
                }
            ],
        },
        "completeness": completeness,
    }
    return context, session, side


def synthetic_benchmark(corpus: Corpus) -> Benchmark:
    sides: list[OracleSide] = []
    for task in corpus.tasks:
        for role, alias in (("provider", task.provider), ("consumer", task.consumer)):
            navigation = OracleNavigation(
                "src/Sample.kt",
                "b" * 40,
                (
                    OracleDeclaration("FUN", "work"),
                    OracleDeclaration("CLASS", "Sample"),
                ),
            )
            sides.append(
                OracleSide(
                    task.task_id,
                    role,
                    alias,
                    "a" * 40,
                    1,
                    1,
                    1,
                    (navigation,),
                )
            )
    return Benchmark(
        EXPECTED_BENCHMARK_DIGEST,
        tuple(sides),
        16,
        12,
        900,
    )


def self_test() -> None:
    service = Service("service-01", "private-1", Path("/private/1"), "a" * 40)
    target_ref = "refs/heads/main"
    opened = {
        "schema": "codeclew-session-open/4.0",
        "status": "OPEN",
        "session": {
            "schema": "codeclew-session/5.0",
            "sessionId": "session:self-test",
            "authorityDigest": authority_digest("session"),
            "repositoryKey": "b" * 64,
            "baseRevision": service.revision,
            "targetRef": target_ref,
            "targetOid": service.revision,
            "runtimeKey": authority_digest("runtime"),
            "runtimeMode": "DEVELOPMENT",
            "language": "KOTLIN",
            "compilations": [COMPILATION],
            "generationJobs": 1,
        },
    }
    parse_session_open(opened, service, target_ref)
    release = json.loads(canonical_bytes(opened))
    release["session"]["runtimeMode"] = "RELEASE"
    parse_session_open(release, service, target_ref)
    invalid_mode = json.loads(canonical_bytes(opened))
    invalid_mode["session"]["runtimeMode"] = "QUALIFIED"
    try:
        parse_session_open(invalid_mode, service, target_ref)
    except GateError:
        pass
    else:
        raise AssertionError("unsupported runtime mode was accepted")

    context, session, side = synthetic_context()
    parsed = parse_context(context, session, side)
    if (
        parsed.task_id != "task-01"
        or parsed.role != "provider"
        or parsed.approved_file_count != 1
        or parsed.callable_descriptor_count != 0
        or parsed.type_descriptor_count != 1
        or not parsed.descriptor_evidence
        or parsed.relation_evidence
        or parsed.boundary_evidence
    ):
        raise AssertionError("context evidence presence was not derived")
    fallback = json.loads(canonical_bytes(context))
    fallback["context"]["verificationObligations"][0]["id"] = (
        "RESTORE_K2_SEMANTIC_ANALYSIS"
    )
    try:
        parse_context(fallback, session, side)
    except GateError:
        pass
    else:
        raise AssertionError("syntax fallback authority was accepted")

    descriptor_unknown = json.loads(canonical_bytes(context))
    descriptor_unknown["context"]["matches"][0]["payload"]["resolution"] = "UNKNOWN"
    try:
        parse_context(descriptor_unknown, session, side)
    except GateError:
        pass
    else:
        raise AssertionError("non-PROVEN descriptor was accepted")

    relation_unknown = json.loads(canonical_bytes(context))
    relation_match = relation_unknown["context"]["matches"][0]
    relation_match["factKey"] = f"kotlin:relation:{'d' * 64}"
    relation_match["payload"] = {
        "schema": "declaration-relation/0.1",
        "file": "src/Sample.kt",
        "start": 0,
        "end": 10,
        "owner": "Sample",
        "target": "Other",
        "resolution": "UNKNOWN",
        "provider": "K2_FIR",
        "sourceProvenance": "COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
    }
    try:
        parse_context(relation_unknown, session, side)
    except GateError:
        pass
    else:
        raise AssertionError("non-PROVEN relation was accepted")

    boundary_only = json.loads(canonical_bytes(context))
    boundary_match = boundary_only["context"]["matches"][0]
    boundary_match["factKey"] = f"kotlin:descriptor-boundary:{'e' * 64}"
    boundary_match["payload"] = {
        "schema": "declaration-descriptor-boundary/0.1",
        "file": "src/Sample.kt",
        "start": 0,
        "end": 10,
        "symbolIdentity": "class:sample/Sample",
        "resolution": "UNKNOWN",
        "provider": "K2_FIR",
        "compilerAuthority": "fir-facts-extractor/0.6",
        "code": "UNSUPPORTED_DESCRIPTOR_DETAIL",
    }
    boundary_parsed = parse_context(boundary_only, session, side)
    if (
        boundary_parsed.approved_file_count != 0
        or boundary_parsed.descriptor_evidence
        or boundary_parsed.relation_evidence
        or boundary_parsed.boundary_evidence
    ):
        raise AssertionError("open boundary qualified a frozen side")
    syntax_boundary = json.loads(canonical_bytes(boundary_only))
    syntax_boundary["context"]["matches"][0]["payload"]["code"] = "SYNTAX_ONLY"
    try:
        parse_context(syntax_boundary, session, side)
    except GateError as error:
        if error.code != "SYNTAX_FALLBACK_REJECTED":
            raise
    else:
        raise AssertionError("syntax-only boundary was accepted")

    syntax_relation_boundary = json.loads(canonical_bytes(context))
    relation_boundary_match = syntax_relation_boundary["context"]["matches"][0]
    relation_boundary_match["factKey"] = f"kotlin:relation-boundary:{'f' * 64}"
    relation_boundary_match["payload"] = {
        "schema": "declaration-relation-boundary/0.1",
        "file": "src/Sample.kt",
        "start": 0,
        "end": 10,
        "owner": "Sample",
        "target": "Other",
        "resolution": "UNKNOWN",
        "provider": "K2_FIR",
        "code": "SYNTAX_ONLY",
    }
    try:
        parse_context(syntax_relation_boundary, session, side)
    except GateError as error:
        if error.code != "SYNTAX_FALLBACK_REJECTED":
            raise
    else:
        raise AssertionError("syntax-only relation boundary was accepted")

    corpus = synthetic_corpus()
    benchmark = synthetic_benchmark(corpus)
    corpus_digest = EXPECTED_CORPUS_DIGEST
    side_results_by_alias: dict[str, list[SideResult]] = {
        service.alias: [] for service in corpus.services
    }
    for oracle_side in benchmark.sides:
        side_results_by_alias[oracle_side.service_alias].append(
            SideResult(
                task_id=oracle_side.task_id,
                role=oracle_side.role,
                alias=oracle_side.service_alias,
                context_authority=authority_digest([oracle_side.key, "context"]),
                evidence_authority=authority_digest([oracle_side.key, "evidence"]),
                compiler_authority=authority_digest([oracle_side.key, "compiler"]),
                approved_file_count=1,
                minimum_approved_files=1,
                callable_descriptor_count=1,
                type_descriptor_count=1,
                descriptor_evidence=True,
                relation_evidence=False,
                boundary_evidence=False,
                k2_ready=True,
                failure_code=None,
            )
        )
    results: list[UnitResult] = []
    for service in corpus.services:
        task_sides = tuple(
            sorted(
                side_results_by_alias[service.alias],
                key=lambda result: (result.task_id, result.role),
            )
        )
        if aggregate_unit_authority(
            "CONTEXT", service.alias, task_sides
        ) != aggregate_unit_authority("CONTEXT", service.alias, tuple(reversed(task_sides))):
            raise AssertionError("unit aggregate depends on side execution order")
        results.append(
            UnitResult(
                alias=service.alias,
                revision_authority=authority_digest([service.alias, "revision"]),
                session_authority=authority_digest([service.alias, "session"]),
                context_authority=aggregate_unit_authority(
                    "CONTEXT", service.alias, task_sides
                ),
                evidence_authority=aggregate_unit_authority(
                    "EVIDENCE", service.alias, task_sides
                ),
                compiler_authority=aggregate_unit_authority(
                    "COMPILER", service.alias, task_sides
                ),
                analysis_authority="COMPILER_WORKER",
                descriptor_evidence=True,
                relation_evidence=False,
                boundary_evidence=False,
                syntax_fallback=False,
                k2_ready=True,
                failure_code=None,
                task_sides=task_sides,
            )
        )
    checked = build_checked_evidence(
        corpus,
        benchmark,
        corpus_digest,
        EXPECTED_BENCHMARK_DIGEST,
        authority_digest("clew"),
        2,
        results,
    )
    checked_verifier.verify_value(checked)
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        private_path = root / "private.json"
        checked_path = root / "checked.json"
        atomic_write(private_path, build_private_output(checked, results), 0o600)
        atomic_write(checked_path, checked, 0o644)
        if stat.S_IMODE(private_path.stat().st_mode) != 0o600:
            raise AssertionError("private evidence mode is not 0600")
        checked_verifier.verify(checked_path)


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser(description=__doc__)
    value.add_argument("--private-corpus", type=Path)
    value.add_argument("--private-benchmark", type=Path)
    value.add_argument("--private-output", type=Path)
    value.add_argument("--checked-output", type=Path)
    value.add_argument("--clew", type=Path)
    value.add_argument("--max-parallelism", type=int, default=2)
    value.add_argument("--timeout-seconds", type=int, default=900)
    value.add_argument("--self-test", action="store_true")
    return value


def main(argv: list[str] | None = None) -> int:
    argument_parser = parser()
    args = argument_parser.parse_args(argv)
    if args.self_test:
        try:
            self_test()
        except Exception:
            print("FAIL: SELF_TEST_FAILED", file=sys.stderr)
            return 1
        print(json.dumps({"schema": PRIVATE_OUTPUT_SCHEMA, "selfTest": "PASS"}, sort_keys=True))
        return 0
    for name in ["private_corpus", "private_benchmark", "private_output", "checked_output", "clew"]:
        if getattr(args, name) is None:
            argument_parser.error(f"--{name.replace('_', '-')} is required")
    if not 1 <= args.max_parallelism <= 4:
        argument_parser.error("--max-parallelism must be between 1 and 4")
    if not 30 <= args.timeout_seconds <= 3600:
        argument_parser.error("--timeout-seconds must be between 30 and 3600")
    try:
        summary = run_gate(args)
    except (GateError, checked_verifier.EvidenceError, OSError):
        print("FAIL: G1K_GATE_EXECUTION_FAILED", file=sys.stderr)
        return 1
    if summary["result"] != "PASS":
        print("FAIL: G1K_STOP_PROFILE_SELECTION", file=sys.stderr)
        return 1
    print(
        json.dumps(
            {
                "schema": checked_verifier.SCHEMA,
                "verification": "PASS",
                "summary": summary,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
