#!/usr/bin/env python3
"""Pinned, read-only K1.12 final-audit recomputation.

This program deliberately does not import the readiness harness.  It reopens
the primary matrices and frozen inputs, recomputes the machine-decidable
contour, and fail-closes every requirement whose named evidence packet is not
present.  ACCEPT means the report is authentic and recomputable, not that K1
must GO; ``expectedDecision`` remains total over GO/PIVOT/STOP.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import tomllib
from collections import Counter
from pathlib import Path
from typing import Any, Mapping

SERIES_ID = "KOTLIN_REAL_REPOSITORY_K1_12_2026_08_13"
EXPECTED_Q = [f"K1-Q{number:02d}" for number in range(1, 7)]
EXPECTED_H = [f"K1-H{number:02d}" for number in range(1, 7)]
ROOT = Path(__file__).resolve().parents[1]
COST_KEYS = {
    "externalWallMicros", "maximumResidentBytes", "sourceHashingMicros", "buildDiscoveryMicros",
    "dependencyPreparationMicros", "dependencyVerificationMicros", "adapterStartupMicros",
    "coldIndexMicros", "warmIndexMicros", "providerProcessingMicros", "serializationMicros",
    "storeWriteMicros", "storeReadMicros", "queryProjectionMicros", "sourceBytesRead",
    "cacheBytesRead", "cacheBytesWritten", "emittedBytes", "storedFactBytes", "factCount",
    "boundaryCount", "cacheRequests", "cacheHits", "modelCalls",
}
BASELINE_GRADLE_VERSION = "9.6.1"
BASELINE_GRADLE_URL = "https\\://services.gradle.org/distributions/gradle-9.6.1-bin.zip"
BASELINE_TRANSIENT_SUFFIXES = [".lock", ".lck", ".part", ".tmp"]
BASELINE_JAVA_HOME = "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home"
BASELINE_PATH = (
    f"{BASELINE_JAVA_HOME}/bin:/opt/homebrew/Cellar/maven/3.9.12/bin:"
    "/opt/homebrew/Cellar/rust/1.92.0/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
)
BASELINE_CARGO_FORBIDDEN_INJECTION_KEYS = {
    "GRADLE_OPTS", "GRADLE_USER_HOME", "JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS",
    "JAVA_OPTS", "_JAVA_OPTIONS", "JAVA_HOME",
}
BASELINE_CARGO_REGISTRY = "index.crates.io-1949cf8c6b5b557f"
BASELINE_CARGO_TARGET = "aarch64-apple-darwin"
BASELINE_CARGO_TOOL_PATHS = {
    "cargo": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/cargo"),
    "rustc": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/rustc"),
    "rustfmt": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/rustfmt"),
    "cargoFmt": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/cargo-fmt"),
    "cargoClippy": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/cargo-clippy"),
    "clippyDriver": Path("/opt/homebrew/Cellar/rust/1.92.0/bin/clippy-driver"),
}
BASELINE_CARGO_RESOLVED_SHA256 = "sha256:726c45cb4e1bd444909c5cf5c162bc9b9e8c631ec153c499bdf305827cd66bdb"
BASELINE_CARGO_NON_RESOLVED_SHA256 = "sha256:e22d1fd865c191168c8473b7b72bc64c9b634c5b11fb79aaec4123835562c2a5"
BASELINE_CARGO_CONFIG_SHA256 = "sha256:5b943a2c6f7eb743f7308aba07bdbb47d9ae44aafecd832d7f15df186afbafb3"
BASELINE_CARGO_SEED_TREE_SHA256 = "sha256:10c3ef4e75cc13f172da0af0809e5c7a21ba559d8530903ea569f6751b4f3e55"
BASELINE_CARGO_SEED_TOTAL_BYTES = 50_190_281
BASELINE_CARGO_GENERATED_SOURCE_TREE_SHA256 = "sha256:4b2049df9a67d32b79c4427c90edb8a31f4a46c5ecf743eb4ff190f5d46dc332"
BASELINE_CARGO_GENERATED_SOURCE_FILE_COUNT = 5_116
BASELINE_CARGO_NON_RESOLVED_KEYS = frozenset({
    ("anstyle-wincon", "3.0.11"), ("bumpalo", "3.20.3"),
    ("curve25519-dalek-derive", "0.1.1"), ("fiat-crypto", "0.2.9"),
    ("futures-core", "0.3.33"), ("futures-task", "0.3.33"),
    ("futures-util", "0.3.33"), ("js-sys", "0.3.103"),
    ("linux-raw-sys", "0.12.1"), ("once_cell_polyfill", "1.70.2"),
    ("pin-project-lite", "0.2.17"), ("r-efi", "6.0.0"),
    ("rsqlite-vfs", "0.1.1"), ("rustversion", "1.0.23"),
    ("slab", "0.4.12"), ("sqlite-wasm-rs", "0.5.5"),
    ("wasi", "0.11.1+wasi-snapshot-preview1"), ("wasm-bindgen", "0.2.126"),
    ("wasm-bindgen-macro", "0.2.126"), ("wasm-bindgen-macro-support", "0.2.126"),
    ("wasm-bindgen-shared", "0.2.126"), ("winapi-util", "0.1.11"),
    ("windows-link", "0.2.1"), ("windows-sys", "0.61.2"),
})
BASELINE_CARGO_AVAILABLE_NON_RESOLVED_KEYS = frozenset({
    ("pin-project-lite", "0.2.17"), ("slab", "0.4.12"),
})
PREPARE_ONLINE_NETWORK_POLICY = "EXPLICIT_ALLOW_NETWORK"
PREPARE_OFFLINE_NETWORK_POLICY = "DENY_DEFAULT_NO_NETWORK_ALLOW"
SECURITY_SUPERVISOR_EXPECTED = {
    "sandbox_network_env":"DENIED_AND_ISOLATED",
    "sandbox_secret_paths":"DENIED",
    "sandbox_unix_network":"DENIED",
    "sandbox_source_write":"DENIED",
    "sandbox_keychain_read":"DENIED",
    "sandbox_background_child":"TERMINATED_WITH_GROUP",
}
PREPARE_SECURITY_CASES = frozenset({
    "prepareMavenLauncherTraversalPassed", "prepareSourceAncestryTraversalPassed",
    "prepareAncestorSecretReadDenied", "prepareAncestorWriteDenied",
    "prepareSelectedSourceWriteDenied", "prepareKeychainReadDenied",
    "prepareTraversalNetworkSemanticsPreserved", "prepareAncestorDataOnlyMutationRejected",
    "prepareBroadSandboxPermissionRejected", "prepareRootAuthoritySubstitutionsRejected",
    "prepareSplitPhaseRootsRejected",
    "prepareDevNullWriteDataPassed", "prepareOnlineVarMetadataOnlyPassed",
    "prepareMissingProfileClauseRejected", "prepareBroadDevNullWriteRejected",
    "prepareOfflineVarAliasRejected", "prepareWrongMavenTmpdirRejected",
    "prepareSplitPhaseEnvironmentRejected",
    "prepareGradleWrapperBootstrapHomePassed",
    "prepareMissingGradleWrapperBootstrapHomeRejected",
    "prepareGradleJvmTmpdirAuthorityPassed",
    "prepareMissingGradleJvmTmpdirRejected",
    "prepareWrongGradleJvmTmpdirRejected",
    "prepareGradleJvmTmpdirFailureClassifiedInfrastructure",
    "prepareGradleStrictOfflineFailureTypedRefusal",
    "prepareGradleStrictOfflineWrongProfileSecurityRejected",
    "prepareGradleOnlineSecurityFailureRejected",
    "prepareMavenOfflineSecurityFailureRejected",
    "prepareMavenOfflineModelGoalsPrefetchedOnline",
    "preparePostPublicationEvidenceRevalidated",
})
FILTER_FREE_SOURCE_CASES = frozenset({
    "repositoryFilterNeverExecuted", "trackedDirtyDetectedWithoutFilter",
    "untrackedDetectedWithoutFilter", "missingDetectedWithoutFilter",
    "repositoryLocalFilterIdentityRejected", "exportSubstTransformationSuppressed",
    "exportIgnoreMutationRejected", "crlfWorktreeBytesAccepted", "rawBlobIdentityImported",
})
PREPARE_NETWORK_SENTINEL_CODE = (
    "socket(S,PF_INET,SOCK_STREAM,6) or exit 31;"
    "connect(S,sockaddr_in(9,inet_aton(\"127.0.0.1\"))) "
    "or exit(($!{EPERM}||$!{EACCES})?0:41);exit 42"
)
PREPARE_NETWORK_SENTINEL_ARGV = ["/usr/bin/perl", "-MSocket", "-e", PREPARE_NETWORK_SENTINEL_CODE]
SANDBOX_METADATA_LITERAL = re.compile(
    r'^\(allow file-read-data file-read-metadata \(literal ("(?:\\.|[^"\\])*")\)\)$'
)
SANDBOX_CONTENT_ROOT = re.compile(
    r'^\(allow file-read\* \((literal|subpath) ("(?:\\.|[^"\\])*")\)\)$'
)
SANDBOX_WRITE_ROOT = re.compile(
    r'^\(allow file-write\* \(subpath ("(?:\\.|[^"\\])*")\)\)$'
)
SANDBOX_DEV_NULL_WRITE = '(allow file-write-data (literal "/dev/null"))'
SANDBOX_ONLINE_VAR_METADATA = '(allow file-read-metadata (literal "/var"))'
PREPARE_STAGING_ROOT = re.compile(
    r'^\.(qualificationDependencySeed|holdoutDependencySeed)\.prepare-[0-9a-f]{24}$'
)
TRUSTED_WORKER_BUILDER_SHA256 = "sha256:6d853cfe8966dbde89caf6177b6757eb39256cecc9ec92afe8c8d6046d082030"
TRUSTED_WORKER_BUILD_INPUTS = {
    "2.1": "sha256:44d0261cdf93e56527058d34abfc5aac2fa9e10b989292438b288d9c289200d5",
    "2.3": "sha256:a9933f98bd95313043761c661d6e256fa91a39a487c21d6699b807df4b3f0096",
    "2.4": "sha256:df42131aee6e91b22425f846ffab8529d624360242da6edef4dfd75519387696",
}


def canonical(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()


def digest(raw: bytes) -> str:
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def is_digest(value: Any) -> bool:
    return isinstance(value, str) and value.startswith("sha256:") and len(value) == 71 and all(character in "0123456789abcdef" for character in value[7:])


def rust_digest(value: Any) -> str:
    return digest(canonical(value).removesuffix(b"\n"))


def file_digest(path: Path) -> str:
    return digest(path.read_bytes())


def security_tripwire_cases_valid(supervisor_cases: Any, requirement_cases: Any) -> bool:
    return (
        isinstance(supervisor_cases, Mapping)
        and all(supervisor_cases.get(name) == value for name,value in SECURITY_SUPERVISOR_EXPECTED.items())
        and isinstance(requirement_cases, Mapping)
        and all(requirement_cases.get(name) is True for name in PREPARE_SECURITY_CASES | FILTER_FREE_SOURCE_CASES)
    )


def sandbox_read_closure_valid(profile_raw: str) -> bool:
    """Independently restrict ancestor access to literal directory traversal."""
    metadata_paths: list[Path] = []
    content_roots: list[tuple[str, Path]] = []
    for line in profile_raw.splitlines():
        line = line.strip()
        if "file-read" not in line:
            continue
        metadata_match = SANDBOX_METADATA_LITERAL.fullmatch(line)
        content_match = SANDBOX_CONTENT_ROOT.fullmatch(line)
        if metadata_match is None and content_match is None:
            return False
        try:
            if metadata_match is not None:
                value = json.loads(metadata_match.group(1))
                selector = None
            else:
                value = json.loads(content_match.group(2))
                selector = content_match.group(1)
        except json.JSONDecodeError:
            return False
        if not isinstance(value, str) or not value.startswith("/") or str(Path(value)) != value:
            return False
        if selector is None:
            metadata_paths.append(Path(value))
        else:
            content_roots.append((selector, Path(value)))
    if not metadata_paths or not content_roots:
        return False
    expected_metadata = {Path("/")}
    for selector, path in content_roots:
        current = path if selector == "subpath" else path.parent
        while current != current.parent:
            expected_metadata.add(current)
            current = current.parent
    return (
        len(metadata_paths) == len(set(metadata_paths))
        and set(metadata_paths) == expected_metadata
        and len(content_roots) == len(set(content_roots))
    )


def sandbox_expected_content_roots(
    entry_work: Path, phase: str,
) -> set[tuple[str, Path]]:
    roots: set[tuple[str, Path]] = {
        ("subpath", entry_work),
        ("subpath", entry_work / "disposable-sources" / phase),
    }
    for path in (
        Path("/System"), Path("/usr"), Path("/bin"), Path("/sbin"), Path("/etc"),
        Path("/Library/Java"), Path("/opt/homebrew"), Path("/dev"),
        Path("/private/var/select"),
    ):
        roots.add(("subpath", path.resolve(strict=False)))
    roots.add((
        "literal",
        (ROOT / "workers/kotlin/src/main/resources/semantic-thread-model.init.gradle").resolve(strict=False),
    ))
    return roots


def preparation_environment(entry_work: Path, build_dsl: str) -> dict[str, str]:
    entry_work = entry_work.resolve(strict=False)
    home = entry_work / "home"
    environment = {
        "HOME": str(home), "USERPROFILE": str(home),
        "PATH": "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home/bin:/opt/homebrew/Cellar/maven/3.9.12/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        "JAVA_HOME": "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home",
        "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", "TMPDIR": str(home),
        "MAVEN_OPTS": f"-Djava.io.tmpdir={home}", "CODECLEW_K1_MODEL_CALLS": "0",
    }
    if build_dsl in {"GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"}:
        environment["GRADLE_USER_HOME"] = str(entry_work / "gradle-user-home")
        environment["GRADLE_OPTS"] = f"-Djava.io.tmpdir={home}"
    return environment


def preparation_environments_valid(
    environments: Any, entry_work: Path, build_dsl: Any,
) -> bool:
    if build_dsl not in {"MAVEN", "GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"}:
        return False
    expected = preparation_environment(entry_work, build_dsl)
    record = {"environment": expected, "environmentSha256": digest(canonical(expected))}
    return (
        isinstance(environments, Mapping)
        and set(environments) == {"online", "offline"}
        and environments.get("online") == record
        and environments.get("offline") == record
    )


def preparation_profile(entry_work: Path, phase: str) -> str:
    roots = sandbox_expected_content_roots(entry_work, phase)
    ancestors: set[Path] = {Path("/")}
    for selector, path in roots:
        current = path if selector == "subpath" else path.parent
        while current != current.parent:
            ancestors.add(current)
            current = current.parent
    read_lines = [
        f"(allow file-read-data file-read-metadata (literal {json.dumps(str(path))}))"
        for path in sorted(ancestors, key=lambda value: (len(value.parts), str(value)))
    ]
    read_lines.extend(
        f"(allow file-read* ({selector} {json.dumps(str(path))}))"
        for selector, path in sorted(roots, key=lambda value: (str(value[1]), value[0]))
    )
    lines = [
        "(version 1)", "(deny default)", "(allow process*)",
        "(allow network*)" if phase == "online" else "(deny network*)",
        "(allow sysctl-read)", "(allow mach-lookup)",
        '(deny mach-lookup (global-name "com.apple.securityd"))',
        '(deny mach-lookup (global-name "com.apple.security.agent"))',
        '(deny mach-lookup (global-name "com.apple.trustd"))',
    ]
    if phase == "online":
        lines.append(SANDBOX_ONLINE_VAR_METADATA)
    lines.extend(read_lines)
    lines.extend((SANDBOX_DEV_NULL_WRITE, f"(allow file-write* (subpath {json.dumps(str(entry_work))}))"))
    return "\n".join(lines) + "\n"


def expected_dependency_prepare_argv(
    entry: Mapping[str, Any], repository: Path, staging: Path, *, offline: bool,
) -> list[str] | None:
    repository = repository.resolve(strict=False)
    staging = staging.resolve(strict=False)
    selected = entry.get("selectedCompilation")
    identifier = entry.get("entry", entry.get("id"))
    if not isinstance(selected, str) or not isinstance(identifier, str) or "/" not in selected:
        return None
    if entry.get("buildDsl") == "MAVEN":
        module = selected.rsplit("/", 1)[0].removeprefix(":").replace(":", "/")
        selected_pom = repository / module / "pom.xml" if module else repository / "pom.xml"
        reactor_selector = ["-pl", module, "-am"] if module else []
        classpath_output = staging / "model-evidence" / f"{identifier}.classpath"
        base = [
            "/opt/homebrew/Cellar/maven/3.9.12/bin/mvn", "-B", "-q", "-DskipTests",
            f"-Dmaven.repo.local={staging / 'maven-repository'}",
            f"-Duser.home={staging / 'home'}",
        ]
        model_probe = [
            f"-Dmdep.outputFile={classpath_output}", "-Dmdep.includeScope=compile",
            "help:effective-pom", "dependency:build-classpath",
        ]
        return (
            [*base, "-o", "-f", str(selected_pom), *model_probe]
            if offline else [
                *base, *reactor_selector, *model_probe[:2],
                "dependency:go-offline", "install", *model_probe[2:],
            ]
        )
    if entry.get("buildDsl") not in {"GRADLE_KOTLIN_DSL", "GRADLE_GROOVY_DSL"}:
        return None
    project_path = selected.rsplit("/", 1)[0] or ":"
    source_set = selected.rsplit("/", 1)[1]
    if not source_set:
        return None
    compile_task = "compileKotlin" if source_set == "main" else f"compile{source_set[:1].upper()}{source_set[1:]}Kotlin"
    model_task = ":semanticThreadModel" if project_path == ":" else f"{project_path}:semanticThreadModel"
    init_script = ROOT / "workers/kotlin/src/main/resources/semantic-thread-model.init.gradle"
    project_cache = staging / "ephemeral-project-cache" / identifier / (
        "offline-verification" if offline else "online"
    )
    values = [
        str(repository / "gradlew"), "-p", str(repository),
        "--gradle-user-home", str(staging / "gradle-user-home"),
        "--project-cache-dir", str(project_cache), "--no-daemon", "--stacktrace",
        f"-Duser.home={staging / 'home'}",
        f"-Pkotlin.project.persistent.dir={project_cache / 'kotlin'}",
        "-I", str(init_script), f"-Dsemantic.thread.compileTask={compile_task}", model_task,
    ]
    if offline:
        values.insert(3, "--offline")
    return values


def sandbox_profile_shape_valid(
    profile_raw: str,
    network_clause: str,
    entry: Mapping[str, Any],
    phase: str,
    command: list[Any],
) -> bool:
    lines = profile_raw.splitlines()
    expected_prefix = [
        "(version 1)", "(deny default)", "(allow process*)", network_clause,
        "(allow sysctl-read)", "(allow mach-lookup)",
        '(deny mach-lookup (global-name "com.apple.securityd"))',
        '(deny mach-lookup (global-name "com.apple.security.agent"))',
        '(deny mach-lookup (global-name "com.apple.trustd"))',
    ]
    special_read_lines = [SANDBOX_ONLINE_VAR_METADATA] if phase == "online" else []
    if (
        len(lines) <= len(expected_prefix) + len(special_read_lines) + 2
        or lines[:len(expected_prefix)] != expected_prefix
        or lines[len(expected_prefix):len(expected_prefix) + len(special_read_lines)] != special_read_lines
        or lines[-2] != SANDBOX_DEV_NULL_WRITE
    ):
        return False
    read_lines = lines[len(expected_prefix) + len(special_read_lines):-2]
    write_match = SANDBOX_WRITE_ROOT.fullmatch(lines[-1])
    if write_match is None or not all("file-read" in line for line in read_lines):
        return False
    try:
        write_root = json.loads(write_match.group(1))
    except json.JSONDecodeError:
        return False
    if not isinstance(write_root, str) or not write_root.startswith("/") or str(Path(write_root)) != write_root:
        return False
    entry_id = entry.get("entry", entry.get("id"))
    entry_work = Path(write_root)
    if (
        phase not in {"online", "offline"}
        or not isinstance(entry_id, str)
        or entry_work.name != entry_id
        or entry_work.parent.name != ".work"
        or PREPARE_STAGING_ROOT.fullmatch(entry_work.parent.parent.name) is None
    ):
        return False
    actual_content_roots: list[tuple[str, Path]] = []
    for line in read_lines:
        match = SANDBOX_CONTENT_ROOT.fullmatch(line)
        if match is None:
            continue
        try:
            value = json.loads(match.group(2))
        except json.JSONDecodeError:
            return False
        if not isinstance(value, str):
            return False
        actual_content_roots.append((match.group(1), Path(value)))
    expected_command = expected_dependency_prepare_argv(
        entry, entry_work / "disposable-sources" / phase, entry_work,
        offline=phase == "offline",
    )
    return (
        profile_raw == preparation_profile(entry_work, phase)
        and command == expected_command
        and len(actual_content_roots) == len(set(actual_content_roots))
        and set(actual_content_roots) == sandbox_expected_content_roots(entry_work, phase)
        and sandbox_read_closure_valid("\n".join(read_lines))
    )


def sandbox_profile_write_root(profile_raw: str) -> Path | None:
    lines = profile_raw.splitlines()
    if not lines:
        return None
    match = SANDBOX_WRITE_ROOT.fullmatch(lines[-1])
    if match is None:
        return None
    try:
        value = json.loads(match.group(1))
    except json.JSONDecodeError:
        return None
    return Path(value) if isinstance(value, str) and value.startswith("/") else None


def cargo_index_relative(name: str) -> Path:
    lowered = name.lower()
    if len(lowered) == 1:
        return Path("1") / lowered
    if len(lowered) == 2:
        return Path("2") / lowered
    if len(lowered) == 3:
        return Path("3") / lowered[0] / lowered
    return Path(lowered[:2]) / lowered[2:4] / lowered


def cargo_config_discovery_absent() -> bool:
    return all(
        not (ancestor / ".cargo" / name).exists()
        and not (ancestor / ".cargo" / name).is_symlink()
        for ancestor in (ROOT, *ROOT.parents)
        for name in ("config", "config.toml")
    )


def cargo_command_argv(argv: tuple[str, ...] | list[str]) -> list[str] | None:
    if len(argv) < 2 or argv[0] != "cargo":
        return None
    return ["$CARGO_1_92_0", argv[1], "--manifest-path", "$REPOSITORY/Cargo.toml", *argv[2:]]


def candidate_cargo_baseline_valid(tools: Any) -> bool:
    return isinstance(tools, Mapping) and tools.get("cargoBaseline") == {
        "launcher": cargo_launcher_expected(),
        "target": BASELINE_CARGO_TARGET,
        "registry": BASELINE_CARGO_REGISTRY,
        "cargoLockSha256": file_digest(ROOT / "Cargo.lock"),
    }


def cargo_lock_projection() -> tuple[list[dict[str, str]], list[dict[str, str]]] | None:
    """Independently derive the preregistered target partition from live Cargo.lock."""
    try:
        lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError):
        return None
    packages = lock.get("package")
    if lock.get("version") != 4 or not isinstance(packages, list):
        return None
    locked: dict[tuple[str, str], str] = {}
    for row in packages:
        if not isinstance(row, Mapping) or not str(row.get("source", "")).startswith("registry+"):
            continue
        name, version, checksum = row.get("name"), row.get("version"), row.get("checksum")
        if (
            row.get("source") != "registry+https://github.com/rust-lang/crates.io-index"
            or not isinstance(name, str) or not name or not isinstance(version, str) or not version
            or not isinstance(checksum, str) or len(checksum) != 64
            or any(character not in "0123456789abcdef" for character in checksum)
            or (name, version) in locked
        ):
            return None
        locked[(name, version)] = checksum
    if (
        len(locked) != 135 or len({name for name, _ in locked}) != 130
        or len(BASELINE_CARGO_NON_RESOLVED_KEYS) != 24
        or not BASELINE_CARGO_NON_RESOLVED_KEYS < set(locked)
        or not BASELINE_CARGO_AVAILABLE_NON_RESOLVED_KEYS < BASELINE_CARGO_NON_RESOLVED_KEYS
    ):
        return None
    resolved = [
        {"name": name, "version": version, "checksum": locked[(name, version)]}
        for name, version in sorted(set(locked) - BASELINE_CARGO_NON_RESOLVED_KEYS)
    ]
    non_resolved = [
        {"name": name, "version": version, "checksum": locked[(name, version)]}
        for name, version in sorted(BASELINE_CARGO_NON_RESOLVED_KEYS)
    ]
    if (
        len(resolved) != 111 or digest(canonical(resolved)) != BASELINE_CARGO_RESOLVED_SHA256
        or digest(canonical(non_resolved)) != BASELINE_CARGO_NON_RESOLVED_SHA256
    ):
        return None
    return resolved, non_resolved


def cargo_seed_lock_valid(seed: Any) -> bool:
    projection = cargo_lock_projection()
    if projection is None or not isinstance(seed, Mapping):
        return False
    expected_resolved, expected_non_resolved = projection
    resolved, non_resolved = seed.get("resolvedPackages"), seed.get("nonResolvedLockedPackages")
    unavailable, excluded, files = seed.get("unavailableLockedArchives"), seed.get("availableNonResolvedArchivesExcluded"), seed.get("files")
    expected_excluded = [
        row for row in expected_non_resolved
        if (row["name"], row["version"]) in BASELINE_CARGO_AVAILABLE_NON_RESOLVED_KEYS
    ]
    expected_unavailable = [row for row in expected_non_resolved if row not in expected_excluded]
    if (
        resolved != expected_resolved or non_resolved != expected_non_resolved
        or unavailable != expected_unavailable or excluded != expected_excluded
        or seed.get("resolvedPackagesSha256") != BASELINE_CARGO_RESOLVED_SHA256
        or seed.get("metadataResolvedPackagesSha256") != BASELINE_CARGO_RESOLVED_SHA256
        or seed.get("nonResolvedLockedPackagesSha256") != BASELINE_CARGO_NON_RESOLVED_SHA256
        or seed.get("unavailableLockedArchivesSha256") != digest(canonical(expected_unavailable))
        or not isinstance(files, list)
    ):
        return False
    file_by_path: dict[str, Mapping[str, Any]] = {}
    for row in files:
        if (
            not isinstance(row, Mapping) or set(row) != {"path", "size", "sha256"}
            or not isinstance(row.get("path"), str) or row["path"] in file_by_path
            or not isinstance(row.get("size"), int) or isinstance(row.get("size"), bool) or row["size"] <= 0
            or not is_digest(row.get("sha256"))
        ):
            return False
        file_by_path[row["path"]] = row
    config_path = f"registry/index/{BASELINE_CARGO_REGISTRY}/config.json"
    index_paths = {
        (Path("registry/index") / BASELINE_CARGO_REGISTRY / ".cache" / cargo_index_relative(name)).as_posix()
        for name in {row["name"] for row in expected_resolved + expected_non_resolved}
    }
    archive_rows = {
        f"registry/cache/{BASELINE_CARGO_REGISTRY}/{row['name']}-{row['version']}.crate": "sha256:" + row["checksum"]
        for row in expected_resolved
    }
    return (
        set(file_by_path) == {config_path, *index_paths, *archive_rows}
        and file_by_path[config_path].get("sha256") == BASELINE_CARGO_CONFIG_SHA256
        and all(file_by_path[path].get("sha256") == checksum for path, checksum in archive_rows.items())
        and seed.get("fileCount") == len(files) == 242
        and seed.get("totalBytes") == sum(row["size"] for row in files)
    )


def cargo_launcher_expected() -> dict[str, Any]:
    version_outputs = {
        "cargo": b"cargo 1.92.0 (Homebrew)\n",
        "rustc": b"rustc 1.92.0 (ded5c06cf 2025-12-08) (Homebrew)\nbinary: rustc\ncommit-hash: ded5c06cf21d2b93bffd5d884aa6e96934ee4234\ncommit-date: 2025-12-08\nhost: aarch64-apple-darwin\nrelease: 1.92.0\nLLVM version: 21.1.7\n",
        "rustfmt": b"rustfmt 1.8.0\n", "cargoFmt": b"rustfmt 1.8.0\n",
        "cargoClippy": b"clippy 0.1.92\n", "clippyDriver": b"clippy 0.1.92\n",
    }
    version_argv = {
        "cargo": ["$RUST_1_92_0/cargo", "-V"], "rustc": ["$RUST_1_92_0/rustc", "-vV"],
        "rustfmt": ["$RUST_1_92_0/rustfmt", "-V"],
        "cargoFmt": ["$RUST_1_92_0/cargo-fmt", "--version"],
        "cargoClippy": ["$RUST_1_92_0/cargo-clippy", "-V"],
        "clippyDriver": ["$RUST_1_92_0/clippy-driver", "-V"],
    }
    return {
        "schema": "codeclew.kotlin-k1-cargo-launcher-authority/0.1",
        "toolchainSha256": file_digest(ROOT / "rust-toolchain.toml"), "channel": "1.92.0",
        "tools": {name: {"requestedPath": str(path), "resolvedRelativePath": path.name, "sha256": file_digest(path)} for name, path in BASELINE_CARGO_TOOL_PATHS.items()},
        "versionIdentities": {name: {"argv": version_argv[name], "stdoutSha256": digest(raw), "stdoutBytes": len(raw)} for name, raw in version_outputs.items()},
        "hostPathRetained": False,
    }


def cargo_execution_valid(authority: Any, expected_argv: tuple[str, ...]) -> bool:
    if not isinstance(authority, Mapping) or set(authority) != {
        "launcher", "dependencySeed", "executionArgv", "executionArgvSha256",
        "executionCwd", "isolatedCargoHome", "isolatedCargoTargetDir", "sharedBaselineExecutionContext",
    } or authority.get("launcher") != cargo_launcher_expected():
        return False
    seed = authority.get("dependencySeed")
    if not isinstance(seed, Mapping) or set(seed) != {
        "schema", "registry", "target", "cargoLockSha256", "resolvedPackages",
        "resolvedPackagesSha256", "metadataResolvedPackagesSha256", "nonResolvedLockedPackages",
        "nonResolvedLockedPackagesSha256", "unavailableLockedArchives",
        "unavailableLockedArchivesSha256", "availableNonResolvedArchivesExcluded", "files",
        "sourceTreeDigest", "copiedTreeDigest", "sourceAfterTreeDigest", "fileCount", "totalBytes",
        "sourceCopyEqual", "credentialInputsCopied", "forbiddenCredentialFilesPresent",
        "sourcePathRetained", "offlineFetchProbe",
        "generatedSourceTreeDigest", "generatedSourceFileCount",
        "configAndCredentialsAbsentBeforeFetch", "configAndCredentialsAbsentAfterFetch",
    }:
        return False
    resolved, non_resolved = seed.get("resolvedPackages"), seed.get("nonResolvedLockedPackages")
    unavailable, excluded, files = seed.get("unavailableLockedArchives"), seed.get("availableNonResolvedArchivesExcluded"), seed.get("files")
    def packages_valid(rows: Any, count: int) -> bool:
        return (
            isinstance(rows, list) and len(rows) == count
            and rows == sorted(rows, key=lambda row: (row.get("name", ""), row.get("version", "")))
            and all(isinstance(row, Mapping) and set(row) == {"name", "version", "checksum"}
                    and isinstance(row["name"], str) and isinstance(row["version"], str)
                    and isinstance(row["checksum"], str) and len(row["checksum"]) == 64
                    and all(character in "0123456789abcdef" for character in row["checksum"]) for row in rows)
        )
    if not (packages_valid(resolved, 111) and packages_valid(non_resolved, 24) and packages_valid(unavailable, 22) and packages_valid(excluded, 2)):
        return False
    files_valid = (
        isinstance(files, list) and len(files) == 242 and files == sorted(files, key=lambda row: row.get("path", ""))
        and all(isinstance(row, Mapping) and set(row) == {"path", "size", "sha256"}
                and isinstance(row["path"], str) and not row["path"].startswith("/")
                and isinstance(row["size"], int) and not isinstance(row["size"], bool) and row["size"] > 0
                and is_digest(row["sha256"]) for row in files)
    )
    fetch = seed.get("offlineFetchProbe")
    normalized_fetch = cargo_command_argv(
        ["cargo", "fetch", "--offline", "--locked", "--target", BASELINE_CARGO_TARGET]
    )
    seed_valid = (
        seed.get("schema") == "codeclew.kotlin-k1-cargo-dependency-seed/0.1"
        and cargo_config_discovery_absent()
        and seed.get("registry") == BASELINE_CARGO_REGISTRY and seed.get("target") == BASELINE_CARGO_TARGET
        and seed.get("cargoLockSha256") == file_digest(ROOT / "Cargo.lock") and files_valid
        and cargo_seed_lock_valid(seed)
        and seed.get("resolvedPackagesSha256") == digest(canonical(resolved))
        and seed.get("metadataResolvedPackagesSha256") == digest(canonical(resolved))
        and seed.get("nonResolvedLockedPackagesSha256") == digest(canonical(non_resolved))
        and seed.get("unavailableLockedArchivesSha256") == digest(canonical(unavailable))
        and {tuple(sorted(row.items())) for row in unavailable + excluded} == {tuple(sorted(row.items())) for row in non_resolved}
        and seed.get("sourceTreeDigest") == BASELINE_CARGO_SEED_TREE_SHA256
        and seed.get("sourceTreeDigest") == digest(canonical({"schema": "codeclew.kotlin-k1-cargo-seed-tree/0.1", "files": files}))
        and seed.get("sourceTreeDigest") == seed.get("copiedTreeDigest") == seed.get("sourceAfterTreeDigest")
        and seed.get("fileCount") == 242 and seed.get("totalBytes") == BASELINE_CARGO_SEED_TOTAL_BYTES
        and seed.get("generatedSourceTreeDigest") == BASELINE_CARGO_GENERATED_SOURCE_TREE_SHA256
        and seed.get("generatedSourceFileCount") == BASELINE_CARGO_GENERATED_SOURCE_FILE_COUNT
        and seed.get("configAndCredentialsAbsentBeforeFetch") is True
        and seed.get("configAndCredentialsAbsentAfterFetch") is True
        and seed.get("sourceCopyEqual") is True and seed.get("credentialInputsCopied") is False
        and seed.get("forbiddenCredentialFilesPresent") is False and seed.get("sourcePathRetained") is False
        and isinstance(fetch, Mapping) and set(fetch) == {"executionArgv", "executionArgvSha256", "executionCwd", "exitCode", "stdoutSha256", "stdoutBytes", "stderrSha256", "stderrBytes"}
        and fetch.get("executionArgv") == normalized_fetch and fetch.get("executionArgvSha256") == digest(canonical(normalized_fetch))
        and fetch.get("executionCwd") == "/" and fetch.get("exitCode") == 0
        and fetch.get("stdoutSha256") == fetch.get("stderrSha256") == digest(b"")
        and fetch.get("stdoutBytes") == fetch.get("stderrBytes") == 0
    )
    normalized = cargo_command_argv(expected_argv)
    return (
        seed_valid and authority.get("executionArgv") == normalized
        and authority.get("executionArgvSha256") == digest(canonical(normalized))
        and authority.get("executionCwd") == "/"
        and authority.get("isolatedCargoHome") is True and authority.get("isolatedCargoTargetDir") is True
        and authority.get("sharedBaselineExecutionContext") is True
        and not re.search(r"(?:/Users/|/home/)[^/]+", json.dumps(authority, sort_keys=True))
    )


def baseline_expected_environment_values(is_gradle: bool) -> dict[str, str]:
    values = {
        "HOME":"$ISOLATED", "TMPDIR":"$ISOLATED",
        "PATH":BASELINE_PATH,
        "LANG":"C.UTF-8", "LC_ALL":"C.UTF-8", "CODECLEW_K1_MODEL_CALLS":"0",
    }
    if is_gradle:
        values.update({
            "JAVA_HOME":BASELINE_JAVA_HOME,
            "GRADLE_USER_HOME":"$ISOLATED/gradle-user-home",
        })
    else:
        values.update({
            "CARGO_HOME":"$ISOLATED/cargo-home",
            "CARGO_TARGET_DIR":"$ISOLATED/cargo-target",
            "CARGO_NET_OFFLINE":"true",
            "CARGO_REGISTRIES_CRATES_IO_PROTOCOL":"sparse",
        })
    return values


def baseline_environment_policy_valid(policy: Any, is_gradle: bool) -> bool:
    expected_values = baseline_expected_environment_values(is_gradle)
    if not isinstance(policy,Mapping) or policy != {
        "keys":sorted(expected_values), "values":expected_values,
        "credentialInheritance":False,
    }:
        return False
    keys = set(policy["values"])
    if is_gradle:
        return "JAVA_TOOL_OPTIONS" not in keys
    return (
        not (BASELINE_CARGO_FORBIDDEN_INJECTION_KEYS & keys)
        and not any(key.startswith("ORG_GRADLE_PROJECT_") for key in keys)
    )


def baseline_command_valid(row: Any, expected_argv: tuple[str, ...]) -> bool:
    if not isinstance(row, Mapping) or row.get("argv") != list(expected_argv):
        return False
    policy = row.get("environmentPolicy")
    if not isinstance(policy, Mapping) or row.get("environmentPolicySha256") != digest(canonical(policy)):
        return False
    is_gradle = expected_argv[0] == "./gradlew"
    is_cargo = expected_argv[0] == "cargo"
    if (
        (not is_gradle and not is_cargo)
        or not baseline_environment_policy_valid(policy,is_gradle)
        or row.get("argvSha256") != digest(canonical(list(expected_argv)))
        or not is_digest(row.get("stdoutSha256")) or not is_digest(row.get("stderrSha256"))
        or not all(isinstance(row.get(key),int) and not isinstance(row.get(key),bool) and row[key] >= 0 for key in (
            "stdoutBytes","stderrBytes","wallMicros",
        ))
        or not isinstance(row.get("exitCode"),int) or isinstance(row.get("exitCode"),bool)
        or row.get("observed") != ("PASS" if row["exitCode"] == 0 else "FAIL")
        or not is_digest(row.get("executionContextId"))
    ):
        return False
    authority = row.get("gradleExecutionAuthority")
    cargo_authority = row.get("cargoExecutionAuthority")
    if is_cargo:
        return authority is None and cargo_execution_valid(cargo_authority, expected_argv)
    if cargo_authority is not None or not is_gradle:
        return authority is None and cargo_authority is None
    if not isinstance(authority,Mapping) or set(authority) != {
        "launcher","dependencySeed","executionArgv","executionArgvSha256",
        "isolatedGradleUserHome","isolatedJavaUserHome",
    }:
        return False
    launcher, seed = authority.get("launcher"), authority.get("dependencySeed")
    if not isinstance(launcher,Mapping) or not isinstance(seed,Mapping):
        return False
    launcher_valid = (
        set(launcher) == {
            "schema","requestedLauncher","version","distributionUrl","wrapperScriptSha256",
            "wrapperJarSha256","wrapperPropertiesSha256","distributionTreeDigest",
            "distributionFileCount","distributionBytes","executableRelativePath","executableSha256",
            "coreJarRelativePath","coreJarSha256","hostPathRetained",
        }
        and launcher.get("schema") == "codeclew.kotlin-k1-gradle-launcher-authority/0.1"
        and launcher.get("requestedLauncher") == "./gradlew"
        and launcher.get("version") == BASELINE_GRADLE_VERSION
        and launcher.get("distributionUrl") == BASELINE_GRADLE_URL
        and launcher.get("wrapperScriptSha256") == file_digest(ROOT / "gradlew")
        and launcher.get("wrapperJarSha256") == file_digest(ROOT / "gradle/wrapper/gradle-wrapper.jar")
        and launcher.get("wrapperPropertiesSha256") == file_digest(ROOT / "gradle/wrapper/gradle-wrapper.properties")
        and all(is_digest(launcher.get(key)) for key in (
            "distributionTreeDigest","executableSha256","coreJarSha256",
        ))
        and launcher.get("executableRelativePath") == "bin/gradle"
        and launcher.get("coreJarRelativePath") == f"lib/gradle-core-{BASELINE_GRADLE_VERSION}.jar"
        and all(isinstance(launcher.get(key),int) and not isinstance(launcher.get(key),bool) and launcher[key] > 0 for key in (
            "distributionFileCount","distributionBytes",
        ))
        and launcher.get("hostPathRetained") is False
    )
    seed_valid = (
        set(seed) == {
            "schema","copiedSubtree","transientExclusionSuffixes","excludedTransientCount",
            "excludedTransientPathsSha256","sourceTreeDigest","copiedTreeDigest",
            "sourceAfterTreeDigest","fileCount","totalBytes","sourceCopyEqual",
            "credentialInputsCopied","forbiddenCredentialFilesPresent","sourcePathRetained",
        }
        and seed.get("schema") == "codeclew.kotlin-k1-gradle-dependency-seed/0.1"
        and seed.get("copiedSubtree") == "caches/modules-2"
        and seed.get("transientExclusionSuffixes") == BASELINE_TRANSIENT_SUFFIXES
        and isinstance(seed.get("excludedTransientCount"),int)
        and not isinstance(seed.get("excludedTransientCount"),bool)
        and seed["excludedTransientCount"] >= 0
        and all(is_digest(seed.get(key)) for key in (
            "excludedTransientPathsSha256","sourceTreeDigest","copiedTreeDigest","sourceAfterTreeDigest",
        ))
        and seed.get("sourceTreeDigest") == seed.get("copiedTreeDigest") == seed.get("sourceAfterTreeDigest")
        and all(isinstance(seed.get(key),int) and not isinstance(seed.get(key),bool) and seed[key] > 0 for key in (
            "fileCount","totalBytes",
        ))
        and seed.get("sourceCopyEqual") is True
        and seed.get("credentialInputsCopied") is False
        and seed.get("forbiddenCredentialFilesPresent") is False
        and seed.get("sourcePathRetained") is False
    )
    execution_argv = ["$GRADLE_9_6_1","-Duser.home=$ISOLATED",*expected_argv[1:]]
    serialized = json.dumps(authority,sort_keys=True)
    return (
        launcher_valid and seed_valid
        and authority.get("executionArgv") == execution_argv
        and authority.get("executionArgvSha256") == digest(canonical(execution_argv))
        and authority.get("isolatedGradleUserHome") is True
        and authority.get("isolatedJavaUserHome") is True
        and not re.search(r"(?:/Users/|/home/)[^/]+", serialized)
    )


def source_anchor_packet() -> dict[str, Any]:
    adapter_path = ROOT / "crates/evidence-adapters/src/bin/kotlin.rs"
    cache_path = ROOT / "crates/evidence-adapters/src/bin/kotlin_k1.rs"
    harness_path = ROOT / "scripts/k1_kotlin_real_repository.py"
    adapter = adapter_path.read_text(encoding="utf-8")
    production = adapter.split("#[cfg(test)]", 1)[0]
    request_kinds = sorted(set(re.findall(r"RequestKind::([A-Za-z0-9_]+)", production)))
    worker_calls = sorted(set(re.findall(r"\bworker\.([A-Za-z0-9_]+)\s*\(", production)))
    cli_block = production[production.index("struct Args"):production.index("enum RunPhase")]
    cli_fields = sorted(set(re.findall(r"^\s{4}([a-z][a-z0-9_]*):", cli_block, re.M)))
    forbidden_cli = sorted(field for field in cli_fields if any(token in field for token in (
        "edit", "apply", "patch", "transaction", "preview", "model", "recipe", "dispatch",
    )))
    forbidden_request_kinds = sorted(set(request_kinds) & {
        "ApplyEdit", "PreviewEdit", "BeginTransaction", "CommitTransaction", "RollbackTransaction",
    })
    forbidden_non_goals = sorted(
        name for name, needle in {
            "JBMC":"jbmc", "BYTEBACK":"byteback", "MODEL_PROVIDER":"openai",
            "ANTHROPIC":"anthropic", "FAMILY_DISPATCH":"benchmark-family",
        }.items() if needle in production.lower()
    )
    worker_source = (ROOT / "workers/kotlin/src/main/kotlin/dev/semanticthread/worker/Worker.kt").read_text(encoding="utf-8")
    worker_tests = (ROOT / "workers/kotlin/src/test/kotlin/dev/semanticthread/worker/ProjectModelCommandTest.kt").read_text(encoding="utf-8")
    index_source = (ROOT / "crates/clew/src/index.rs").read_text(encoding="utf-8")
    cache_source = cache_path.read_text(encoding="utf-8")
    checks = {
        "onlyReadOnlyRequestKinds":request_kinds == ["OpenProject"],
        "onlyReadOnlyWorkerCalls":set(worker_calls) <= {"index_files_verified","inspect_verified_index","request","shutdown"},
        "noMutationCliAuthority":not forbidden_cli,
        "noMutationRequestKind":not forbidden_request_kinds,
        "noModelOrExcludedNonGoalReachability":not forbidden_non_goals,
        "dedicatedRunnerConstructsExactCommand":"exact_command = [" in harness_path.read_text(encoding="utf-8"),
        "adapterOwnsTypedAttemptAndCache":"KotlinAttempt" in adapter and "SemanticCache" in cache_source,
        "futureCompilerValuesCovered":"futureCompilerDescriptorValuesBecomeTypedBoundaries" in worker_tests,
        "malformedRowsCovered":"malformedCompilerFactRowIsRetainedAsBothTypedGraphBoundaries" in worker_tests,
        "utf16ToUtf8Covered":"compilerUtf16OffsetsAreConvertedToUtf8BytesWithoutSplittingEmoji" in worker_tests,
        "effectiveVisibilityLocalIsTyped":'SUPPORTED_EFFECTIVE_VISIBILITIES = setOf(' in worker_source and '"local"' not in worker_source.split('SUPPORTED_EFFECTIVE_VISIBILITIES = setOf(',1)[1].split(')',1)[0] and '"UNKNOWN_EFFECTIVE_VISIBILITY"' in worker_source,
        "quarantinedRowsCannotBecomeProven":"REFERENCE_TO_QUARANTINED_DESCRIPTOR" in worker_source and "REFERENCE_TO_QUARANTINED_DESCRIPTOR" in index_source,
        "cacheCorruptionAndSymlinkCovered":"cache_rejects_corruption_and_symlink" in cache_source,
        "cacheKeyOrderCovered":"cache_key_binds_ordered_manifest" in cache_source,
        "cacheInputDriftCovered":"cache_payload_receipt_must_bind_exact_key_inputs" in cache_source,
        "terminalDigestVolatilePathCovered":"terminal_identity_ignores_absolute_staging_path" in cache_source,
    }
    return {
        "schema":"codeclew.kotlin-k1-source-anchor-packet/0.1",
        "status":"PASS" if all(checks.values()) else "FAIL", "checks":checks,
        "requestKinds":request_kinds,"workerCalls":worker_calls,"cliFields":cli_fields,
        "forbiddenCliFields":forbidden_cli,"forbiddenRequestKinds":forbidden_request_kinds,
        "forbiddenNonGoals":forbidden_non_goals,
        "sources":{"kotlinAdapter":file_digest(adapter_path),"kotlinK1Protocol":file_digest(cache_path),"harness":file_digest(harness_path)},
    }


def build_packet_valid(packet: Any) -> bool:
    required_mutations = {
        "compilerArgumentOrder", "classpathOrder", "jarBytes", "coordinate", "scope",
        "repository", "plugin", "reactor", "generatedConfiguration", "missingReflectiveField",
    }
    sources = {
        "worker":file_digest(ROOT / "workers/kotlin/src/main/kotlin/dev/semanticthread/worker/Worker.kt"),
        "mavenModel":file_digest(ROOT / "workers/kotlin/src/main/kotlin/dev/semanticthread/worker/MavenProjectModel.kt"),
        "gradleModel":file_digest(ROOT / "workers/kotlin/src/main/resources/semantic-thread-model.init.gradle"),
        "adapter":file_digest(ROOT / "crates/evidence-adapters/src/bin/kotlin.rs"),
    }
    return (
        isinstance(packet, Mapping) and packet.get("schema") == "codeclew.kotlin-k1-build-dependency-conformance/0.1"
        and packet.get("status") == "PASS" and all(packet.get("checks", {}).values())
        and set(packet.get("mutations", {})) == required_mutations
        and all(row.get("digestChanged") is True for row in packet["mutations"].values())
        and packet["mutations"]["missingReflectiveField"].get("validShape") is False
        and packet.get("mutationResultsSha256") == digest(canonical(packet["mutations"]))
        and packet.get("sourceAuthorities") == sources
    )


def determinism_packet_valid(packet: Any) -> bool:
    if not isinstance(packet, Mapping) or packet.get("schema") != "codeclew.kotlin-k1-determinism-conformance/0.1":
        return False
    base = {"entities":[{"opaqueId":"b"},{"opaqueId":"a"}],"facts":[{"relation":"calls","owner":"a","target":"b"}],"boundaries":[],"orderedCompilerArguments":["-Xa","-Xb"],"volatile":{"timestamp":1,"path":"/tmp/one"}}
    semantic = lambda value: rust_digest({"entities":sorted(value["entities"],key=lambda row:row["opaqueId"]),"facts":sorted(value["facts"],key=canonical),"boundaries":sorted(value["boundaries"],key=canonical),"orderedCompilerArguments":value["orderedCompilerArguments"]})
    reordered = json.loads(json.dumps(base)); reordered["entities"].reverse()
    volatile = json.loads(json.dumps(base)); volatile["volatile"]={"timestamp":999,"path":"/tmp/two"}
    arguments = json.loads(json.dumps(base)); arguments["orderedCompilerArguments"].reverse()
    expected = {"base":semantic(base),"reordered":semantic(reordered),"volatile":semantic(volatile),"arguments":semantic(arguments)}
    checks = {"trueSetOrderEquivalent":expected["base"]==expected["reordered"],"volatileMetadataExcluded":expected["base"]==expected["volatile"],"orderedArgumentsSignificant":expected["base"]!=expected["arguments"]}
    return packet.get("status") == "PASS" and packet.get("checks") == checks and packet.get("digests") == expected


def load(path: Path, schema: str) -> tuple[dict[str, Any], str]:
    if path.is_symlink() or not path.is_file():
        raise ValueError("audit input must be a regular non-symlink file")
    raw = path.read_bytes()
    value = json.loads(raw)
    if not isinstance(value, dict) or canonical(value) != raw or value.get("schema") != schema or value.get("seriesId") != SERIES_ID:
        raise ValueError("audit input canonical/schema/series mismatch")
    return value, digest(raw)


def telemetry(rows: list[Mapping[str, Any]], thresholds: Mapping[str, Any]) -> bool:
    if len(rows) != 24:
        return False
    for row in rows:
        cost = row.get("adapterCost")
        if not isinstance(cost, Mapping) or set(cost) != COST_KEYS or cost.get("modelCalls") != 0:
            return False
        wall, rss = row.get("externalWallMicros"), row.get("maximumResidentBytes")
        if not isinstance(wall, int) or isinstance(wall, bool) or wall < 0 or wall > thresholds["perInvocationWallSecondsMaximum"] * 1_000_000:
            return False
        if not isinstance(rss, int) or isinstance(rss, bool) or rss < 0 or rss > thresholds["perInvocationMaximumResidentBytes"]:
            return False
        for key, value in cost.items():
            if key in {"maximumResidentBytes", "dependencyPreparationMicros", "dependencyVerificationMicros"}:
                if not ((isinstance(value, int) and not isinstance(value, bool) and value >= 0) or (isinstance(value, str) and value)):
                    return False
            elif not isinstance(value, int) or isinstance(value, bool) or value < 0:
                return False
    return True


def exact_success(row: Mapping[str, Any], entries: Mapping[str, Mapping[str, Any]], analyzers: Mapping[str, Any], tools: Mapping[str, Any]) -> bool:
    entry = entries.get(str(row.get("entry")))
    if not isinstance(entry, Mapping):
        return False
    minor = entry["trustedAnalyzerMinorLine"]
    worker = tools.get("workerManifests", {}).get(minor, {})
    return (
        row.get("status") == "ADAPTER_OUTPUT" and row.get("successAuthorityValidated") is True
        and row.get("nonemptyProjection", {}).get("passed") is True
        and row.get("declaredProjectCompilerVersion") == entry["declaredKotlinVersion"]
        and row.get("analyzerCompilerVersion") == analyzers[minor]["compilerVersion"] == worker.get("compilerVersion")
        and row.get("candidateToolsManifestSha256") == tools["manifestSha256"]
        and row.get("workerDistributionIdentity") == {key:worker.get(key) for key in ("treeHash","buildInputDigest","pluginFingerprint")}
    )


def terminal_protocol(row: Mapping[str, Any]) -> bool:
    if row.get("status") == "ADAPTER_OUTPUT":
        return row.get("successAuthorityValidated") is True and isinstance(row.get("adapterAuthority"), Mapping)
    return row.get("status") in {"PARTIAL","REFUSED","FAILED"} and isinstance(row.get("reasonCode"), str) and not row["reasonCode"].startswith("UNTYPED_FAILURE/") and is_digest(row.get("terminalSemanticDigest")) and row.get("successAuthorityValidated") is False


def build_authority_valid(row: Mapping[str, Any]) -> bool:
    authority = row.get("buildModelAuthority")
    manifest = authority.get("semanticInputManifest") if isinstance(authority, Mapping) else None
    return isinstance(manifest, Mapping) and authority.get("semanticInputManifestHash") == rust_digest(manifest) and all(field in manifest for field in (
        "orderedCompileClasspath","orderedFriendPaths","orderedCompilerPlugins","orderedFreeCompilerArguments",
        "orderedOptIns","orderedCompilerPluginOptions","dependencyCoordinates","repositories","reactorPoms",
        "buildPlugins","generatedSourceConfiguration","fieldBoundaries","buildModelBoundaries","buildRoot",
        "projectDirectory","module","sourceSet","platform","jdkHomeFingerprint","buildState","modelInputs",
    )) and all(is_digest(authority.get(field)) for field in (
        "dependencyGraphDigest","buildModelDigest","buildConfigurationDigest","generatedSourcesManifestDigest","boundariesDigest",
    ))


def offline_prepare_row(row: Mapping[str, Any]) -> bool:
    commands = row.get("prepareArgv")
    profiles = row.get("sandboxProfiles")
    environments = row.get("prepareEnvironments")
    sentinel = row.get("offlineNetworkSentinel")
    if (
        not isinstance(commands, list) or len(commands) != 2
        or not all(isinstance(command, list) for command in commands)
        or row.get("prepareArgvSha256") != digest(canonical(commands))
        or not isinstance(profiles, Mapping) or set(profiles) != {"online", "offline"}
        or not isinstance(sentinel, Mapping)
    ):
        return False
    online, offline = commands
    flag = "-o" if row.get("buildDsl") == "MAVEN" else "--offline"
    online_profile, offline_profile = profiles.get("online"), profiles.get("offline")
    if not isinstance(online_profile, Mapping) or not isinstance(offline_profile, Mapping):
        return False
    online_raw = online_profile.get("profileBytes")
    offline_raw = offline_profile.get("profileBytes")
    if not isinstance(online_raw, str) or not isinstance(offline_raw, str):
        return False
    online_write_root = sandbox_profile_write_root(online_raw)
    offline_write_root = sandbox_profile_write_root(offline_raw)
    profiles_valid = (
        online_profile == {
            "policy": PREPARE_ONLINE_NETWORK_POLICY,
            "profileSha256": digest(online_raw.encode()),
            "profileBytes": online_raw,
        }
        and offline_profile == {
            "policy": PREPARE_OFFLINE_NETWORK_POLICY,
            "profileSha256": digest(offline_raw.encode()),
            "profileBytes": offline_raw,
        }
        and [line for line in online_raw.splitlines() if line in {"(allow network*)", "(deny network*)"}] == ["(allow network*)"]
        and [line for line in offline_raw.splitlines() if line in {"(allow network*)", "(deny network*)"}] == ["(deny network*)"]
        and online_raw != offline_raw
        and online_write_root is not None
        and online_write_root == offline_write_root
        and sandbox_profile_shape_valid(
            online_raw, "(allow network*)", row, "online", online,
        )
        and sandbox_profile_shape_valid(
            offline_raw, "(deny network*)", row, "offline", offline,
        )
    )
    environments_valid = (
        online_write_root is not None
        and preparation_environments_valid(
            environments, online_write_root, row.get("buildDsl"),
        )
    )
    sentinel_base = (
        sentinel.get("argv") == PREPARE_NETWORK_SENTINEL_ARGV
        and sentinel.get("argvSha256") == digest(canonical(PREPARE_NETWORK_SENTINEL_ARGV))
        and sentinel.get("denialErrnos") == ["EACCES", "EPERM"]
        and is_digest(sentinel.get("stdoutSha256"))
        and is_digest(sentinel.get("stderrSha256"))
    )
    denied = (
        sentinel.get("executed") is True and sentinel.get("exitCode") == 0
        and sentinel.get("stdoutSha256") == digest(b"")
        and sentinel.get("stderrSha256") == digest(b"")
    )
    online_refusal = (
        row.get("outcome") == "TYPED_REFUSAL"
        and row.get("failureStage") == "ONLINE_DEPENDENCY_PREPARATION"
        and sentinel.get("executed") is False and sentinel.get("exitCode") is None
    )
    marker = row.get("offlineNoDownloadMarker") == {
        "flag": flag, "commandIndex": 1,
        "presentExactlyOnce": offline.count(flag) == 1,
        "offlineCommandSha256": digest(canonical(offline)),
    }
    return (
        profiles_valid and environments_valid and sentinel_base and marker
        and flag not in online and flag in offline and online != offline
        and (denied or online_refusal)
    )


def candidate_prepare_tools_valid(tools: Mapping[str, Any]) -> bool:
    perl = Path("/usr/bin/perl")
    builder = ROOT / "scripts/build-trusted-worker-distributions.py"
    return (
        not perl.is_symlink() and perl.is_file()
        and tools.get("systemTools", {}).get("perl") == {
            "path": str(perl), "sha256": file_digest(perl),
        }
        and file_digest(builder) == TRUSTED_WORKER_BUILDER_SHA256
        and tools.get("sourceAuthorities", {}).get("trustedWorkerDistributionBuilder")
            == TRUSTED_WORKER_BUILDER_SHA256
        and all(
            tools.get("workerManifests", {}).get(minor, {}).get("buildInputDigest") == expected
            for minor, expected in TRUSTED_WORKER_BUILD_INPUTS.items()
        )
    )


def row_seal_valid(row: Any) -> bool:
    if not isinstance(row, Mapping) or set(row) != {"predicate","measured","missingEvidence","failureClass","evidence","status","evidenceSha256"}:
        return False
    if not isinstance(row.get("measured"), bool) or not isinstance(row.get("missingEvidence"), list):
        return False
    passed = row["measured"] and not row["missingEvidence"]
    if row.get("status") != ("PASS" if passed else "FAIL") or (row.get("failureClass") is None) != passed:
        return False
    if not passed and row.get("failureClass") not in {"STOP","GAP"}:
        return False
    body = {key:row[key] for key in ("predicate","measured","missingEvidence","failureClass","evidence")}
    return row.get("evidenceSha256") == digest(canonical(body))


def expected_requirement_statuses(
    rows: list[Mapping[str, Any]], qualification: Mapping[str, Any], holdout: Mapping[str, Any],
    requirements: Mapping[str, Any], corpus: Mapping[str, Any], tools: Mapping[str, Any],
    safety: Mapping[str, Any], cache: Mapping[str, Any], conformance: Mapping[str, Any],
    baseline: Mapping[str, Any], harness: Mapping[str, Any], freeze: Mapping[str, Any],
) -> tuple[dict[str, str], dict[str, str]]:
    entries = {entry["id"]:entry for entry in corpus["entries"]}
    analyzers = corpus["frozenExecutionPolicy"]["trustedAnalyzers"]
    successes = [row for row in rows if row.get("status") == "ADAPTER_OUTPUT"]
    pairs = {(row.get("entry"),row.get("invocation")) for row in rows}
    expected_pairs = {(entry, invocation) for entry in EXPECTED_Q + EXPECTED_H for invocation in ("COLD","WARM")}
    source_packet = harness.get("sourceAnchorPacket")
    source_checks = source_packet.get("checks", {}) if isinstance(source_packet, Mapping) else {}
    static_current = source_packet == source_anchor_packet() and source_packet.get("status") == "PASS" if isinstance(source_packet, Mapping) else False
    build_current = build_packet_valid(harness.get("buildDependencyConformance"))
    deterministic = determinism_packet_valid(harness.get("determinismConformance"))
    raw = conformance.get("rawEvidence", {})
    prep = raw.get("qualificationPreparationAttempts", []) + raw.get("holdoutPreparationAttempts", []) if isinstance(raw, Mapping) else []
    prep_by_entry = {row.get("entry"):row for row in prep if isinstance(row, Mapping)}
    closure_reasons = {"DEPENDENCY_CLOSURE_UNAVAILABLE","OFFLINE_MODEL_PROBE_FAILED","UNSUPPORTED_BUILD_CONFIGURATION"}
    closure_sound = len(prep) == 12 and all(row.get("outcome") == "READY" or row.get("outcome") == "TYPED_REFUSAL" and row.get("reasonCode") in closure_reasons for row in prep)
    terminal = pairs == expected_pairs and len(rows) == 24 and all(terminal_protocol(row) for row in rows)
    source_equal = all(row.get("sourceMutation") is False and isinstance(row.get("repositoryBefore"), Mapping) and row.get("repositoryBefore") == row.get("repositoryAfter") and row["repositoryBefore"].get("head") == entries.get(row.get("entry"),{}).get("commit") and row["repositoryBefore"].get("tree") == entries.get(row.get("entry"),{}).get("gitTree") for row in rows)
    prepare_tools_current = candidate_prepare_tools_valid(tools)
    identities_safe = prepare_tools_current and all(exact_success(row, entries, analyzers, tools) for row in successes) and all(row.get("candidateToolsManifestSha256") == tools["manifestSha256"] for row in rows)
    lines = sorted({entries[row["entry"]]["trustedAnalyzerMinorLine"] for row in successes if exact_success(row, entries, analyzers, tools)})
    dsl_counts = {dsl:sum(entry["buildDsl"] == dsl for entry in corpus["entries"]) for dsl in ("GRADLE_KOTLIN_DSL","GRADLE_GROOVY_DSL","MAVEN")}
    preparation_parity = set(prep_by_entry) == set(entries) and all(all(prep_by_entry[key].get(field) == entry[field] for field in ("commit","gitTree","selectedCompilation","buildDsl")) and prep_by_entry[key].get("prepareArgvSha256") == digest(canonical(prep_by_entry[key].get("prepareArgv"))) for key,entry in entries.items())
    organizations = Counter(entry.get("organization") for entry in corpus["entries"])
    r02 = [entry.get("id") for entry in corpus["entries"]] == EXPECTED_Q + EXPECTED_H and len(entries)==12 and len(organizations)>=6 and max(organizations.values(),default=99)<=2 and all(isinstance(entry.get("commit"),str) and len(entry["commit"]) in {40,64} and all(character in "0123456789abcdef" for character in entry["commit"]) and isinstance(entry.get("gitTree"),str) and len(entry["gitTree"]) in {40,64} and all(character in "0123456789abcdef" for character in entry["gitTree"]) for entry in corpus["entries"])
    r03 = all(value >= 3 for value in dsl_counts.values()) and all(entry.get("selectedCompilation") for entry in corpus["entries"]) and preparation_parity
    workload = requirements["workloadPolicy"]["query"]
    projection = all(row.get("nonemptyProjection",{}).get("passed") is True and row.get("workload",{}).get("selectionAuthority") == "HARNESS_DERIVED_ONLY" and isinstance(row.get("workload",{}).get("seedEntity"),str) and row["workload"].get("maxDepth") == workload["maxDepth"] and row["workload"].get("maxEntities") == workload["maxEntities"] for row in successes)
    cache_protocol = pairs == expected_pairs and all((cold.get("status") != "ADAPTER_OUTPUT" and warm.get("status") == cold.get("status") and warm.get("terminalSemanticDigest") == cold.get("terminalSemanticDigest") and warm.get("cacheHit") is False) or (cold.get("status") == warm.get("status") == "ADAPTER_OUTPUT" and cold.get("terminalSemanticDigest") == warm.get("terminalSemanticDigest") and cold.get("semanticFactsDigest") == warm.get("semanticFactsDigest") and warm.get("cacheHit") is True and is_digest(cold.get("semanticCacheKeyDigest")) and cold.get("semanticCacheKeyDigest") == warm.get("semanticCacheKeyDigest")) for entry in EXPECTED_Q+EXPECTED_H for cold,warm in [[next(row for row in rows if row.get("entry")==entry and row.get("invocation")=="COLD"),next(row for row in rows if row.get("entry")==entry and row.get("invocation")=="WARM")]])
    sandbox_rows = all(row.get("sourceExecutionAuthority",{}).get("kind") == "SANITIZED_DISPOSABLE_GIT" and row.get("sourceExecutionAuthority",{}).get("selectedSourceTreeSha256") == row.get("sourceExecutionAuthority",{}).get("executionSourceTreeSha256") and row.get("sourceExecutionAuthority",{}).get("discardedBeforePublication") is True and row.get("selectedInputs",{}).get("sandboxDefaultPolicy") == "DENY_DEFAULT_NETWORK_DENY" and row.get("selectedInputs",{}).get("productionCredentialInheritance") is False and all(is_digest(row.get("selectedInputs",{}).get(key)) for key in ("environmentPolicySha256","networkSandboxProfileSha256","sandboxAuthorizedReadPathsSha256","sandboxAuthorizedWritePathsSha256","sandboxExecutableSha256")) for row in rows)
    supervisor = harness.get("supervisor",{}).get("cases",{})
    supervisor_required = {"empty","nonzero","build_failure","invalid_json","truncated_json","oom_like_signal","direct_adapter_output","background_child","typed_nonzero","timeout","limit","sandbox_network_env","sandbox_secret_paths","sandbox_unix_network","sandbox_source_write","sandbox_keychain_read","sandbox_background_child"}
    supervisor_ok = (
        isinstance(supervisor,Mapping) and supervisor_required <= set(supervisor)
        and all(supervisor.get(name) == value for name,value in SECURITY_SUPERVISOR_EXPECTED.items())
    )
    authority_required = {
        "alternateGraphRejected", "alternateThresholdRejected", "alternateCorpusRejected",
        "staleInputRejected", "directNodeForgeryRejected", "earlyHoldoutRejected",
        "callerAttemptForgeryRejected", "conditionalRootForgeryRejected", "cancelledOrderingRejected",
        "trackedLinkEscapeRejected", "dirtySourceSetRejected", "prepareSupervisorNonzeroRetained",
        "prepareMavenLauncherTraversalPassed", "prepareSourceAncestryTraversalPassed",
        "prepareAncestorSecretReadDenied", "prepareAncestorWriteDenied",
        "prepareSelectedSourceWriteDenied", "prepareKeychainReadDenied",
        "prepareTraversalNetworkSemanticsPreserved", "prepareAncestorDataOnlyMutationRejected",
        "prepareBroadSandboxPermissionRejected", "prepareRootAuthoritySubstitutionsRejected",
        "prepareSplitPhaseRootsRejected", "requirementR18SupervisorNotRunRejected",
        "requirementR18PrepareNotRunRejected", "prepareGradleWrapperBootstrapHomePassed",
        "prepareMissingGradleWrapperBootstrapHomeRejected",
        "prepareGradleJvmTmpdirAuthorityPassed",
        "prepareMissingGradleJvmTmpdirRejected",
        "prepareWrongGradleJvmTmpdirRejected",
        "prepareGradleJvmTmpdirFailureClassifiedInfrastructure",
        "prepareGradleStrictOfflineFailureTypedRefusal",
        "prepareGradleStrictOfflineWrongProfileSecurityRejected",
        "prepareGradleOnlineSecurityFailureRejected",
        "prepareMavenOfflineSecurityFailureRejected",
        "prepareMavenOfflineModelGoalsPrefetchedOnline",
        "preparePostPublicationEvidenceRevalidated",
    }
    authority_cases = harness.get("requirementCases",{})
    publication_packet = harness.get("dependencyPublicationSelfTest")
    publication_ok = publication_packet == {
        "schema": "codeclew.kotlin-k1-dependency-publication-self-test/0.1",
        "status": "PASS", "nestedMoveBeforeRootSeal": True,
        "cohortMoveBeforeRootSeal": True, "postRenameFailureRemoved": True,
    }
    authority_ok = publication_ok and isinstance(authority_cases,Mapping) and authority_required <= set(authority_cases) and all(authority_cases[key] is True for key in authority_required)
    baseline_commands = baseline.get("commands",[])
    focused = {tuple(row.get("argv",[])):row for row in baseline_commands if isinstance(row,Mapping)}
    required_focused = {
        ("cargo","test","--offline","--locked","-p","evidence-core","--all-targets","--","--test-threads=1"),
        ("cargo","test","--offline","--locked","-p","evidence-adapters","--all-targets","--","--test-threads=1"),
        ("cargo","test","--offline","--locked","-p","clew","--lib","worker::tests::compiler_receipt_requires_explicit_successful_k2_validation","--","--exact","--test-threads=1"),
        ("cargo","test","--offline","--locked","-p","clew","--lib","worker::tests::trusted_distribution_identity_is_read_only_cache_key_material","--","--exact","--test-threads=1"),
        ("cargo","test","--offline","--locked","-p","clew","--lib","index::tests::declaration_descriptor_ingestion_roundtrips_unknown_and_commits_snapshot","--","--exact","--test-threads=1"),
        ("cargo","test","--offline","--locked","-p","clew","--lib","index::tests::declaration_descriptor_ingestion_rejects_malformed_hash_and_provenance","--","--exact","--test-threads=1"),
        ("cargo","test","--offline","--locked","-p","clew","--lib","index::tests::declaration_relation_ingestion_roundtrips_typed_unknown_and_commits_snapshot","--","--exact","--test-threads=1"),
        ("cargo","test","--offline","--locked","-p","clew","--lib","index::tests::declaration_relation_ingestion_rejects_hash_malformed_and_snapshot_mismatch","--","--exact","--test-threads=1"),
        ("./gradlew","--offline",":workers:kotlin:test","--tests","dev.semanticthread.worker.ProjectModelCommandTest.futureCompilerDescriptorValuesBecomeTypedBoundaries","--tests","dev.semanticthread.worker.ProjectModelCommandTest.malformedCompilerFactRowIsRetainedAsBothTypedGraphBoundaries",":workers:kotlin21:compileKotlin",":workers:kotlin23:compileKotlin","--no-daemon"),
        ("cargo","fmt","--all","--check"),
    }
    expected_historical = {
        ("cargo","clippy","--offline","--locked","-p","clew","--lib","--","-D","warnings"),
        ("cargo","clippy","--offline","--locked","-p","semantic-corpus","--lib","--","-D","warnings"),
    }
    historical_rows = [
        row for row in baseline_commands
        if isinstance(row,Mapping) and row.get("policy") == "HISTORICAL_BASELINE"
    ] if isinstance(baseline_commands,list) else []
    historical_projection = [
        {"argvSha256":row.get("argvSha256"),"observed":row.get("observed"),"stderrSha256":row.get("stderrSha256")}
        for row in historical_rows
    ]
    historical_visible = (
        {tuple(row.get("argv",[])) for row in historical_rows} == expected_historical
        and all(baseline_command_valid(row,tuple(row["argv"])) for row in historical_rows)
        and baseline.get("historicalBaselineOutcomes") == historical_projection
        and baseline.get("historicalClaims") == {
            "clewClippyDiagnosticsAtM1":12,
            "semanticCorpusClippyDiagnosticsAtM1":4,
            "sourceReportSha256":file_digest(ROOT / "docs/experiments/codeclew-multilanguage-m1-implementation-report-2026-08-13.md"),
        }
    )
    focused_valid = all(
        (row := focused.get(command)) is not None
        and row.get("policy") == "REQUIRED_GREEN" and row.get("exitCode") == 0
        and baseline_command_valid(row,command)
        for command in required_focused
    )
    packet_cargo = baseline.get("cargoExecutionAuthority")
    context_id = baseline.get("executionContextId")
    postcheck = baseline.get("executionContextPostcheck")
    cargo_rows = [row for row in baseline_commands if isinstance(row,Mapping) and isinstance(row.get("argv"),list) and row["argv"] and row["argv"][0] == "cargo"]
    gradle_rows = [row for row in baseline_commands if isinstance(row,Mapping) and isinstance(row.get("argv"),list) and row["argv"] and row["argv"][0] == "./gradlew"]
    recomputed_context_id = digest(canonical({
        "schema":"codeclew.kotlin-k1-baseline-execution-context/0.1",
        "cargoLauncher":packet_cargo.get("launcher") if isinstance(packet_cargo,Mapping) else None,
        "cargoSeed":packet_cargo.get("dependencySeed") if isinstance(packet_cargo,Mapping) else None,
        "gradleLauncher":gradle_rows[0].get("gradleExecutionAuthority",{}).get("launcher") if len(gradle_rows)==1 else None,
        "gradleSeed":gradle_rows[0].get("gradleExecutionAuthority",{}).get("dependencySeed") if len(gradle_rows)==1 else None,
    }))
    baseline_authority_complete = (
        is_digest(context_id)
        and context_id == recomputed_context_id
        and {row.get("executionContextId") for row in baseline_commands if isinstance(row,Mapping)} == {context_id}
        and isinstance(packet_cargo,Mapping) and set(packet_cargo) == {
            "executionContextId","launcher","dependencySeed","isolatedCargoHome",
            "isolatedCargoTargetDir","sharedBaselineExecutionContext","executionCwd",
        }
        and packet_cargo.get("executionContextId") == context_id
        and packet_cargo.get("executionCwd") == "/"
        and packet_cargo.get("isolatedCargoHome") is packet_cargo.get("isolatedCargoTargetDir") is packet_cargo.get("sharedBaselineExecutionContext") is True
        and packet_cargo.get("launcher") == cargo_launcher_expected()
        and all(row.get("cargoExecutionAuthority",{}).get("launcher") == packet_cargo.get("launcher")
                and row.get("cargoExecutionAuthority",{}).get("dependencySeed") == packet_cargo.get("dependencySeed") for row in cargo_rows)
        and isinstance(postcheck,Mapping) and postcheck == {
            "schema":"codeclew.kotlin-k1-baseline-context-postcheck/0.1",
            "executionContextId":context_id,"cargoSeedMembersUnchanged":True,
            "hostSeedMembersUnchanged":True,"cargoLauncherUnchanged":True,
            "gradleLauncherUnchanged":True,"allowedGeneratedStateOnly":True,
            "generatedSourceTreeDigest":packet_cargo.get("dependencySeed",{}).get("generatedSourceTreeDigest"),
            "generatedSourceFileCount":packet_cargo.get("dependencySeed",{}).get("generatedSourceFileCount"),
            "cargoConfigAndCredentialsAbsentAfterCommands":True,
        }
        and baseline.get("candidateToolsManifestSha256") == tools.get("manifestSha256")
        and candidate_cargo_baseline_valid(tools)
        and baseline.get("repositoryHeadBefore") == baseline.get("repositoryBaseRevision")
        and baseline.get("repositoryHeadAfter") == baseline.get("repositoryBaseRevision")
    )
    baseline_green = (
        isinstance(baseline_commands,list)
        and len(baseline_commands) == len(required_focused) + len(expected_historical)
        and set(focused) == required_focused | expected_historical
        and baseline.get("repositoryBaseRevision") == "be19fd06ce7509ffbaf4622b8f8d13ec7cbcd854"
        and baseline.get("requiredGreen") is True
        and focused_valid and historical_visible and baseline_authority_complete
    )
    materialize = raw.get("holdoutMaterialization",{}) if isinstance(raw,Mapping) else {}
    holdout_guard = freeze.get("postFreezeChangesAllowed") is False and materialize.get("semanticInspectionPerformed") is False and all(is_digest(row.get("phaseReceipts",{}).get("CANDIDATE_FREEZE_VERIFY")) for row in holdout.get("attempts",[]))
    totality = static_current and all(source_checks.get(key) is True for key in ("futureCompilerValuesCovered","malformedRowsCovered","utf16ToUtf8Covered","effectiveVisibilityLocalIsTyped","quarantinedRowsCannotBecomeProven"))
    cache_mutations = static_current and all(source_checks.get(key) is True for key in ("cacheCorruptionAndSymlinkCovered","cacheKeyOrderCovered","cacheInputDriftCovered"))
    proof = safety.get("structuralConformance",{})
    models_zero = all(row.get("adapterCost",{}).get("modelCalls") == 0 for row in rows) and qualification.get("modelCalls") == holdout.get("modelCalls") == baseline.get("modelCalls") == harness.get("modelCalls") == 0
    measured = {
        "K1-R01":static_current and source_equal,"K1-R02":r02,"K1-R03":r03,
        "K1-R04":identities_safe and lines == ["2.1","2.3","2.4"],
        "K1-R05":build_current and terminal and all(build_authority_valid(row) for row in successes) and closure_sound,
        "K1-R06":build_current and terminal and all(build_authority_valid(row) for row in successes) and closure_sound,
        "K1-R07":source_equal and safety.get("sourceMutations")==0,
        "K1-R08":totality and proof.get("status")=="PASS",
        "K1-R09":terminal and supervisor_ok,
        "K1-R10":safety.get("falseProven")==0 and safety.get("falseComplete")==0 and proof.get("status")=="PASS",
        "K1-R11":projection,"K1-R12":cache_protocol and cache_mutations,
        "K1-R13":len(prep)==12 and prepare_tools_current and all(offline_prepare_row(row) for row in prep) and safety.get("offlineReplayEqual") is True and sandbox_rows and closure_sound,
        "K1-R14":telemetry(rows,requirements["decisionThresholds"]),"K1-R15":holdout_guard,
        "K1-R16":authority_ok,"K1-R17":deterministic and source_checks.get("terminalDigestVolatilePathCovered") is True and safety.get("offlineReplayEqual") is True,
        "K1-R18":publication_ok and prepare_tools_current and sandbox_rows and supervisor_ok and security_tripwire_cases_valid(supervisor,authority_cases),"K1-R19":isinstance(raw.get("k0ByteExact",{}).get("byteExact"),Mapping) and baseline_green,
        "K1-R20":static_current and models_zero,
    }
    classes = {identifier:"STOP" for identifier in measured}
    if not measured["K1-R04"] and identities_safe:
        classes["K1-R04"]="GAP"
    return ({identifier:("PASS" if value else "FAIL") for identifier,value in measured.items()}, {identifier:("NONE" if value else classes[identifier]) for identifier,value in measured.items()})


def main() -> None:
    parser = argparse.ArgumentParser()
    for argument in ("matrix-safety", "applicability", "cache-cost", "requirement-conformance", "candidate-freeze", "qualification-matrix", "holdout-matrix", "requirements", "corpus", "candidate-tools", "baseline-packet", "harness-self-test-packet"):
        parser.add_argument("--" + argument, required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    safety, safety_sha = load(args.matrix_safety, "codeclew.kotlin-k1-matrix-safety/0.1")
    applicability, applicability_sha = load(args.applicability, "codeclew.kotlin-k1-applicability/0.1")
    cache, cache_sha = load(args.cache_cost, "codeclew.kotlin-k1-cache-cost/0.1")
    conformance, conformance_sha = load(args.requirement_conformance, "codeclew.kotlin-k1-requirement-conformance/0.1")
    freeze, freeze_sha = load(args.candidate_freeze, "codeclew.kotlin-k1-candidate-freeze/0.1")
    qualification, qualification_sha = load(args.qualification_matrix, "codeclew.kotlin-k1-matrix/0.1")
    holdout, holdout_sha = load(args.holdout_matrix, "codeclew.kotlin-k1-matrix/0.1")
    requirements, requirements_sha = load(args.requirements, "codeclew.kotlin-real-repository-requirements/0.1")
    corpus, corpus_sha = load(args.corpus, "codeclew.kotlin-real-repository-corpus/0.1")
    candidate_tools, candidate_tools_sha = load(args.candidate_tools, "codeclew.kotlin-k1-candidate-tools/0.1")
    baseline, baseline_sha = load(args.baseline_packet, "codeclew.kotlin-k1-baseline-packet/0.2")
    harness_packet, harness_sha = load(args.harness_self_test_packet, "codeclew.kotlin-k1-harness-self-test-packet/0.1")
    candidate_tools["manifestSha256"] = candidate_tools_sha
    rows = qualification.get("attempts", []) + holdout.get("attempts", [])
    expected_pairs = {(entry, invocation) for entry in EXPECTED_Q + EXPECTED_H for invocation in ("COLD", "WARM")}
    actual_pairs = {(row.get("entry"), row.get("invocation")) for row in rows if isinstance(row, Mapping)}
    requirement_rows = conformance.get("requirements", {})
    expected_ids = [f"K1-R{number:02d}" for number in range(1, 21)]
    exact_requirement_keys = isinstance(requirement_rows, Mapping) and list(requirement_rows) == expected_ids
    independent_status, independent_classes = expected_requirement_statuses(
        rows, qualification, holdout, requirements, corpus, candidate_tools,
        safety, cache, conformance, baseline, harness_packet, freeze,
    )
    row_seals = exact_requirement_keys and all(row_seal_valid(requirement_rows[key]) for key in expected_ids)
    conformance_matches = row_seals and all(
        requirement_rows[key].get("status") == independent_status[key]
        and (
            requirement_rows[key].get("failureClass") is None
            if independent_classes[key] == "NONE"
            else requirement_rows[key].get("failureClass") == independent_classes[key]
        ) for key in expected_ids
    )
    recomputed_stop = sorted(key for key in expected_ids if independent_classes[key] == "STOP")
    recomputed_gaps = sorted(key for key in expected_ids if independent_classes[key] == "GAP")
    row_mutations = conformance.get("rowMutationConformance", {})
    mutation_cases = row_mutations.get("cases", {}) if isinstance(row_mutations, Mapping) else {}
    row_mutations_valid = (
        row_mutations.get("schema") == "codeclew.kotlin-k1-requirement-row-mutation-conformance/0.1"
        and row_mutations.get("status") == "PASS" and set(mutation_cases) == set(expected_ids)
        and all(value is True for value in mutation_cases.values())
        and row_mutations.get("resultsSha256") == digest(canonical(mutation_cases))
    )
    conformance_summary = (
        conformance.get("allPassed") == all(value == "PASS" for value in independent_status.values())
        and conformance.get("stopViolations") == recomputed_stop
        and conformance.get("gapRequirements") == recomputed_gaps
        and row_mutations_valid
    )
    source_mutations = sum(row.get("sourceMutation") is True for row in rows if isinstance(row, Mapping))
    false_proven = sum(len(row.get("proofSafety", {}).get("falseProven", [])) for row in rows if isinstance(row, Mapping))
    false_complete = sum(len(row.get("proofSafety", {}).get("falseComplete", [])) for row in rows if isinstance(row, Mapping))
    replay = all(
        next((row.get("terminalSemanticDigest") for row in rows if row.get("entry") == entry and row.get("invocation") == "COLD"), None)
        == next((row.get("terminalSemanticDigest") for row in rows if row.get("entry") == entry and row.get("invocation") == "WARM"), None)
        and next((row.get("terminalSemanticDigest") for row in rows if row.get("entry") == entry and row.get("invocation") == "COLD"), None) is not None
        for entry in EXPECTED_Q + EXPECTED_H
    )
    untyped = sum(str(row.get("reasonCode", "")).startswith("UNTYPED_FAILURE/") for row in rows if isinstance(row, Mapping))
    recomputed_safe = actual_pairs == expected_pairs and source_mutations == false_proven == false_complete == untyped == 0 and replay
    expected_decision = "STOP" if not recomputed_safe or recomputed_stop else ("GO" if applicability.get("passed") is True and cache.get("passed") is True and all(value == "PASS" for value in independent_status.values()) else "PIVOT")
    frozen_snapshots = freeze.get("snapshots", {})
    checks = {
        "primaryMatricesExact": actual_pairs == expected_pairs and qualification.get("cohort") == "QUALIFICATION" and holdout.get("cohort") == "BLIND_HOLDOUT",
        "safetyRecomputed": safety.get("sourceMutations") == source_mutations and safety.get("untypedFailures") == untyped and safety.get("falseProven") == false_proven and safety.get("falseComplete") == false_complete and safety.get("offlineReplayEqual") == replay and safety.get("safe") == recomputed_safe,
        "requirementsIndependentlyRecomputed": conformance_matches and conformance_summary,
        "candidateImmutable": freeze.get("postFreezeChangesAllowed") is False and frozen_snapshots.get("candidateTools", {}).get("sha256") == candidate_tools_sha and frozen_snapshots.get("independentAuditorSource", {}).get("sha256") == digest(Path(__file__).read_bytes()),
        "producerPacketsBound": baseline.get("modelCalls") == 0 and harness_packet.get("modelCalls") == 0 and isinstance(conformance.get("producerReceiptDigests"), Mapping) and set(conformance["producerReceiptDigests"]) == {"matrixSafety","applicability","cacheCost","baseline","harnessSelfTest","qualificationPrepare","holdoutPrepare","holdoutMaterialize","k0ByteExact"} and all(is_digest(value) for value in conformance["producerReceiptDigests"].values()),
        "modelCalls": all(row.get("adapterCost", {}).get("modelCalls") == 0 for row in rows) and qualification.get("modelCalls") == holdout.get("modelCalls") == candidate_tools.get("modelCalls") == 0,
    }
    report = {
        "schema": "codeclew.kotlin-k1-independent-audit/0.2", "seriesId": SERIES_ID,
        "auditorSourceSha256": digest(Path(__file__).read_bytes()), "checks": checks,
        "matrixSafetySha256": safety_sha, "applicabilitySha256": applicability_sha,
        "cacheCostSha256": cache_sha, "requirementConformanceSha256": conformance_sha,
        "candidateFreezeSha256": freeze_sha, "qualificationMatrixSha256": qualification_sha,
        "holdoutMatrixSha256": holdout_sha, "requirementsSha256": requirements_sha,
        "corpusSha256": corpus_sha, "candidateToolsSha256": candidate_tools_sha,
        "baselinePacketSha256": baseline_sha, "harnessSelfTestPacketSha256": harness_sha,
        "recomputedRequirementStatus": independent_status, "expectedDecision": expected_decision,
        "decision": "ACCEPT" if all(checks.values()) else "REJECT", "modelCalls": 0,
    }
    if args.output.exists() or args.output.is_symlink():
        raise ValueError("audit output is create-only")
    args.output.write_bytes(canonical(report))


if __name__ == "__main__":
    main()
