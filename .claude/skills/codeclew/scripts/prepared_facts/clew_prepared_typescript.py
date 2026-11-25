"""Project TypeScript facts from verified reader ranges and retained relationships."""

from collections import defaultdict
from hashlib import sha256
import json
from pathlib import Path
import re
import subprocess


TYPESCRIPT_FILE = re.compile(r"\.(?:[cm]?[jt]s|[jt]sx)$", re.IGNORECASE)
SOURCE_ID = re.compile(r"S[0-9]{3,}")
BINDINGS = {"TYPE_CHECKER_BINDING", "UNIQUE_LEXICAL_DECLARATION"}
TRIGGERS = {"KAFKAJS_RETAINED_SOURCE", "TIMER_RETAINED_SOURCE"}


def digest(text):
    return "sha256:" + sha256(text.encode()).hexdigest()


def position(site):
    return tuple(site.get(key) for key in ("file", "revision", "startLine", "endLine"))


def span(source):
    values = source.get("startByte"), source.get("endByte")
    if all(isinstance(value, int) for value in values) and 0 <= values[0] < values[1]:
        return values
    occurrence = source.get("occurrence") or {}
    values = occurrence.get("startByte"), occurrence.get("endByte")
    if all(isinstance(value, int) for value in values) and 0 <= values[0] < values[1]:
        return values
    return None


def retained_ranges(process, context):
    """Resolve exact occurrences before joining internal references to reader IDs."""
    metadata = defaultdict(list)
    for ref, source in process.get("sources", {}).items():
        if source.get("authority") == "EXACT_SNAPSHOT_TEXT" and not source.get("peerProcess"):
            metadata[position(source)].append((ref, source))

    excerpts, references, matched = {}, {}, {}
    sites = {}
    for site in context.get("sourceRanges", ()):
        identifier = site.get("id", "")
        if not SOURCE_ID.fullmatch(identifier) or not TYPESCRIPT_FILE.search(site.get("file", "")):
            continue
        aliases = set(site.get("references", ()))
        candidates = [(ref, source) for ref, source in metadata[position(site)] if not aliases or ref in aliases]
        sites[identifier] = site, candidates
        if isinstance(site.get("text"), str):
            literal = site["text"]
            matches = [(ref, source) for ref, source in candidates if source.get("textDigest") == digest(literal)]
            if not matches:
                continue
            excerpts[identifier] = {"id": identifier, "file": site["file"], "text": literal,
                                    "startLine": site["startLine"], "anchors": []}
            matched[identifier] = matches
            for ref, _ in matches:
                references[ref] = identifier

    private = context.get("_sourceOccurrences", {})
    for identifier, (site, candidates) in sites.items():
        parent = site.get("textIn")
        if parent not in excerpts or identifier in excerpts:
            continue
        body = excerpts[parent]
        raw = body["text"].encode()
        occurrences = defaultdict(list)
        for ref, source in candidates:
            source_span = span(source) or span(private.get(ref, {}))
            if not source_span:
                continue
            for parent_ref, containing in matched[parent]:
                parent_span = span(containing) or span(private.get(parent_ref, {}))
                if (position(source)[:2] != position(containing)[:2] or not parent_span
                        or parent_span[1] - parent_span[0] != len(raw)):
                    continue
                start, end = (value - parent_span[0] for value in source_span)
                if 0 <= start < end <= len(raw):
                    try:
                        literal = raw[start:end].decode()
                    except UnicodeDecodeError:
                        continue
                    if digest(literal) == source.get("textDigest"):
                        occurrences[start, end].append(ref)
        if len(occurrences) == 1:
            (start, end), refs = next(iter(occurrences.items()))
            # TypeScript positions count UTF-16 code units.
            units = lambda byte: len(raw[:byte].decode().encode("utf-16-le")) // 2
            body["anchors"].append({"id": identifier, "start": units(start), "end": units(end)})
            for ref in refs:
                references[ref] = identifier
    return excerpts, references


def request_for(process, excerpts, references, private=None, *, allow_retained_body=False):
    flow = process.get("sourceFlow", {})
    linked = process.get("entryLinkedDefinitions", {})
    trigger = process.get("trigger") or {}
    if (not allow_retained_body and not flow and linked.get("authority") != "RETAINED_TYPESCRIPT_SOURCE_AND_CHECKER"
            and trigger.get("authority") not in TRIGGERS):
        return None

    def ids(refs):
        return list(dict.fromkeys(references[ref] for ref in refs if ref in references))

    roots = ids([trigger.get("callback"), *(node.get("sourceReference") for node in flow.get("nodes", ())
                   if node.get("kind") == "START" and node.get("id") == flow.get("start"))])
    if not roots:
        roots = ids(ref for fact in process.get("facts", ())
                    if fact.get("kind") == "BODY" and fact.get("declaration") == process.get("root")
                    for ref in fact.get("sources", ()))
    calls, setup, callbacks = [], [], []
    consumers, state_uses, configurations = [], [], []
    sources = process.get("sources", {})

    def contains(container, member):
        left, right = sources.get(container, {}), sources.get(member, {})
        first = span(left) or span((private or {}).get(container, {}))
        second = span(right) or span((private or {}).get(member, {}))
        return (first and second and position(left)[:2] == position(right)[:2]
                and first[0] <= second[0] < second[1] <= first[1])

    for row in linked.get("rows", ()):
        steps = defaultdict(list)
        aliases = defaultdict(list)
        guards = []
        for step in row.get("steps", ()):
            role = step["role"]
            identifier = references.get(step.get("reference"))
            if identifier:
                steps[role].append(identifier)
                aliases[role].append(step["reference"])
            if role in {"Condition true", "Condition false"}:
                guards.append({"source": identifier, "outcome": role.endswith("true")})
        if row.get("kind") == "LOCAL_CONTINUATION" and row.get("authority") in BINDINGS:
            if len(steps["Call site"]) == len(steps["Declared implementation"]) == 1:
                calls.append({"call": steps["Call site"][0], "target": steps["Declared implementation"][0]})
        elif row.get("kind") == "SOURCE_SETUP" and trigger.get("authority") in TRIGGERS:
            if any(not guard["source"] for guard in guards):
                continue
            consumers.extend(aliases["Consumer initialization"])
            for role in ("Registration", "Consumer connection setup", "Subscription setup"):
                for action in steps[role]:
                    setup.append({"role": role, "action": action, "guards": guards})
            if steps["Selected callback"] and steps["Handler configuration"]:
                configurations.extend(steps["Handler configuration"])
                callbacks.append({"sources": ids([trigger.get("registration")]) + steps["Selected callback"] +
                                  steps["Handler configuration"], "guards": guards})
        elif row.get("kind") == "STATE_DEFINITION":
            if len(steps["State use"]) == len(aliases["State declaration"]) == 1:
                state_uses.append((steps["State use"][0], aliases["State declaration"][0]))
        elif row.get("kind") == "CALLBACK_BINDING":
            controls, bodies = steps["Controlling call"], steps["Callback definition"]
            if len(controls) == len(bodies) == 1 and row.get("authority") in BINDINGS:
                callbacks.append({"control": controls[0], "body": bodies[0],
                                  "proof": steps["Local callback invocation site"]})
    for_at = defaultdict(list)
    for row in process.get("staticForwards", ()):
        if address := references.get(row.get("sourceReference")):
            for_at[address].append({"method": row["method"], "target": row["target"],
                                    "proof": ids([row.get("awaitReference")])})
    forwards = {identifier: rows[0] for identifier, rows in for_at.items() if len(rows) == 1}
    stages = [{"role": row["kind"], "sources": ids([row.get("sourceReference")])}
              | ({"target": row["claim"]["target"]} if row.get("claim", {}).get("target") else {})
              for row in flow.get("nodes", ()) if row.get("kind") in {"INSERT", "READ", "DELETE", "PUBLISH"}]
    callback = {"sources": ids([trigger.get("registration"), trigger.get("callback")]),
                "kind": trigger.get("kind"), "active": trigger.get("authority") in TRIGGERS}
    offset_receivers = [{"use": use, "declaration": references[declaration], "consumer": references[consumer]}
                        for use, declaration in state_uses for consumer in consumers if contains(consumer, declaration)]
    return {"roots": roots, "calls": calls, "callbacks": callbacks, "setup": setup, "configurations": configurations,
            "offsetReceivers": offset_receivers, "forwards": forwards, "stages": stages, "callback": callback,
            "excerpts": list(excerpts.values())}


def project(request, typescript_module, *, node="node", timeout=12):
    command = [node, str(Path(__file__).with_name("clew_prepared_typescript.cjs"))]
    if typescript_module:
        command.append(str(Path(typescript_module).resolve()))
    try:
        result = subprocess.run(command, input=json.dumps(request), encoding="utf-8", capture_output=True,
                                timeout=timeout, check=True)
        return json.loads(result.stdout)
    except (OSError, ValueError, subprocess.SubprocessError):
        if focus := request.get("focus"):
            return [{"kind": "write_boundary", "text": "The TypeScript compiler could not project this write.",
                     "sources": [focus["site"]]}]
        sites = request["roots"] or [row["action"] for row in request["setup"]]
        return [{"kind": "analysis_boundary", "text": "TypeScript source facts are unavailable.",
                 "sources": sites[:1], "uncertainty": "The TypeScript compiler projection did not complete."}]


def derive(process, context, typescript_module=None):
    """Return bounded source facts. The optional module matches preparation's compiler argument."""
    excerpts, references = retained_ranges(process, context)
    request = request_for(process, excerpts, references, context.get("_sourceOccurrences"))
    if not request or not (request["roots"] or request["setup"]) or not excerpts:
        return []
    allowed = {row["id"] for row in context.get("sourceRanges", ())}
    return [fact for fact in project(request, typescript_module) if fact["sources"] and set(fact["sources"]) <= allowed]


def selected_spans(process, context, excerpts, references, site):
    """Locate a verified byte range in each candidate body excerpt."""
    text = site.get("text")
    if not isinstance(text, str) or digest(text) != site.get("textDigest"):
        return {}
    raw = text.encode()
    trimmed = raw.strip()
    if not trimmed:
        return {}
    native = process.get("sources", {})
    private = context.get("_sourceOccurrences", {})

    def source_span(ref):
        return span(native[ref]) or span(private.get(ref, {}))

    aliases = defaultdict(list)
    for ref, identifier in references.items():
        source = native[ref]
        if position(source)[:2] == position(site)[:2] and (bounds := source_span(ref)):
            aliases[identifier].append((source, bounds))

    target = span(site)
    if target is None:
        candidates = {bounds for source, bounds in aliases.get(site.get("id"), ())
                      if position(source) == position(site) and source.get("textDigest") == site.get("textDigest")}
        if len(candidates) != 1:
            return {}
        target = candidates.pop()
    if target[1] - target[0] != len(raw):
        return {}

    selected = {}
    for identifier, excerpt in excerpts.items():
        body = excerpt["text"].encode()
        offsets = set()
        for source, (first, last) in aliases[identifier]:
            start, end = (value - first for value in target)
            if (last - first == len(body) and 0 <= start < end <= len(body) and body[start:end] == raw
                    and source["startLine"] + body[:start].count(b"\n") == site["startLine"]
                    and source["startLine"] + body[:end - 1].count(b"\n") == site["endLine"]):
                # TypeScript positions count UTF-16 code units.
                start += len(raw) - len(raw.lstrip())
                end = start + len(trimmed)
                offsets.add(tuple(len(body[:point].decode().encode("utf-16-le")) // 2 for point in (start, end)))
        if len(offsets) == 1:
            selected[identifier] = next(iter(offsets))
    return selected


def derive_write(process, context, graph, site, typescript_module=None):
    """Project paths to this verified write under the retained callback binding."""
    identifier = site.get("id", "")
    if not SOURCE_ID.fullmatch(identifier) or not TYPESCRIPT_FILE.search(site.get("file", "")):
        return []
    allowed = {row["id"] for row in context.get("sourceRanges", ())}
    if identifier not in allowed:
        return []
    excerpts, references = retained_ranges(process, context)
    spans = selected_spans(process, context, excerpts, references, site)
    request = request_for(process, excerpts, references, context.get("_sourceOccurrences"))
    callback = (process.get("trigger") or {}).get("callback")
    if request and callback in references:
        request["roots"] = [references[callback]]
    if not request or len(request["roots"]) != 1 or not spans:
        return [{"kind": "write_boundary", "text": "The exact write could not be attached to one retained TypeScript callback body.",
                 "sources": [identifier]}]
    request["focus"] = {"site": identifier, "spans": spans,
                        **({"topic": site["incomingTopic"]} if isinstance(site.get("incomingTopic"), str) else {})}
    return [fact for fact in project(request, typescript_module)
            if fact["sources"] and set(fact["sources"]) <= allowed]


def background_anchors(request, references, sources, background):
    """Map checker-selected relationships into exact offsets within callback excerpts."""
    excerpts = {row["id"]: row for row in request["excerpts"]}
    hosts = sorted((sources[ref] for ref in background["bodies"] if ref in references),
                   key=lambda site: len(site["text"].encode()))
    attached = set()
    for child in background["anchored"]:
        if child not in references:
            continue
        witness = sources[child]
        start, end = span(witness)
        for host in hosts:
            first, last = span(host)
            if position(witness)[:2] != position(host)[:2] or not first <= start < end <= last:
                continue
            raw = host["text"].encode()
            if raw[start - first:end - first] != witness["text"].encode():
                continue
            offsets = [len(raw[:byte - first].decode().encode("utf-16-le")) // 2 for byte in (start, end)]
            identifier = references[child]
            excerpts[references[host["id"]]]["anchors"].append(
                {"id": identifier, "start": offsets[0], "end": offsets[1]})
            attached.add(identifier)
            break
    request["excerpts"] = [row for row in request["excerpts"] if row["id"] not in attached]
    for call, target in background["calls"]:
        if call in references and target in references:
            request["calls"].append({"call": references[call], "target": references[target]})
    for use, declaration, consumer in background["offsets"]:
        if all(ref in references for ref in (use, declaration, consumer)):
            request["offsetReceivers"].append(
                {"use": references[use], "declaration": references[declaration], "consumer": references[consumer]})
    request["publications"] = {references[call]: [references[producer], references[client]]
                               for call, producer, client in background.get("publications", ())
                               if all(ref in references for ref in (call, producer, client))
                               and references[call] in attached}
    if registration := next(iter(request["callback"]["sources"]), None):
        request["setup"].append({"role": "Registration", "action": registration, "guards": []})
