"""Acquire bounded OpenAPI candidates through the installed retained evidence paths."""

from pathlib import Path, PurePosixPath
import os
import re
import selectors
import subprocess
import sys
import time


DISCOVERY_ROOTS = ("api", "contracts", "docs", "openapi", "spec", "specs", "swagger", "src/main/resources")
DIRECT_NAMES = ("api.json", "api-docs.json", "openapi.json", "swagger.json")
MAX_TREE_BYTES = 512 * 1024


def resource_candidates(rows, limit):
    result = []
    for row in rows:
        record = row.get("record") or {}
        source = record.get("source") or {}
        if (row.get("kind") == "SOURCE_NAVIGATION" and isinstance(source.get("file"), str)
                and source["file"].lower().endswith(".json") and isinstance(row.get("id"), str)
                and isinstance(record.get("fullRecordReference"), str)):
            result.append({"record": {"sourceIds": [row["id"]]}, "sourceReferences": [record["fullRecordReference"]]})
            if len(result) == limit:
                break
    return result


def git_candidates(repository, revision, paths, exclude, *, limit, max_size, deadline):
    """List regular blobs in bounded repository-relative locations from the pinned revision."""
    if not repository or not re.fullmatch(r'[0-9a-f]{40}|[0-9a-f]{64}', revision) or deadline - time.monotonic() <= 0:
        return []
    prefixes = tuple(dict.fromkeys(part for file in sorted(paths)
                                   if len(PurePosixPath(file).parts) > 1
                                   and (part := PurePosixPath(file).parts[0]) not in {"src", ".", "..", "/"}
                                   and part.isascii() and part.replace("_", "").replace("-", "").isalnum()))[:4]
    locations = [*DIRECT_NAMES, *DISCOVERY_ROOTS]
    locations.extend(f"{prefix}/{location}" for prefix in prefixes for location in (*DIRECT_NAMES, *DISCOVERY_ROOTS))
    env = {name: value for name, value in os.environ.items() if not name.startswith("GIT_")}
    env.update(GIT_NO_LAZY_FETCH="1", GIT_TERMINAL_PROMPT="0", GIT_ALLOW_PROTOCOL="")
    args = ["git", "--no-replace-objects", "--literal-pathspecs", "-C", repository, "ls-tree",
            "-r", "-l", "-z", "--full-tree", revision, "--", *locations]
    data = bytearray()
    try:
        with subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                              env=env) as process:
            try:
                with selectors.DefaultSelector() as ready:
                    ready.register(process.stdout, selectors.EVENT_READ)
                    deadline = min(deadline, time.monotonic() + 3)
                    while events := ready.select(max(0, deadline - time.monotonic())):
                        chunk = os.read(events[0][0].fd, 32 * 1024)
                        if not chunk:
                            break
                        data.extend(chunk)
                        if len(data) > MAX_TREE_BYTES or deadline <= time.monotonic():
                            return []
                    else:
                        return []
                if process.wait(timeout=max(.05, deadline - time.monotonic())):
                    return []
            finally:
                if process.poll() is None:
                    process.kill()
    except (OSError, subprocess.SubprocessError):
        return []
    candidates = []
    for line in bytes(data).rstrip(b"\0").split(b"\0"):
        header, _, name = line.partition(b"\t")
        parts = header.split()
        if len(parts) != 4 or parts[0] not in {b"100644", b"100755"} or parts[1] != b"blob":
            continue
        try:
            file, size = name.decode(), int(parts[3])
        except (UnicodeError, ValueError):
            continue
        stem = PurePosixPath(file).stem.lower()
        if (file in exclude or not file.lower().endswith(".json") or not 0 < size <= max_size or len(file) > 1024
                or any(ord(char) < 32 or char in "*?[]:\\" for char in file)):
            continue
        candidates.append((not any(name in stem for name in ("openapi", "swagger", "api-docs", "spec")), file))
    return [file for _, file in sorted(candidates)[:limit]]


def packet_sources(entries, packet, manifest):
    texts = {row["source"]["id"]: packet["texts"][row["textKey"]] for row in packet["outcomes"]}
    return {handle: {**row["source"], "text": texts[row["source"]["id"]], "manifest": manifest}
            for handle, row in entries.items()}


def pinned_sources(job, service, revision, requirements, repository, sources, roots, projector, home, deadline):
    symbols = [row["entrySymbol"] for row in requirements if row.get("entrySymbol") and row.get("entryKind") == "HTTP_ENDPOINT"]
    if not symbols or not repository or time.monotonic() + 2 >= deadline:
        return {}
    files = git_candidates(repository, revision, roots, {row["file"] for row in sources.values()},
                           limit=projector["MAX_FILES"], max_size=projector["MAX_BYTES"], deadline=deadline)
    if not files:
        return {}
    selection = home / "contracts-native-inputs.json"
    save(selection, [{"service": service, "files": files}])
    common = ["--clew", job["clew"], "--root", job["root"]]
    try:
        packet = command([job["clew"], "docs", "evidence", "retain-source", "--root", job["root"],
                          "--snapshot", job["snapshot"], "--service", service, "--symbol", symbols[0],
                          "--native-inputs", str(selection)], home, "contracts-native",
                         timeout=min(8, max(.1, deadline - time.monotonic())))
        if not isinstance(packet, dict) or packet.get("snapshot") != job["snapshot"]:
            return {}
        selected = [item["id"] for item in packet.get("items", ()) if item.get("kind") == "SOURCE_NATIVE"
                    and (record := item.get("record", {})).get("file") in files
                    and record.get("service") == service and record.get("revision") == revision
                    and isinstance(record.get("text"), str) and projector["json_declaration"](record["text"])]
        if not selected or time.monotonic() + 1 >= deadline:
            return {}
        directory = home / "contracts-native-citations"
        script = Path(__file__).with_name("author_citations.py")
        command([sys.executable, "-I", "-S", str(script), "import-native", *common,
                 "--input", str(home / "contracts-native.stdout"), "--output-dir", str(directory),
                 *(value for identity in selected for value in ("--source", identity))], home, "bind-native-contracts",
                timeout=min(8, max(.1, deadline - time.monotonic())))
        manifest = directory / "citations.json"
        _, validated, _, entries = citation_load(manifest)
        if validated["scope"] != {"service": service, "snapshot": job["snapshot"]}:
            return {}
        return packet_sources(entries, validated, manifest)
    except (OSError, Stop, CitationStop, ValueError, AttributeError, KeyError):
        return {}


def prepare(job, service, scope, requirements, exact, resources, repository, home, deadline):
    if not any(row.get("entryKind") == "HTTP_ENDPOINT" for row in requirements):
        return {}, {}
    projector = openapi
    output = home / "contracts-retained"
    output.mkdir(mode=0o700)
    operations, resources = [], resource_candidates(resources, projector["MAX_FILES"])
    routes = {projector["route"](row.get("method"), row.get("path")) for row in requirements
              if row.get("entryKind") == "HTTP_ENDPOINT"} - {None}
    if time.monotonic() + 1 < deadline:
        try:
            rows = work_rows(job, scope, output / "operations", {"kind": "CONTRACT_OPERATION"}, deadline)
            for row in rows or ():
                record = row.get("record") or {}
                operation = record.get("normalized") or {}
                if (row.get("kind") == "DEPENDENCY" and record.get("kind") == "CONTRACT_OPERATION"
                        and record.get("service") == service
                        and projector["route"](operation.get("method"), operation.get("path"),
                                              specification=True) in routes):
                    operations.append(row)
        except (Stop, ValueError, OSError):
            pass
    candidates = {handle: source for handle, source in exact.items() if source["file"].lower().endswith(".json")
                  or source["authority"] == "DECLARED_OPENAPI"}
    if operations or resources:
        candidates.update(acquire_sources(job, service, scope, output, [operations, resources], exact, deadline,
                                          authorities={"EXACT_SNAPSHOT_TEXT", "DECLARED_OPENAPI"}))
    if time.monotonic() + 2 < deadline:
        candidates.update(pinned_sources(job, service, scope["revision"], requirements, repository, candidates,
                                         {row["file"] for row in exact.values()}, projector, output, deadline))
    return projector["project"](service, scope["revision"], requirements, candidates, operations), candidates
