#!/usr/bin/env python3
"""Portable citations over Work READ or engine-reloaded registered native bytes.

This checks mechanical bindings, never entailment, producer authenticity, current
runtime behavior or publication approval. Only explicit cite: links are expanded;
all other Markdown bytes are preserved. Work proof replay is local. SOURCE_NATIVE
requires the caller's trusted engine to reload its immutable evidence set.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import runpy
from string import Formatter
from urllib.parse import quote, urlsplit

B = runpy.run_path(str(Path(__file__).with_name("citation_evidence.py")))
Stop, digest, encode = (B[k] for k in ("Stop", "digest", "encode"))
R = B["ReadProof"]
MANIFEST = "codeclew-author-citations/1.0"
READY = ("DECLARATION_SOURCE_RETURNED", "RETAINED_SOURCE_RETURNED")
LINK = re.compile(r'\[[^\]\n]*\]\(([^\s)]+)\)')
NATIVE_ENGINE = None
NATIVE_MODULE = None


def configure_engine(clew=None, root=None):
    """Explicit caller authority; manifests never supply executable paths."""
    global NATIVE_ENGINE
    if (clew is None) != (root is None):
        raise Stop("NATIVE_ENGINE_REQUIRES_CLEW_AND_ROOT", exit_code=2)
    NATIVE_ENGINE = None if clew is None else (clew, root)


def native_module():
    global NATIVE_MODULE
    if NATIVE_MODULE is None:
        NATIVE_MODULE = runpy.run_path(str(Path(__file__).with_name("native_source_citations.py")),
            init_globals={"Stop": Stop, "digest": digest, "encode": encode})
    return NATIVE_MODULE


def warning_leaves(value, path=""):
    """Group equal diagnostic leaves; full positional metadata stays in packet."""
    if isinstance(value, dict):
        return {leaf for k, v in value.items() for leaf in warning_leaves(v, path + "/" + ("*" if str(k).isdigit() else str(k)))}
    if isinstance(value, list):
        return {leaf for v in value for leaf in warning_leaves(v, path + "/*")}
    return {(path, json.dumps(value, ensure_ascii=False, separators=(",", ":")))}


def warnings(value):
    return {path + "=" + scalar for path, scalar in warning_leaves(value)}


def grouped_warnings(value):
    """Retain every diagnostic path, share only exactly equal value vectors."""
    paths = {}
    for path, scalar in sorted(warning_leaves(value)):
        paths.setdefault(path, []).append(scalar)
    groups = []
    for path, scalars in paths.items():
        values = [json.loads(s) for s in scalars]
        group = next((g for g in groups if encode(g['values']) == encode(values)), None)
        if group is None:
            group = {'paths': [], 'values': values}; groups.append(group)
        group['paths'].append(path)
    return groups


def manifest_value(packet, discovery):
    value = {"schema": MANIFEST, "packet": "packet.json", "packetDigest": digest(packet),
             "discovery": "discovery.input.json", "discoveryDigest": digest(discovery),
             "entries": [{"key": "e" + str(i + 1), "outcome": i} for i, o in enumerate(packet.get("outcomes", []))
                         if o.get("status") in READY]}
    return {**value, "id": digest(value)}


def project(packet, discovery, directory, *, include_reused=False, save=True, source_union=False, include_texts=True):
    """Called directly by READ, not an extra model-visible verbose acquisition."""
    if packet.get("kind") == "NATIVE":
        if source_union:
            raise Stop("NATIVE_SOURCE_UNION_UNSUPPORTED", exit_code=2)
        return native_module()["project"](packet, packet["nativeManifest"],
            Path(directory) / "citations.json", include_texts=include_texts)
    manifest = manifest_value(packet, discovery)
    if save:
        (directory / "citations.json").write_text(encode(manifest), encoding="utf-8")
    tag = "m" + manifest["id"].split(":")[1][:12]
    records, texts, groups = [], {}, {}
    by_text = {}; failures = []; source_rows = []
    for i, outcome in enumerate(packet.get("outcomes", [])):
        if outcome.get("status") not in READY:
            failures.append(outcome)
            continue
        source = outcome["source"]
        declaration = outcome.get("declaration", {})
        key = outcome["textKey"]
        if key not in by_text:
            by_text[key] = "t" + str(len(by_text) + 1)
        text_key = by_text[key]
        if include_texts and (include_reused or not outcome.get("evidenceReused")):
            texts[text_key] = packet["texts"][key]
        leaves = sorted(warnings(declaration.get("limits", {})))
        group = next((k for k, v in groups.items() if v == leaves), None)
        if group is None:
            group = "l" + str(len(groups) + 1); groups[group] = leaves
        records.append({"cite": tag + ":e" + str(i + 1),
            **({"wholeFile": True} if outcome.get("fileBinding", {}).get("wholeFiles") else {}),
            "location": f'{source["service"]}:{source["file"]}:{source["startLine"]}-{source["endLine"]}',
            "role": declaration.get("descriptor", {}).get("name", declaration.get("symbol", "retained file fragment")),
            "authority": source.get("authority"), "bodyStatus": declaration.get("bodyStatus"),
            **({"scope": declaration["descriptor"]["scope"]} if "scope" in declaration.get("descriptor", {}) else {}),
            "limits": group, "text": text_key,
            **({"retainedText": True} if text_key not in texts else {})})
        source_rows.append((records[-1], outcome))
    union = {}
    if source_union:
        projection = runpy.run_path(str(Path(__file__).with_name("source_union.py")))
        records, texts, metadata = projection["compact"](source_rows, packet["texts"], packet["scope"], include_reused=include_reused)
        union["sourceUnion"] = metadata
    return {"schema": "codeclew-author-read/1.0", "status": packet["status"], **union,
        "manifest": {"path": str(directory / "citations.json"), "id": manifest["id"]},
        "records": records, "texts": texts, "limitations": groups, "failures": failures,
        "discoveryQueryComplete": packet.get("discoveryQueryComplete", False),
        "coverageWarnings": grouped_warnings(B["limitations"](packet.get("coverage", {}))),
        "meaning": ("Complete original retained file bytes where wholeFile=true; no semantic/runtime/publication approval."
                    if any(r.get("wholeFile") for r in records) else "Retained fragment; null body unspecified; no semantic/runtime/publication approval."),
        "inspect": "author_citations.py inspect --manifest PATH"}


def load(path):
    path = Path(path)
    if path.is_symlink() or (path.parent / "packet.json").is_symlink():
        raise Stop("UNSAFE_REUSE_ARTIFACT_PATH", exit_code=2)
    path = path.resolve(strict=True); manifest = json.loads(path.read_bytes())
    if isinstance(manifest, dict) and manifest.get("schema") == "codeclew-author-native-citations/1.0":
        result = native_module()["load"](path, NATIVE_ENGINE)
        result[1]["nativeManifest"] = result[0]
        return result
    if (not isinstance(manifest, dict) or manifest.get("schema") != MANIFEST
            or manifest.get("id") != digest({k: v for k, v in manifest.items() if k != "id"})
            or manifest.get("packet") != "packet.json" or manifest.get("discovery") != "discovery.input.json"):
        raise Stop("CITATION_MANIFEST_MISMATCH", exit_code=2)
    # Reuse the existing proof validator and safe artifact path reader.
    discovery_path = path.parent / "discovery.input.json"
    if discovery_path.is_symlink():
        raise Stop("UNSAFE_REUSE_ARTIFACT_PATH", exit_code=2)
    discovery_bytes = discovery_path.read_bytes(); discovery = json.loads(discovery_bytes)
    proof = R(path.parent / "packet.json", discovery["scope"], discovery["work"])
    packet = proof.packet
    if (manifest != manifest_value(packet, discovery)
            or packet.get("inputPacketDigest") != "sha256:" + hashlib.sha256(discovery_bytes).hexdigest()):
        raise Stop("CITATION_MANIFEST_MISMATCH", exit_code=2)
    if (packet.get("status") != "SELECTED_SOURCES_READY" or not packet.get("outcomes")
            or len(manifest["entries"]) != len(packet["outcomes"])):
        raise Stop("CITATION_EVIDENCE_INCOMPLETE", exit_code=2)
    rows = {row["id"]: row for row in discovery["choices"]}
    if len(rows) != len(discovery["choices"]):
        raise Stop("AMBIGUOUS_CITATION_DECLARATION", exit_code=2)
    tag = "m" + manifest["id"].split(":")[1][:12]
    entries = {}
    for entry in manifest["entries"]:
        outcome = packet["outcomes"][entry["outcome"]]
        retained = (proof.find_source(outcome["sourceBinding"], outcome["selection"]) if "sourceBinding" in outcome
                    else proof.find(rows[outcome["declaration"]["id"]], outcome["selection"]))
        if retained is None or retained[0] != outcome:
            raise Stop("CITATION_BINDING_UNAVAILABLE", exit_code=2)
        entries[tag + ":" + entry["key"]] = {"manifestId": manifest["id"],
            "scope": packet["scope"], "work": packet["work"], "source": outcome["source"],
            **({"sourceBinding":outcome["sourceBinding"]} if "sourceBinding" in outcome else {"declaration":outcome["declaration"]}),
            "sourceRecordDigest": outcome["sourceRecordDigest"]}
    return manifest, packet, discovery, entries


def source_view(packet, records, file, start_line=None, end_line=None, cite=None):
    """Slice exact original LF/CRLF lines, never interpret or summarize source."""
    matches = [i for i, outcome in enumerate(packet["outcomes"]) if outcome["source"]["file"] == file]
    if not matches:
        raise Stop("UNKNOWN_INSPECT_FILE", exit_code=2, file=file)
    if cite is not None:
        matches = [i for i in matches if records[i]["cite"] == cite]
        if not matches:
            raise Stop("UNKNOWN_OR_FOREIGN_CITATION_HANDLE", exit_code=2, file=file, cite=cite)
    if len(matches) != 1:
        raise Stop("AMBIGUOUS_INSPECT_FILE", exit_code=2, file=file)
    index = matches[0]; outcome = packet["outcomes"][index]; source = outcome["source"]
    text = packet["texts"][outcome["textKey"]]
    first, last = source["startLine"], source["endLine"]
    # Match source LF/CRLF lines without treating Unicode separators as new lines.
    pieces = text.split("\n")
    lines = [piece + "\n" for piece in pieces[:-1]] + ([pieces[-1]] if pieces[-1] else [])
    if packet.get("kind") == "NATIVE" and text == "" and first == last == 1:
        lines = [""]  # Preserve the engine's empty-file convention without inventing bytes.
    if (type(first) is not int or type(last) is not int or first < 1
            or last < first or len(lines) != last - first + 1):
        raise Stop("INSPECT_SOURCE_LINES_UNAVAILABLE", exit_code=2, file=file)
    if start_line is None and end_line is None:
        start_line, end_line = first, last
    elif (type(start_line) is not int or type(end_line) is not int
            or not first <= start_line <= end_line <= last):
        raise Stop("INVALID_INSPECT_LINE_RANGE", exit_code=2, file=file,
                   availableStartLine=first, availableEndLine=last)
    selected = "".join(lines[start_line - first:end_line - first + 1])
    return index, start_line, end_line, selected


def inspect_sources(manifest_path, *, files=None, start_line=None, end_line=None, source_union=False):
    """Select a view only after validating the entire original READ closure."""
    _, packet, discovery, _ = load(manifest_path)
    if source_union and files is not None:
        raise Stop("SOURCE_UNION_WITH_FILE_SELECTION_UNSUPPORTED", exit_code=2)
    value = project(packet, discovery, Path(manifest_path).parent, include_reused=True, save=False, source_union=source_union)
    if files is None:
        if start_line is not None or end_line is not None:
            raise Stop("INSPECT_FILE_REQUIRED_FOR_RANGE", exit_code=2)
        return value
    if len(files) != 1:
        raise Stop("SINGLE_INSPECT_FILE_REQUIRED", exit_code=2)
    file = files[0]
    index, start_line, end_line, selected = source_view(packet, value["records"], file, start_line, end_line)
    outcome = packet["outcomes"][index]; source = outcome["source"]
    record = value["records"][index]
    value["records"] = [{**record,
        "location": f'{source["service"]}:{file}:{start_line}-{end_line}',
        "source": source, "sourceRecordDigest": outcome["sourceRecordDigest"]}]
    value["texts"] = {record["text"]: selected}
    value["limitations"] = {record["limits"]: value["limitations"][record["limits"]]}
    value["view"] = {"file": file, "startLine": start_line, "endLine": end_line,
                     "originalEvidenceUnchanged": True}
    value["meaning"] += " Selected lines view the original citation; no new evidence or whole-file coverage."
    return value


def inspect_grouped(manifest_path, request_path):
    """Batch views under exactly equal original evidence identities, not text."""
    def unique_object(pairs):
        if len(pairs) != len({key for key, _ in pairs}):
            raise Stop("INVALID_GROUPED_VIEW_REQUEST", exit_code=2)
        return dict(pairs)
    request_raw = Path(request_path).read_bytes()
    if len(request_raw) > 1024 * 1024:
        raise Stop("GROUPED_VIEW_LIMIT", exit_code=2)
    requests = json.loads(request_raw, object_pairs_hook=unique_object)
    return grouped_sources(manifest_path, requests)


def grouped_sources(manifest_path, requests):
    """Shared retained-proof projection; no acquisition or source rewriting."""
    _, packet, discovery, _ = load(manifest_path)
    value = project(packet, discovery, Path(manifest_path).parent, include_reused=True, save=False)
    if not isinstance(requests, list) or not 0 < len(requests) <= 256:
        raise Stop("INVALID_GROUPED_VIEW_REQUEST", exit_code=2)
    groups, views, identities, fragments, total = [], [], {}, {}, 0
    for number, request in enumerate(requests):
        if (not isinstance(request, dict) or set(request) - {"file", "startLine", "endLine", "cite"}
                or not isinstance(request.get("file"), str) or not request["file"]
                or ("cite" in request and (not isinstance(request["cite"], str) or not request["cite"]))
                or (("startLine" in request or "endLine" in request) and
                    (type(request.get("startLine")) is not int or type(request.get("endLine")) is not int))):
            raise Stop("INVALID_GROUPED_VIEW_REQUEST", exit_code=2, request=number)
        index, first, last, text = source_view(packet, value["records"], request["file"],
            request.get("startLine"), request.get("endLine"), request.get("cite"))
        outcome = packet["outcomes"][index]; source = outcome["source"]; record = value["records"][index]
        # Full validated identity decides equality; compact displayed fields do not.
        identity = encode({"source": source, "sourceRecordDigest": outcome["sourceRecordDigest"]})
        if identity not in identities:
            identities[identity] = len(groups)
            groups.append({"id": "g" + str(len(groups) + 1),
                **{key: source.get(key) for key in ("service", "file", "revision", "authority")},
                "sourceLines": [source["startLine"], source["endLine"]], "citations": [], "fragments": []})
        group = groups[identities[identity]]
        citation = {key: record[key] for key in ("cite", "role", "bodyStatus", "limits", "scope") if key in record}
        if citation not in group["citations"]:
            group["citations"].append(citation)
        key = (identity, first, last)
        if key not in fragments:
            total += len(text.encode("utf-8"))
            if total > 16 * 1024 * 1024:
                raise Stop("GROUPED_VIEW_LIMIT", exit_code=2)
            fragments[key] = len(group["fragments"])
            group["fragments"].append({"startLine": first, "endLine": last, "text": text})
        views.append({"request": number, "group": group["id"], "fragment": fragments[key], "cite": record["cite"]})
    return {"schema": "codeclew-grouped-source-view/1.0", "status": value["status"],
        "manifest": value["manifest"], "groups": groups, "views": views,
        **{key: value[key] for key in ("limitations", "coverageWarnings", "failures", "discoveryQueryComplete")},
        "meaning": "Verbatim views of original citations, not new evidence. Gaps are separate fragments; overlaps remain verbatim; identical ranges share a fragment. Null body unspecified. No new semantic/compiler/runtime/publication approval. Full metadata stays in the original manifest; use inspect/bind/check."}


def saved_grouped(path):
    """Recheck a saved projection against its unchanged original native closure."""
    path = Path(path)
    if path.stat().st_size > 128 * 1024 * 1024:
        raise Stop("GROUPED_DISPLAY_LIMIT", exit_code=2)
    value = json.loads(path.read_bytes())
    if not isinstance(value, dict) or value.get("schema") != "codeclew-grouped-source-view/1.0":
        raise Stop("INVALID_GROUPED_DISPLAY", exit_code=2)
    groups = {g['id']: g for g in value['groups']}
    requests = []
    for view in value['views']:
        group = groups[view['group']]; fragment = group['fragments'][view['fragment']]
        requests.append({'file': group['file'], 'cite': view['cite'],
                         'startLine': fragment['startLine'], 'endLine': fragment['endLine']})
    checked = grouped_sources(Path(value['manifest']['path']), requests)
    if encode(value) != encode(checked):
        raise Stop("GROUPED_DISPLAY_CHANGED", exit_code=2)
    return value


def source_heading(group, fragment, cite):
    return f"{group['file']}:{fragment['startLine']}-{fragment['endLine']} · cite:{cite}"


def prior_body(raw, group, fragment, cite):
    """Match a complete identity-linked block, never a loose text substring."""
    if not all(isinstance(group.get(k), str) and group[k]
               and group[k].upper() not in ('UNKNOWN', 'UNRESOLVED', 'UNSPECIFIED')
               for k in ('service', 'file', 'revision', 'authority')):
        return None
    headers = [source_heading(group, fragment, cite)]
    # Existing compose-original retained whole-file displays omit explicit lines.
    # Its raw cite + full file range resolve to this same validated source identity.
    if [fragment['startLine'], fragment['endLine']] == group['sourceLines']:
        headers.append(f"{group['file']} · cite:{cite}")
    body = fragment['text'].encode('utf-8')
    for heading in headers:
        marker = heading.encode('utf-8') + b'\n'
        start = 0
        while (start := raw.find(marker, start)) >= 0:
            line_start = start; start += len(marker)
            if line_start and raw[line_start-1:line_start] != b'\n':
                continue
            fence_end = raw.find(b'\n', start)
            if fence_end < 0:
                continue
            opening = re.fullmatch(rb'(`{3,})[^`\r\n]*', raw[start:fence_end])
            if not opening:
                continue
            body_start = fence_end + 1; body_end = body_start + len(body)
            separators = [b''] if body.endswith(b'\n') else [b'\n']
            if heading != headers[0] and body.endswith(b'\n'):
                separators.append(b'\n')  # Existing whole-source Markdown adds a fence separator.
            if (raw[body_start:body_end] == body and any(
                    raw[body_end:body_end+len(separator+opening[1])] == separator+opening[1]
                    and raw[body_end+len(separator+opening[1]):body_end+len(separator+opening[1])+1] in (b'', b'\n')
                    for separator in separators)):
                return (raw[:body_start].count(b'\n') + 1,
                        raw[:body_start].count(b'\n') + max(1, body.count(b'\n') + (not body.endswith(b'\n'))))
    return None


def display_grouped(path, *, against=(), prior_complete=False, reread=False):
    """Plain saved source; opt-in references are a caller premise, not read proof."""
    value = saved_grouped(path)
    prior = []
    if prior_complete and not reread:
        for file in against:
            try:
                raw = Path(file).read_bytes()
            except OSError:
                continue  # Unavailable prior delivery means show complete source.
            if re.search(rb'(?i)(truncated output|\b\d+\s+tokens? truncated\b|output exceeded|\[truncated\])', raw):
                continue
            prior.append((str(file), raw))
    chunks = [f"Retained source. Sidecar: {path}\nNo semantic/runtime proof; null body coverage is unspecified.\n"]
    if not value['discoveryQueryComplete']:
        chunks.append('Gap: discovery is incomplete.\n')
    for warning in value['coverageWarnings']:
        chunks.append('Coverage gap: ' + encode(warning))
    if value['failures']:
        statuses = sorted({str(f.get('status', 'unavailable')) for f in value['failures']})
        chunks.append('Source failures: ' + ', '.join(statuses) + '; full details in sidecar.\n')
    used_prior = {}; shown_limits = set()
    for group in value['groups']:
        for citation in group['citations']:
            # Immutable identity/position fields belong to the sidecar, not the
            # code reading surface. Keep boundary/status/resolution distinctions.
            metadata_keys = {'authority', 'origin', 'scope', 'startByte', 'endByte',
                             'startLine', 'endLine', 'textDigest', 'evidenceDigest', 'recordDigest'}
            limits = [leaf for leaf in value['limitations'][citation['limits']]
                      if leaf.split('=', 1)[0].rsplit('/', 1)[-1] not in metadata_keys]
            if limits and encode(limits) not in shown_limits:
                chunks.append('Source limits: ' + '; '.join(limits) + '\n'); shown_limits.add(encode(limits))
            if citation.get('bodyStatus'):
                chunks.append(f"Body coverage {citation['cite']}: {citation['bodyStatus']}\n")
        for index, fragment in enumerate(group['fragments']):
            cites = list(dict.fromkeys(v['cite'] for v in value['views'] if v['group'] == group['id'] and v['fragment'] == index))
            heading = source_heading(group, fragment, cites[0])
            if len(cites) > 1:
                heading += ' · also ' + ', '.join(cites[1:])
            chunks.append('\n' + heading + '\n')
            match = next(((file, found) for file, raw in prior
                          if (found := prior_body(raw, group, fragment, cites[0]))), None)
            if match:
                file, (first, last) = match
                if file not in used_prior:
                    used_prior[file] = 'prior' + str(len(used_prior) + 1)
                    chunks.append(f"{used_prior[file]}: {file} (caller asserts intact delivery; not proof of reading).\n")
                chunks.append(f"[Same exact body: {used_prior[file]} lines {first}-{last}; --reread shows full text.]\n")
            else:
                text = fragment['text']
                fence = '`' * max(3, max((len(m[0]) + 1 for m in re.finditer(r'`+', text)), default=3))
                chunks.extend([fence + '\n', text, '' if text.endswith('\n') else '\n', fence + '\n'])
    return ''.join(chunks)


def location(markdown, claim):
    text = claim.get("text")
    if not isinstance(text, str) or not text.strip():
        raise Stop("EMPTY_AUTHORED_CLAIM", exit_code=2)
    matches = [m.start() for m in re.finditer(re.escape(text), markdown)]
    occurrence = claim.get("occurrence")
    if occurrence is None:
        if len(matches) != 1:
            raise Stop("CLAIM_LOCATION_MISSING_OR_AMBIGUOUS", exit_code=2, claim=claim.get("id"))
        occurrence = 1
    if type(occurrence) is not int or not 1 <= occurrence <= len(matches):
        raise Stop("CLAIM_LOCATION_MISSING_OR_AMBIGUOUS", exit_code=2)
    start = matches[occurrence - 1]; end = start + len(text)
    return {"startByte": len(markdown[:start].encode()), "endByte": len(markdown[:end].encode()),
            "startLine": markdown[:start].count("\n") + 1, "endLine": markdown[:end].count("\n") + 1}


def mapped_source(entry, mapping):
    if "nativeBinding" in entry:
        result = native_module()["mapped_source"](entry)
    else:
        result = {**entry["source"], "work": entry["work"], "snapshot": entry["scope"]["snapshot"],
            "sourceReference": entry["sourceBinding"]["reference"] if "sourceBinding" in entry else entry["declaration"]["selectedSourceReference"],
            **({"evidenceKind":"RETAINED_SOURCE"} if "sourceBinding" in entry else {"declarationId":entry["declaration"]["id"]}),
            "sourceRecordDigest": entry["sourceRecordDigest"],
            "url": None, "repository": None, "linkMapping": "MISSING"}
    source = entry["source"]; config = mapping.get(source["service"])
    if config is not None:
        if not isinstance(config, dict) or set(config) != {"repository", "urlTemplate"}:
            raise Stop("INVALID_EXPLICIT_LINK_MAPPING", exit_code=2)
        repository, template = config["repository"], config["urlTemplate"]
        if "nativeBinding" in entry and repository != result["bindingRepository"]:
            raise Stop("NATIVE_REPOSITORY_IDENTITY_OVERRIDE", exit_code=2)
        if (not isinstance(repository, str) or urlsplit(repository).scheme not in ("https", "http")
                or not urlsplit(repository).netloc or urlsplit(repository).username is not None
                or not isinstance(template, str)):
            raise Stop("INVALID_EXPLICIT_LINK_MAPPING", exit_code=2)
        try:
            fields = list(Formatter().parse(template))
            if ({field for _, field, _, _ in fields if field is not None}
                    != {"revision", "path", "startLine", "endLine"}
                    or any(spec or conversion for _, field, spec, conversion in fields if field is not None)):
                raise ValueError("Template must bind revision/path/startLine/endLine without expressions")
            url = template.format(revision=quote(source["revision"], safe=""), path=quote(source["file"], safe="/"),
                                  startLine=source["startLine"], endLine=source["endLine"])
        except (KeyError, ValueError, AttributeError, IndexError) as error:
            raise Stop("INVALID_EXPLICIT_LINK_MAPPING", exit_code=2) from error
        if urlsplit(url).scheme not in ("https", "http") or not urlsplit(url).netloc:
            raise Stop("INVALID_EXPLICIT_LINK_MAPPING", exit_code=2)
        result.update(repository=repository, url=url, linkMapping="EXPLICIT_CALLER_MAPPING")
    return result


def compile_map(manifests, markdown_path, claims_path, output, *, mapping=None, previous=None):
    entries, identities = {}, {}
    for path in manifests:
        manifest, _, _, found = load(path)
        tag = manifest["id"].split(":")[1][:12]
        if tag in identities:
            raise Stop("AMBIGUOUS_CITATION_MANIFEST", exit_code=2)
        identities[tag] = manifest["id"]; entries.update(found)
    markdown_bytes = Path(markdown_path).read_bytes(); markdown = markdown_bytes.decode("utf-8")
    request = json.loads(Path(claims_path).read_bytes())
    if not isinstance(request, dict) or set(request) - {"claims", "remove", "replace"}:
        raise Stop("INVALID_AUTHORED_CLAIMS", exit_code=2)
    if not isinstance(request.get("claims"), list) or not request["claims"]:
        raise Stop("EMPTY_AUTHORED_CLAIMS", exit_code=2)
    mapping = {} if mapping is None else json.loads(Path(mapping).read_bytes())
    if not isinstance(mapping, dict):
        raise Stop("INVALID_EXPLICIT_LINK_MAPPING", exit_code=2)
    claims, ids, used = [], set(), set()
    for claim in request["claims"]:
        if (not isinstance(claim, dict) or set(claim) - {"id", "text", "occurrence", "evidence"}
                or not isinstance(claim.get("id"), str) or not claim["id"] or claim["id"] in ids):
            raise Stop("INVALID_OR_DUPLICATE_CLAIM", exit_code=2)
        ids.add(claim["id"]); position = location(markdown, claim)
        handles = claim.get("evidence")
        if (not isinstance(handles, list) or not handles or any(not isinstance(h, str) for h in handles)
                or len(set(handles)) != len(handles)):
            raise Stop("EMPTY_OR_AMBIGUOUS_CLAIM_EVIDENCE", exit_code=2)
        if any(h not in entries for h in handles):
            raise Stop("UNKNOWN_OR_FOREIGN_CITATION_HANDLE", exit_code=2)
        used.update(handles)
        claims.append({**claim, "location": position,
                       "sources": [mapped_source(entries[h], mapping) for h in handles]})
    changes = {k: request.get(k, []) for k in ("remove", "replace")}
    if any(not isinstance(v, list) or any(not isinstance(x, str) for x in v) or len(set(v)) != len(v) for v in changes.values()):
        raise Stop("INVALID_EXPLICIT_REPAIR", exit_code=2)
    if previous is None:
        if any(changes.values()):raise Stop("REPAIR_REQUIRES_PREVIOUS_MAP", exit_code=2)
    else:
        old = json.loads(Path(previous).read_bytes())
        if (old.get("schema") != "codeclew-authored-sources/1.0"
                or old.get("mapDigest") != digest({k: v for k, v in old.items() if k != "mapDigest"})
                or not isinstance(old.get("claims"), list)
                or any(not isinstance(c, dict) or not isinstance(c.get("id"), str) for c in old["claims"])
                or len({c["id"] for c in old["claims"]}) != len(old["claims"])):
            raise Stop("INVALID_PREVIOUS_MAP", exit_code=2)
        old_claims = {c["id"]: c for c in old["claims"]}
        missing = set(old_claims) - ids
        replaced = {c["id"] for c in claims if c["id"] in old_claims and
                    (c["evidence"] != old_claims[c["id"]]["evidence"] or c["sources"] != old_claims[c["id"]]["sources"])}
        if missing != set(changes["remove"]) or replaced != set(changes["replace"]):
            raise Stop("EXPLICIT_CLAIM_REMOVAL_OR_REBIND_REQUIRED", exit_code=2,
                       remove=sorted(missing), replace=sorted(replaced))
    source_links, unverified_links, replacements = [], [], []
    expected = {h: mapped_source(entries[h], mapping)["url"] for h in used}
    for match in LINK.finditer(markdown):
        target = match.group(1)
        if target.startswith("cite:"):
            handle = target[5:]
            if handle not in used:raise Stop("UNBOUND_MARKDOWN_CITATION", exit_code=2)
            url = expected[handle]
            if url is None:raise Stop("SOURCE_LINK_MAPPING_MISSING", exit_code=2, handle=handle)
            replacements.append((match.start(1), match.end(1), url))
            source_links.append({"handle": handle, "url": url})
        elif target in expected.values():
            source_links.append({"url": target})
        elif any(target.startswith(config["repository"].rstrip("/") + "/") for config in mapping.values()):
            raise Stop("SOURCE_LINK_NOT_BOUND_TO_SELECTED_SPAN", exit_code=2, url=target)
        else:unverified_links.append(target)
    # Refuse unsupported citation syntax rather than leaving a broken pseudo-link.
    if len(re.findall(r'cite:', markdown)) != len(replacements):
        raise Stop("UNSUPPORTED_CITATION_LINK_SYNTAX", exit_code=2)
    rendered = markdown
    for start, end, url in reversed(replacements):rendered = rendered[:start] + url + rendered[end:]
    for claim in claims:
        original = claim["text"]
        def expand(match):
            if not match.group(1).startswith("cite:"):return match.group(0)
            start, end = match.start(1) - match.start(), match.end(1) - match.start()
            return match.group(0)[:start] + expected[match.group(1)[5:]] + match.group(0)[end:]
        claim["text"] = LINK.sub(expand, original)
        if original != claim["text"]:claim["authoredText"] = original
        claim["location"] = location(rendered, claim)
    value = {"schema": "codeclew-authored-sources/1.0", "claims": claims,
        "manifests": sorted(identities.values()), "markdownDigest": "sha256:" + hashlib.sha256(rendered.encode()).hexdigest(),
        "semanticTruthAssessed": False, "runtimeVerified": False, "nativePublicationApproval": False}
    value["mapDigest"] = digest(value)
    result = {"schema": "codeclew-authored-citation-check/1.0", "status": "MECHANICALLY_VALID",
        "semanticTruthAssessed": False, "sourceFallbackAllowed": False, "nativeCommands": 0,
        "claims": len(claims), "sourceLinks": source_links, "unverifiedOtherLinks": unverified_links,
        "missingLinkMappings": sorted(h for h in used if expected[h] is None),
        "linkMapping": "COMPLETE" if all(expected.values()) else "UNRESOLVED",
        "inputMarkdownDigest": "sha256:" + hashlib.sha256(markdown_bytes).hexdigest(),
        "markdownBytesUnchanged": not replacements,
        "transformation": "Only inline Markdown cite: targets expanded using explicit mapping; all other bytes retained.",
        "limits": "Mechanical source bindings and literal Markdown locations only; not rendered Markdown/AST validation, entailment, authenticity, fresh runtime or publication approval."}
    output = Path(output); output.mkdir(parents=True, exist_ok=False)
    (output / "index.md").write_bytes(rendered.encode())
    (output / "sources.json").write_text(encode(value), encoding="utf-8")
    (output / "checks.json").write_text(encode(result), encoding="utf-8")
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    native = sub.add_parser("import-native", help="Validate engine-retained SOURCE_NATIVE bytes and save distinct author citations")
    native.add_argument("--input", type=Path, required=True)
    native.add_argument("--output-dir", type=Path, required=True)
    native.add_argument("--source", action="append", help="Select an exact returned SOURCE_NATIVE ID; default all registered files")
    inspect = sub.add_parser("inspect", help="Validate retained evidence and show the full selected text")
    inspect.add_argument("--manifest", type=Path, required=True)
    inspect.add_argument("--source-union", action="store_true", help="Lossless agreeing-overlap view; original evidence unchanged; no file/range selector")
    inspect.add_argument("--file", action="append", help="One exact retained file path; no glob or substring matching")
    inspect.add_argument("--start-line", type=int, help="Inclusive original source line; requires --file and --end-line")
    inspect.add_argument("--end-line", type=int, help="Inclusive original source line; requires --file and --start-line")
    grouped = sub.add_parser("inspect-grouped", help="Show exact retained file/range views under shared original evidence headers")
    grouped.add_argument("--manifest", type=Path, required=True)
    grouped.add_argument("--request", type=Path, required=True, help="JSON array of {file,startLine?,endLine?,cite?}; one manifest")
    display = sub.add_parser("display-grouped", help="Read saved inspect-grouped as plain verbatim source, retaining full sidecars")
    display.add_argument("--input", type=Path, required=True)
    display.add_argument("--against", type=Path, action="append", default=[], help="Prior visible text; exact identity-linked bodies only")
    display.add_argument("--prior-complete", action="store_true", help="Explicit caller premise that --against text arrived intact; not native read proof")
    display.add_argument("--reread", action="store_true", help="Always display full source, even with --against")
    bind = sub.add_parser("bind", help="Generate source map/checks; semantic truth remains unassessed")
    bind.add_argument("--manifest", type=Path, action="append", required=True)
    for name in ("markdown", "claims", "output-dir"):bind.add_argument("--" + name, type=Path, required=True)
    bind.add_argument("--repositories", type=Path, help="Explicit service -> {repository,urlTemplate}; no guessed links")
    bind.add_argument("--previous", type=Path, help="Prior generated sources.json; removal/rebinding requires explicit lists")
    repair = sub.add_parser("repair", help="Apply explicit addressed Markdown replacements, bind and check existing renderer output")
    repair.add_argument("--manifest", type=Path, action="append", required=True)
    repair.add_argument("--base", type=Path, required=True, help="Previous directory containing index.md and sources.json")
    repair.add_argument("--patch", type=Path, required=True)
    repair.add_argument("--renderer-command", type=Path, required=True, help="Caller-trusted JSON argv with separate {input} and {output} arguments")
    repair.add_argument("--repositories", type=Path)
    repair.add_argument("--output-dir", type=Path, required=True)
    check = sub.add_parser("check-repair", help="Recheck preserved addressed repair, retained evidence and rendered artifacts")
    check.add_argument("--repair-directory", type=Path, required=True)
    check.add_argument("--output", type=Path, required=True, help="Fresh check report file; never overwrites")
    for command in (native, inspect, grouped, display, bind, repair, check):
        command.add_argument("--clew", type=Path, help="Caller-trusted supported launcher; required for SOURCE_NATIVE manifests")
        command.add_argument("--root", type=Path, help="Managed root owning the immutable prepared evidenceSet")
    args = parser.parse_args(argv)
    output_existed = getattr(args, "output_dir", None) is not None and args.output_dir.exists()
    try:
        configure_engine(args.clew, args.root)
        if args.action == "import-native":
            value = native_module()["register"](args.input, args.output_dir, NATIVE_ENGINE, args.source)
        elif args.action == "inspect":
            value = inspect_sources(args.manifest, files=args.file,
                                    start_line=args.start_line, end_line=args.end_line, source_union=args.source_union)
        elif args.action == "inspect-grouped":
            value = inspect_grouped(args.manifest, args.request)
        elif args.action == "display-grouped":
            value = display_grouped(args.input, against=args.against, prior_complete=args.prior_complete, reread=args.reread)
            print(value, end="")
            return 0
        elif args.action == "bind":
            value = compile_map(args.manifest, args.markdown, args.claims, args.output_dir,
                                mapping=args.repositories, previous=args.previous)
        else:
            adapter = runpy.run_path(str(Path(__file__).with_name("reader_markdown_repair.py")),
                                    init_globals={"CITATIONS":globals()})
            if args.action == "repair":
                value = adapter["repair"](args.manifest, args.base, args.patch, args.renderer_command,
                                         args.output_dir, mapping=args.repositories)
            else:
                if args.output.exists():raise Stop("CHECK_REPORT_ALREADY_EXISTS", exit_code=2)
                value = adapter["check"](args.repair_directory)
        code = 0
    except (Stop, OSError, ValueError, TypeError, KeyError, IndexError) as error:
        value = {"status": error.status if isinstance(error, Stop) else "INVALID_CITATION_INPUT",
                 "details": error.details if isinstance(error, Stop) else str(error),
                 "sourceFallbackAllowed": False, "semanticTruthAssessed": False}
        code = 2
    if args.action == "repair" and not output_existed and args.output_dir.is_dir():
        (args.output_dir / "result.json").write_text(encode(value), encoding="utf-8")
    if args.action == "check-repair" and not args.output.exists():
        args.output.write_text(encode(value), encoding="utf-8")
    print(encode(value), end="")
    return code


if __name__ == "__main__":raise SystemExit(main())
