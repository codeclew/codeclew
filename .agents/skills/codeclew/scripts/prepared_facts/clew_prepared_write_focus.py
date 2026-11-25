"""Bind exact write positions before invoking the language-specific focused projectors."""

from hashlib import sha256
import time


MAX_WRITES = 4
MAX_TOPICS = 2
MAX_FACTS_PER_WRITE = 6
WRITES = {"INSERT", "UPDATE", "DELETE"}
FOCUS_KINDS = {"write_compatibility", "write_guard", "write_request", "write_commit", "write_lineage",
               "write_formation", "write_sql", "write_skip", "write_boundary", "sql-write",
               "conditional-no-write", "http-commit-before-response", "transactional-write-completion"}


def absolute_bounds(parent):
    for row in (parent, parent.get("occurrence") or {}):
        start, end = row.get("startByte"), row.get("endByte")
        if type(start) is type(end) is int and 0 <= start < end:
            return start, end
    return None


def admitted_parent(parent, context):
    """Recover the unique retained occurrence behind the adapted body citation."""
    originals = context.get("_sourceOccurrences", {})
    identity = ("file", "revision", "textDigest", "startLine", "endLine")
    candidates = [row for ref in parent.get("references", ()) if (row := originals.get(ref)) and not row.get("peerService")
                  and not row.get("peerProcess") and all(row.get(key) == parent.get(key) for key in identity)
                  and absolute_bounds(row) is not None]
    records = [row for row in [parent, *candidates] if absolute_bounds(row) is not None]
    bounds = {absolute_bounds(row) for row in records}
    origins = {(proof["snapshot"], proof["blob"]) for row in records if (proof := row.get("occurrence"))
               and proof.get("snapshot") and proof.get("blob")}
    if len(bounds) != 1 or len(origins) > 1:
        return None
    start, end = bounds.pop()
    proof = next((proof for row in records if (proof := row.get("occurrence"))
                  and proof.get("snapshot") and proof.get("blob")), None)
    admitted = {**parent, "startByte": start, "endByte": end}
    if proof:
        admitted["occurrence"] = proof
    return admitted


def selected_site(parent, address, identifier):
    text = parent["text"]
    raw = text.encode()
    bounds = absolute_bounds(parent)
    if bounds is None or bounds[1] - bounds[0] != len(raw):
        return None
    start, end = address.get("start"), address.get("end")
    if type(start) is not int or type(end) is not int:
        return None
    fmt = address.get("units")
    try:
        if fmt == "UTF16":
            units = text.encode("utf-16-le")
            if not 0 <= start < end <= len(units) // 2:
                return None
            first = len(units[:2 * start].decode("utf-16-le").encode())
            last = len(units[:2 * end].decode("utf-16-le").encode())
        elif fmt == "CHARACTERS" and 0 <= start < end <= len(text):
            first, last = len(text[:start].encode()), len(text[:end].encode())
        elif fmt == "ABSOLUTE_BYTES":
            first, last = start - bounds[0], end - bounds[0]
        else:
            return None
        if not 0 <= first < last <= len(raw) or last - first > 8192:
            return None
        selected = raw[first:last]
        visible = selected.decode()
    except UnicodeError:
        return None
    line = parent["startLine"] + raw[:first].count(b"\n")
    fingerprint = "sha256:" + sha256(selected).hexdigest()
    if fmt == "ABSOLUTE_BYTES" and address.get("textDigest") != fingerprint:
        return None
    byte_start, byte_end = bounds[0] + first, bounds[0] + last
    metadata = {key: value for key, value in parent.items() if key not in {"occurrence", "textIn"}}
    proof = parent.get("occurrence") or {}
    if proof.get("snapshot") and proof.get("blob"):
        metadata["occurrence"] = {**proof, "startByte": byte_start, "endByte": byte_end}
    return {**metadata, "id": identifier, "references": [identifier], "text": visible,
            "textDigest": fingerprint, "startLine": line, "endLine": line + selected.removesuffix(b"\n").count(b"\n"),
            "startByte": byte_start, "endByte": byte_end}


def incoming_topics(process):
    configuration = (process.get("trigger") or {}).get("configuration") or {}
    topics = configuration.get("topics", ())
    if isinstance(topics, str):
        topics = (topics,)
    if not isinstance(topics, (tuple, list)):
        return ()
    return tuple(dict.fromkeys(value for value in topics if isinstance(value, str)))[:MAX_TOPICS]


def unique_details(results):
    """Keep distinct kinds visible within the per-occurrence delivery bound."""
    groups = {}
    for result in results:
        if result["kind"] in FOCUS_KINDS:
            groups.setdefault(result["kind"], []).append(result)
    layers = (group[index] for index in range(MAX_FACTS_PER_WRITE) for group in groups.values() if index < len(group))
    chosen = []
    for result in layers:
        chosen.append(result)
        if len(chosen) == MAX_FACTS_PER_WRITE:
            break
    return chosen


def project_write(process, context, site, invoke, memo, deadline, topic):
    """Replay one already validated write; reuse results only within this operation."""
    identity = tuple(site[key] for key in ("file", "revision", "startByte", "endByte", "textDigest"))
    retained = site.get("occurrence") or {}
    key = identity, retained.get("snapshot"), retained.get("blob"), topic
    if key in memo:
        return memo[key]
    timeout = deadline - time.monotonic()
    if timeout <= 0:
        return None
    identifier = site["id"]
    allowed = {row["id"] for row in context["sourceRanges"]} | {identifier}
    frame = {**context, "sourceRanges": [*context["sourceRanges"], site],
             "_sourceOccurrences": {**context.get("_sourceOccurrences", {}), identifier: site}}
    operation = {**process, "sources": {**process["sources"], identifier: site}}
    results = invoke(operation, frame, {**site, "incomingTopic": topic}, timeout)
    details = unique_details(result for result in results if result.get("sources") and set(result["sources"]) <= allowed)
    memo[key] = site, details
    return memo[key]


def project(process, context, rows, invoke, *, specialize_topics=False, reserved=()):
    """Use ordinary reached writes to select exact sites for the existing focused projectors."""
    if not process.get("root"):
        return [], {}
    sources = {site["id"]: site for site in context["sourceRanges"]}
    occupied = set(sources) | set(reserved)
    matches, witnesses, output, memo = set(), {}, [], {}
    topics = incoming_topics(process) if specialize_topics else ()
    deadline = min(context.get("_deadline", float("inf")), time.monotonic() + 5)
    for row in rows:
        edge, address = row.get("_edge") or {}, row.get("_writeRange")
        occurrence = edge.get("occurrence")
        if (edge.get("kind") not in WRITES or not occurrence or occurrence in matches or edge.get("site") not in row.get("sources", ())
                or not address or address.get("host") not in sources):
            continue
        if len(matches) >= MAX_WRITES or deadline <= time.monotonic():
            break
        parent = admitted_parent(sources[address["host"]], context)
        index = len(occupied) + 1
        while (identifier := f"S{index:03d}") in occupied:
            index += 1
        site = selected_site(parent, address, identifier) if parent else None
        if site is None or ((bind := context.get("_bindWitness")) and not bind(site)):
            continue
        occupied.add(identifier)
        matches.add(occurrence)
        for topic in topics or (None,):
            if not (replayed := project_write(process, context, site, invoke, memo, deadline, topic)):
                break
            anchor, details = replayed
            ref = anchor["id"]
            for result in details:
                details_only = {key: value for key, value in result.items() if key not in {"_edge", "_writeRange"}}
                output.append({**details_only, "sources": list(dict.fromkeys([ref, *result["sources"]])),
                               "_writeFocus": {"occurrence": occurrence, "topic": topic,
                                               "compatibility": result.get("compatibility")}})
            if details:
                witnesses[ref] = anchor
    return output, witnesses
