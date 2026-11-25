"""Adapt admitted Work source and program rows to bounded language projections."""

from functools import lru_cache
from hashlib import sha256
import json
from pathlib import Path
import runpy
import shutil
import subprocess
from tempfile import TemporaryDirectory
import time


HERE = Path(__file__).resolve().parent
KOTLIN = {".kt", ".kts"}
TYPESCRIPT = {".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"}
MANAGED = {".cs"}
SOURCE_AUTHORITY = "EXACT_SNAPSHOT_TEXT"
MAX_DISCOVERY_BODIES = 8
MAX_FILE_BYTES = 128 * 1024


@lru_cache
def module(name):
    return runpy.run_path(str(HERE / ("clew_" + name + ".py")))


def fingerprint(text):
    return "sha256:" + sha256(text.encode()).hexdigest()


def exact(site):
    text = site.get("text")
    return (site.get("authority") == SOURCE_AUTHORITY and isinstance(text, str) and bool(text)
            and site.get("textDigest") == fingerprint(text))


def root_sources(process):
    return {ref for row in process.get("facts", ()) if row["kind"] == "BODY"
            and row.get("declaration") == process.get("root") for ref in row["sources"]}


def remaining(context, limit):
    deadline = context.get("_deadline")
    return min(limit, deadline - time.monotonic()) if deadline is not None else limit


def limitation(context, reason):
    if "_projectionUnavailable" in context:
        context["_projectionUnavailable"].append(reason)


def unavailable(context, reason):
    limitation(context, reason)
    return []


def adapt_sources(process, context, extensions):
    selected = [row for row in context["sourceRanges"] if Path(row["file"]).suffix.lower() in extensions
                and exact(row) and not row.get("peerService") and not row.get("peerProcess")]
    aliases, sites = {}, []
    for index, row in enumerate(selected, 1):
        alias = f"S{index:03d}"
        aliases[alias] = row["id"]
        sites.append({**row, "id": alias})
    by_handle = {handle: alias for alias, handle in aliases.items()}
    entry_sources = [by_handle[handle] for handle in context.get("entrySources", ()) if handle in by_handle]
    bindings = {ref: row for row in sites for ref in row.get("references", ())}
    identity = ("file", "revision", "startLine", "endLine", "textDigest")
    native = {ref: row for ref, row in process.get("sources", {}).items() if ref in bindings and exact(row)
              and all(row.get(key) == bindings[ref].get(key) for key in identity)}
    for row in sites:
        row["references"] = [ref for ref in row.get("references", ()) if ref in native]
    return ({**process, "sources": native}, {**context, "sourceRanges": sites, "entrySources": entry_sources,
             "entryKind": process.get("entryKind"), "trigger": process.get("trigger"), "title": process.get("title"),
             "_sourceOccurrences": {ref: row for ref, row in context.get("_sourceOccurrences", {}).items()
                                    if ref in native and exact(row)}}, aliases)


def complete_site(context):
    file = context.get("file")
    whole = context.get("_nativeFiles", {}).get(file)
    if not whole or not isinstance(whole.get("text"), str) or len(whole["text"].encode()) > MAX_FILE_BYTES:
        return None
    return next((row for row in context["sourceRanges"]
                 if (row["file"], row["revision"], row["text"]) == (file, whole["revision"], whole["text"])
                 and row["startLine"] == 1), None)


def kotlin_discovery(process, context):
    site = complete_site(context)
    if site is None:
        return [], {}
    projector = module("prepared_kotlin")
    parser = projector["Kotlin"]
    try:
        unit = parser(site["text"], site["file"])
    except projector["Unsupported"]:
        return [], {}
    scopes = {None, *(owner.body for owner in unit.owners)}
    facts, witnesses, count = [], {}, 0
    for index, word in enumerate(unit.words[:-1]):
        if word != "fun" or unit.scope[index] not in scopes:
            continue
        end = unit.pairs[unit.scope[index]] if unit.scope[index] is not None else len(unit.tokens)
        function = unit.function(index, end)
        if function is None:
            continue
        if count == MAX_DISCOVERY_BODIES or remaining(context, 10) <= 0:
            facts.append({"kind": "SCOPE_LIMIT", "text": "More Kotlin declarations remain beyond the bounded projection.",
                          "sources": [site["id"]], "scope": "PROJECTION_BUDGET"})
            break
        count += 1
        span = unit.function_site(function)
        text = unit.excerpt(span)
        first = 1 + unit.text.count("\n", 0, span.start)
        start = len(unit.text[:span.start].encode())
        alias = f"S{len(context['sourceRanges']) + count:03d}"
        narrowed = {**site, "id": alias, "text": text, "textDigest": fingerprint(text), "references": [alias],
                    "startLine": first, "endLine": first + text.count("\n"),
                    "startByte": start, "endByte": start + len(text.encode())}
        branch = {**process, "root": alias, "entrySymbol": function.name, "entryKind": None, "trigger": None,
                  "sources": {alias: narrowed}, "facts": [{"kind": "BODY", "declaration": alias, "sources": [alias]}]}
        rows = projector["derive"](branch, {**context, "sourceRanges": [narrowed], "_sourceOccurrences": {alias: narrowed}})
        facts.extend(rows)
        if rows:
            witnesses[alias] = narrowed
    return facts, {ref: witnesses[ref] for row in facts for ref in row["sources"] if ref in witnesses}


def kotlin_receivers(process, context):
    """Join supported receiver declaration syntax to a retained callable."""
    bind_witness = context.get("_bindWitness")
    roots = root_sources(process) & process["sources"].keys()
    if not bind_witness or len(roots) != 1 or remaining(context, 10) <= 0:
        return process, context, {}
    root, = roots
    source = process["sources"][root]
    text = pinned_text(context, source)
    if not text or len(text.encode()) > MAX_FILE_BYTES:
        return process, context, {}

    projector = module("prepared_kotlin")
    try:
        unit = projector["Kotlin"](text, source["file"])
        method = unit.select(source)
        owner = unit.enclosing_owner(method) if method else None
        if owner is None:
            return process, context, {}
        _, fields = unit.members(owner)
    except projector["Unsupported"]:
        return process, context, {}
    fields = {name: field for name, field in fields.items() if field.type in module("prepared_kotlin_protocols")["TYPES"]
              and not field.inferred_type and field.stable}
    if not fields:
        return process, context, {}

    witnesses, rows, declaration_refs = {}, [], {}
    start_index = method.body + 1

    def add_witness(site):
        raw = unit.excerpt(site)
        start_line = 1 + text.count("\n", 0, site.start)
        start_byte = len(text[:site.start].encode())
        alias = f"S{len(context['sourceRanges']) + len(witnesses) + 1:03d}"
        item = {**source, "id": alias, "references": [alias], "text": raw, "textDigest": fingerprint(raw),
                "startLine": start_line, "endLine": start_line + raw.count("\n"),
                "startByte": start_byte, "endByte": start_byte + len(raw.encode())}
        if bind_witness(item):
            witnesses[alias] = item
            return alias
        return None

    for index in range(start_index, method.end):
        name = unit.words[index]
        if (name not in fields or unit.words[index + 1] not in {".", "?."}
                or unit.words[index - 1] in {".", "?."} or len(rows) >= 24):
            continue
        field = fields[name]
        if name not in declaration_refs:
            declaration_refs[name] = add_witness(field.site)
        declaration = declaration_refs[name]
        if declaration and (use := add_witness(unit.site(index, index + 1))):
            rows.append({"kind": "FIELD", "authority": projector["SYNTAX"],
                         "details": {"member": name, "declaredType": field.type, "conditionalReceiver": True},
                         "sites": [{"role": "use", "reference": use},
                                   {"role": "declaration", "reference": declaration}]})
    return ({**process, "sources": {**process["sources"], **witnesses}, "entryKotlinSourceContext": [*process.get("entryKotlinSourceContext", ()), *rows]},
            {**context, "sourceRanges": [*context["sourceRanges"], *witnesses.values()],
             "_sourceOccurrences": {**context["_sourceOccurrences"], **witnesses}}, witnesses)


def typescript(process, context, node):
    projector = module("prepared_typescript")
    if not process.get("root"):
        site = complete_site(context)
        if site is None:
            return []
        body = {"id": site["id"], "file": site["file"], "text": site["text"], "anchors": []}
        request = projector["request_for"](process, {site["id"]: body}, {}, allow_retained_body=True)
        request["fileRoot"] = site["id"]
    else:
        excerpts, references = projector["retained_ranges"](process, context)
        request = projector["request_for"](process, excerpts, references, context.get("_sourceOccurrences"),
                                            allow_retained_body=True)
        if not (request and (request["roots"] or request["setup"])):
            return []
    if background := process.get("_background"):
        projector["background_anchors"](request, references, process["sources"], background)
    timeout = remaining(context, 12)
    if timeout <= 0:
        return []
    if not node:
        return unavailable(context, "TYPESCRIPT_COMPILER_UNAVAILABLE_OR_MISMATCHED")
    facts = projector["project"](request, context.get("_typescriptModule"), node=node, timeout=timeout)
    if any(row["kind"] == "analysis_boundary" and row["text"] == "TypeScript source facts are unavailable."
           for row in facts):
        return unavailable(context, "TYPESCRIPT_COMPILER_UNAVAILABLE_OR_MISMATCHED")
    if facts and not request["calls"] and (request.get("fileRoot") or any(
            row["kind"] in {"CALL_BOUNDARY", "CANDIDATE"} or row.get("event") in {"CALL", "CONSTRUCT"}
            for row in process.get("facts", ()))):
        limitation(context, "TYPESCRIPT_CALL_TARGETS_OPEN")
    return facts


def build_roslyn(dotnet, timeout):
    """Build the packaged helper once in an isolated temporary artifact directory."""
    build = TemporaryDirectory(prefix="clew-prepared-roslyn-")
    output = Path(build.name)
    project = HERE / "roslyn" / "PreparedControlFlow.csproj"
    try:
        result = subprocess.run(
            [dotnet, "build", str(project), "--nologo", "--artifacts-path", str(output),
             "-p:NuGetAudit=false", "--disable-build-servers"], cwd=output, check=False,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout)
        matches = list((output / "bin").glob("PreparedControlFlow/*/PreparedControlFlow.dll"))
        if result.returncode == 0 and len(matches) == 1:
            return build, matches[0]
    except (OSError, subprocess.SubprocessError):
        pass
    build.cleanup()
    return None


def pinned_text(context, source):
    file, revision = source["file"], source["revision"]
    whole = context.get("_nativeFiles", {}).get(file)
    if whole and whole["revision"] == revision and fingerprint(whole["text"]) == whole["textDigest"]:
        return whole["text"]
    if loader := context.get("_loadPinnedFile"):
        return loader(file, revision)
    return None


def captured_csharp_unit(context, source):
    """Require the original Work unit before replaying any Roslyn decision."""
    file, revision = source["file"], source["revision"]
    whole = context.get("_nativeFiles", {}).get(file)
    if not whole or (whole.get("file"), whole.get("revision")) != (file, revision):
        return None
    proof = whole.get("authority")
    occurrence = whole.get("occurrence") or {}
    source_occurrence = source.get("occurrence") or {}
    if proof not in {"RETAINED_CSHARP_COMPILATION_UNIT", "RECONSTRUCTED_GIT_BLOB"}:
        return None
    if proof == "RECONSTRUCTED_GIT_BLOB" and not source_occurrence:
        return None
    if (source_occurrence and (source_occurrence.get("generatedOrigin")
            or (source_occurrence.get("snapshot"), source_occurrence.get("blob"))
            != (occurrence.get("snapshot"), occurrence.get("blob")))):
        return None
    text = whole.get("text")
    if not isinstance(text, str) or not text or fingerprint(text) != whole.get("textDigest"):
        return None
    raw = text.encode()
    if (len(raw) > MAX_FILE_BYTES or occurrence.get("generatedOrigin")
            or not occurrence.get("snapshot") or not occurrence.get("blob")
            or occurrence.get("startByte") != 0 or occurrence.get("endByte") != len(raw)):
        return None
    return raw, occurrence["snapshot"]


def managed(process, context):
    evidence = process["sources"]
    roots = root_sources(process) & evidence.keys()
    if not roots:
        return []
    effects = module("effect_continuations")
    verified, covered, failed_roots = {}, set(), set()
    source_snapshot = None
    selections = effects["eligible"](process, evidence)
    for ref in sorted(selections, key=lambda ref: (not selections[ref][1], ref)):
        if len(verified) >= effects["MAX_FILES"] or remaining(context, 10) <= 0:
            break
        source = evidence[ref]
        key = source["file"], source["revision"]
        unit = captured_csharp_unit(context, source)
        if unit is None:
            if ref in roots:
                failed_roots.add("CSHARP_CAPTURED_COMPILATION_UNIT_UNAVAILABLE")
            continue
        raw, snapshot = unit
        if source_snapshot is None and ref in roots:
            source_snapshot = snapshot
        if source_snapshot != snapshot:
            if ref in roots:
                failed_roots.add("CSHARP_CAPTURED_COMPILATION_UNIT_AMBIGUOUS")
            continue
        interval = effects["locate"](raw, source)
        occurrence = source.get("occurrence") or {}
        if interval is None or (occurrence and interval != (occurrence.get("startByte"), occurrence.get("endByte"))):
            if ref in roots:
                failed_roots.add("CSHARP_CAPTURED_BODY_MISMATCH")
            continue
        verified[key] = raw
        covered.add(ref)
    grouped = {row["id"]: {**row, "references": [ref for ref in row["references"] if ref in covered]}
               for row in context["sourceRanges"]}
    managed_evidence = {ref: evidence[ref] for ref in covered}
    if failed_roots or not roots <= covered:
        for reason in sorted(failed_roots or {"CSHARP_CAPTURED_COMPILATION_UNIT_UNAVAILABLE"}):
            limitation(context, reason)
        return []
    if selections.keys() - covered:
        limitation(context, "CSHARP_HELPER_COMPILATION_UNITS_INCOMPLETE")
    # Entry requirements already carry declared route metadata. A body
    # citation by itself does not attest to a route via the surrounding registration.
    if roslyn_command(context) is None:
        return unavailable(context, "ROSLYN_SDK_OR_BUILD_UNAVAILABLE")
    graph = effects["derive"]({**process, "routes": []}, managed_evidence, grouped, verified,
                              lambda request: roslyn_request(request, context), lambda blobs, source: blobs[source["file"], source["revision"]].decode(),
                              module("effect_storage"), module("source_candidates")["sql_tables"],
                              module("effect_commands"))
    if graph and graph.get("status") == "OPEN":
        limitation(context, "CSHARP_CONTROL_FLOW_PROJECTION_OPEN")
    return module("prepared_managed")["derive"]({**process, "routes": []}, context, graph)

def roslyn_command(context):
    dotnet = shutil.which("dotnet")
    timeout = remaining(context, 35)
    if dotnet is None or timeout <= 0:
        return None
    if dotnet not in _roslyn_builds:
        _roslyn_builds[dotnet] = build_roslyn(dotnet, timeout)
    if compiled := _roslyn_builds[dotnet]:
        directory, dll = compiled
        return [dotnet, str(dll)], directory.name
    return None


def roslyn_request(request, context):
    command = roslyn_command(context)
    timeout = remaining(context, 12)
    if command is None or timeout <= 0:
        return None
    argv, cwd = command
    try:
        result = subprocess.run(argv, input=json.dumps(request), encoding="utf-8",
                                cwd=cwd, capture_output=True, check=True, timeout=timeout)
        return json.loads(result.stdout)
    except (OSError, ValueError, subprocess.SubprocessError):
        return None


def discover_background(files, *, node, deadline, family):
    payload = {file: row["text"] for file, row in files.items()}
    context = {"_deadline": deadline}
    if family == "csharp":
        return roslyn_request({"files": payload, "discoverHostedWorkers": True}, context)
    timeout = remaining(context, 12)
    if not node or timeout <= 0:
        return None
    return module("prepared_typescript")["project"](
        {"backgroundFiles": payload, "roots": [], "setup": []}, None, node=node, timeout=timeout)


_roslyn_builds = {}


def project(process, context, *, node):
    """Return supported propositions using Work handles or exact narrower witnesses."""
    root = (process.get("sources", {}).get(ref, {}) for ref in sorted(root_sources(process)))
    extensions = {Path(row["file"]).suffix.lower() for row in root if "file" in row}
    if not process.get("root") and context.get("file"):
        extensions = {Path(context["file"]).suffix.lower()}
    families = [(family, authority) for family, authority in (
        (KOTLIN, "BOUNDED_KOTLIN_SOURCE_PROJECTION"),
        (TYPESCRIPT, "BOUNDED_TYPESCRIPT_SOURCE_SYNTAX"),
        (MANAGED, "BOUNDED_ROSLYN_SOURCE_PROJECTION"),
    ) if family & extensions]
    if len(families) != 1:
        return [], {}
    family, authority = families[0]
    process, context, aliases = adapt_sources(process, context, family)
    witnesses = {}
    if family is KOTLIN:
        if process.get("root"):
            process, context, witnesses = kotlin_receivers(process, context)
            rows = module("prepared_kotlin")["derive"](process, context)
        else:
            rows, witnesses = kotlin_discovery(process, context)
    elif family is TYPESCRIPT:
        rows = typescript(process, context, node)
    else:
        rows = managed(process, context)
    allowed = set(aliases) | witnesses.keys()
    facts = []
    for row in rows:
        refs = row.get("sources", [])
        if not refs or any(ref not in allowed for ref in refs):
            continue
        values = {key: row[key] for key in ("kind", "text", "condition", "uncertainty", "status") if key in row}
        if (edge := row.get("_edge")) and edge["site"] in refs:
            values["_edge"] = {**edge, "site": aliases.get(edge["site"], edge["site"])}
        if "_edgeCause" in row:
            values["_edgeCause"] = row["_edgeCause"]
        declared = row.get("authority")
        values["authority"] = declared if isinstance(declared, str) else (
            "RETAINED_COMPILER_CALL_SITE" if row.get("scope", "").startswith("COMPILER_SELECTED_") else authority)
        values["sources"] = list(dict.fromkeys(aliases.get(ref, ref) for ref in refs))
        facts.append(values)
    return facts, {ref: witnesses[ref] for row in facts for ref in row["sources"] if ref in witnesses}
