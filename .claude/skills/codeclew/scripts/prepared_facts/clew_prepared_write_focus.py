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


def unique_details(results, retain_alternatives=False):
    """Keep distinct kinds visible within the per-occurrence delivery bound."""
    groups = {}
    for result in results:
        if result["kind"] in FOCUS_KINDS:
            groups.setdefault(result["kind"], []).append(result)
    lineage = groups.get("write_lineage", ())
    if len(lineage) > 1 and "write_formation" in groups:
        # Give a second source-proven value path the slot used for the generic initializer summary.
        groups["write_formation"].insert(0, lineage.pop(1))
    layers = (group[index] for index in range(MAX_FACTS_PER_WRITE) for group in groups.values() if index < len(group))
    chosen = []
    for result in layers:
        chosen.append(result)
        if len(chosen) == MAX_FACTS_PER_WRITE:
            break
    if retain_alternatives:
        represented = {row.get("condition") for row in chosen if row["kind"] == "write_request"}
        for row in groups.get("write_request", ()):
            if row.get("condition") not in represented:
                chosen.append(row)
                represented.add(row.get("condition"))
    return chosen


def site_identity(site):
    occurrence = site.get("occurrence") or {}
    fields = ("file", "revision", "startByte", "endByte", "textDigest")
    return (*(site[key] for key in fields), occurrence.get("snapshot"), occurrence.get("blob"))


def add_sites(context, rows, reserved=()):
    """Retain the exact selected invocation independently of adapter-local aliases."""
    sources = {site["id"]: site for site in context["sourceRanges"]}
    occupied = set(sources) | set(reserved)
    witnesses, known = {}, {}
    for row in rows:
        edge, address = row.get("_edge") or {}, row.get("_writeRange")
        if (edge.get("kind") not in WRITES or not edge.get("occurrence") or edge.get("site") not in row.get("sources", ())
                or not address or address.get("host") not in sources):
            continue
        parent = admitted_parent(sources[address["host"]], context)
        index = len(occupied) + 1
        while (identifier := f"S{index:03d}") in occupied:
            index += 1
        site = selected_site(parent, address, identifier) if parent else None
        if site is None or ((bind := context.get("_bindWitness")) and not bind(site)):
            continue
        identity = site_identity(site)
        if identity not in known:
            known[identity] = identifier
            witnesses[identifier] = site
            occupied.add(identifier)
        chosen = known[identity]
        row["_writeSite"] = chosen
        row["sources"] = list(dict.fromkeys([*row["sources"], chosen]))
    return witnesses


def project_sites(process, context, candidates, invoke, *, specialize_topics=False, reserved=(), limit=MAX_WRITES,
                  retain_alternatives=False, reuse_aliases=False):
    """Replay at most once per source coordinate and topic within this operation."""
    if not process.get("root"):
        return [], {}
    known, output, witnesses = {}, [], {}
    occupied = {site["id"] for site in context["sourceRanges"]} | set(reserved)
    topics = incoming_topics(process) if specialize_topics else ()
    deadline = min(context.get("_deadline", float("inf")), time.monotonic() + 5)
    for site, occurrence in candidates:
        known.setdefault(site_identity(site), (site, {}))[1][occurrence] = None
    limitations = context.get("_projectionUnavailable")
    if len(known) > limit and limitations is not None:
        limitations.append("WRITE_FOCUS_SITE_LIMIT")
    for site, occurrences in list(known.values())[:limit]:
        identifier = site["id"] if reuse_aliases else None
        if identifier is None:
            index = len(occupied) + 1
            while (identifier := f"S{index:03d}") in occupied:
                index += 1
        occupied.add(identifier)
        anchor = {**site, "id": identifier, "references": [identifier]}
        allowed = {row["id"] for row in context["sourceRanges"]} | {identifier}
        frame = {**context, "sourceRanges": [*context["sourceRanges"], anchor],
                 "_sourceOccurrences": {**context.get("_sourceOccurrences", {}), identifier: anchor}}
        operation = {**process, "sources": {**process["sources"], identifier: anchor}}
        for topic in topics or (None,):
            timeout = deadline - time.monotonic()
            if timeout <= 0:
                if limitations is not None:
                    limitations.append("WRITE_FOCUS_TIMEOUT")
                return output, witnesses
            results = invoke(operation, frame, {**anchor, "incomingTopic": topic}, timeout)
            supported = (row for row in results if row.get("sources") and set(row["sources"]) <= allowed)
            selected = unique_details(supported, retain_alternatives)
            if selected:
                witnesses[identifier] = anchor
            for result in selected:
                details = {key: value for key, value in result.items() if key not in {"_edge", "_writeRange"}}
                for occurrence in occurrences:
                    output.append({**details, "sources": list(dict.fromkeys([identifier, *result["sources"]])),
                                   "_writeSite": identifier,
                                   "_writeFocus": {"occurrence": occurrence, "topic": topic,
                                                   "compatibility": result.get("compatibility")}})
    return output, witnesses


def project(process, context, rows, invoke, *, specialize_topics=False, reserved=(), limit=MAX_WRITES,
            retain_alternatives=False):
    """Use ordinary reached write events to select their independent focused paths."""
    anchors = add_sites(context, rows, reserved)
    candidates = [(anchors[row["_writeSite"]], row["_edge"]["occurrence"])
                  for row in rows if row.get("_writeSite") in anchors]
    output, witnesses = project_sites(process, context, candidates, invoke, specialize_topics=specialize_topics,
                                      reserved=reserved, limit=limit, retain_alternatives=retain_alternatives,
                                      reuse_aliases=True)
    return output, {**anchors, **witnesses}
