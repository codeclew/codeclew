#!/usr/bin/env python3
"""Bind exact locations in already-authored Markdown using retained native proof.

No prose generation, source acquisition, semantic assessment or renderer changes.
Each page keeps the existing source-map contract; the index qualifies page IDs.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import runpy
import shutil
import tempfile

C = runpy.run_path(str(Path(__file__).with_name("author_citations.py")))
D = runpy.run_path(str(Path(__file__).with_name("declaration_components.py")))
Stop, digest, encode = (C[k] for k in ("Stop", "digest", "encode"))
REQUEST = "codeclew-page-claims/1.0"
INDEX = "codeclew-authored-page-index/1.0"
MAX_FILE = 2 * 1024 * 1024
MAX_MAP = 32 * 1024 * 1024
MAX_TOTAL = 16 * 1024 * 1024
MAX_PAGES = 64
MAX_CLAIMS = 4096


def component_call(name, *args):
    try:
        return D[name](*args)
    except D["ComponentError"] as error:
        raise Stop(error.status, exit_code=2, **error.details) from error


def configure_engine(clew=None, root=None, components=None, component_set_digest=None):
    C["configure_engine"](clew, root)
    component_call("configure", clew, root, components, component_set_digest)


def require(condition, status, **details):
    if not condition:
        raise Stop(status, exit_code=2, **details)


def sha(raw):
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def page_name(value):
    require(isinstance(value, str) and 0 < len(value) <= 512, "INVALID_PAGE_PATH")
    path = PurePosixPath(value)
    require(not path.is_absolute() and str(path) == value and "\\" not in value
            and not any(p in (".", "..") for p in path.parts)
            and not any(ord(c) < 32 for c in value) and path.suffix == ".md", "INVALID_PAGE_PATH")
    return value


def read(root, relative, maximum=MAX_FILE):
    root = Path(root)
    require(not root.is_symlink(), "UNSAFE_PAGE_ARTIFACT")
    relative = PurePosixPath(relative)
    require(not relative.is_absolute() and relative.parts
            and not any(p in (".", "..") for p in relative.parts), "UNSAFE_PAGE_ARTIFACT")
    path = root
    for part in relative.parts:
        path = path / part
        require(not path.is_symlink(), "UNSAFE_PAGE_ARTIFACT")
    require(path.is_file() and path.stat().st_size <= maximum, "PAGE_ARTIFACT_UNAVAILABLE_OR_OVERSIZED", artifact=str(relative))
    return path.read_bytes()


def json_file(path):
    path = Path(path)
    return json.loads(read(path.parent, path.name))


def line_ranges(raw):
    ends = [i + 1 for i, byte in enumerate(raw) if byte == 10]
    if raw and (not ends or ends[-1] != len(raw)):
        ends.append(len(raw))
    return list(zip([0, *ends[:-1]], ends))


def locations(root, pages, output):
    require(0 < len(pages) <= MAX_PAGES and len(set(pages)) == len(pages), "INVALID_PAGE_SELECTION")
    found, total = [], 0
    for name in pages:
        name = page_name(name); raw = read(root, name); raw.decode("utf-8"); total += len(raw)
        require(total <= MAX_TOTAL, "PAGE_BYTES_LIMIT")
        ranges = line_ranges(raw)
        require(len(ranges) <= 16384, "LOCATION_METADATA_LIMIT")
        found.append({"page": name, "markdownDigest": sha(raw), "bytes": len(raw),
                      "lines": [{"line": i + 1, "startByte": a, "endByte": b}
                                for i, (a, b) in enumerate(ranges)]})
    value = {"schema": "codeclew-page-locations/1.0", "pages": found}
    raw = encode(value).encode(); require(len(raw) <= MAX_FILE, "LOCATION_METADATA_LIMIT")
    with Path(output).open("xb") as stream:
        stream.write(raw)
    return {"status": "PAGE_LOCATIONS_SAVED", "pages": len(found), "output": str(output)}


def catalogue(manifests):
    require(0 < len(manifests) <= MAX_PAGES, "INVALID_MANIFEST_SELECTION")
    identities, handles, tags = {}, {}, set()
    for path in manifests:
        manifest, _, _, entries = C["load"](path)
        tag = manifest["id"].split(":")[1][:12]
        require(tag not in tags, "AMBIGUOUS_CITATION_MANIFEST")
        tags.add(tag); identities[manifest["id"]] = Path(path)
        handles.update({handle: manifest["id"] for handle in entries})
    return identities, handles


def extract(raw, spec):
    require(isinstance(spec, dict) and not set(spec) - {"page", "markdownDigest", "claims", "remove", "replace"},
            "INVALID_PAGE_CLAIMS")
    page_name(spec.get("page"))
    require(spec.get("markdownDigest") == sha(raw), "STALE_PAGE_DIGEST", page=spec["page"])
    for field in ("remove", "replace"):
        values = spec.get(field, [])
        require(isinstance(values, list) and all(isinstance(v, str) for v in values)
                and len(set(values)) == len(values), "INVALID_EXPLICIT_REPAIR")
    markdown = raw.decode("utf-8")
    selected = spec.get("claims")
    require(isinstance(selected, list) and 0 < len(selected) <= 1024, "INVALID_PAGE_CLAIMS")
    claims, ranges, ids, selected_bytes = [], [], set(), 0
    lines = line_ranges(raw)
    for item in selected:
        require(isinstance(item, dict) and not set(item) - {
            "id", "evidence", "startByte", "endByte", "startLine", "endLine", "textDigest"
        }, "INVALID_LOCATION_CLAIM")
        identity = item.get("id")
        require(isinstance(identity, str) and 0 < len(identity) <= 512 and identity not in ids,
                "INVALID_OR_DUPLICATE_PAGE_CLAIM")
        ids.add(identity)
        byte_mode = "startByte" in item or "endByte" in item
        line_mode = "startLine" in item or "endLine" in item
        require(byte_mode != line_mode, "AMBIGUOUS_OR_MISSING_LOCATION", claim=identity)
        if byte_mode:
            a, b = item.get("startByte"), item.get("endByte")
            require(type(a) is int and type(b) is int and 0 <= a < b <= len(raw), "INVALID_BYTE_RANGE")
        else:
            first, last = item.get("startLine"), item.get("endLine")
            require(type(first) is int and type(last) is int and 1 <= first <= last <= len(lines), "INVALID_LINE_RANGE")
            a, b = lines[first - 1][0], lines[last - 1][1]
        require(all(not (0 < pos < len(raw) and raw[pos - 1:pos + 1] == b"\r\n") for pos in (a, b)),
                "SELECTION_SPLITS_CRLF")
        try:
            prefix, text = raw[:a].decode("utf-8"), raw[a:b].decode("utf-8")
        except UnicodeDecodeError as error:
            raise Stop("INVALID_UTF8_SELECTION", exit_code=2) from error
        require(text.strip(), "EMPTY_LOCATION_CLAIM")
        require("textDigest" not in item or item["textDigest"] == sha(raw[a:b]), "SELECTION_DIGEST_MISMATCH")
        matches = [match.start() for match in re.finditer(re.escape(text), markdown)]
        require(len(prefix) in matches, "LOCATION_NOT_REPRESENTABLE_BY_LITERAL_CONTRACT")
        claims.append({"id": identity, "text": text, "occurrence": matches.index(len(prefix)) + 1,
                       "evidence": item.get("evidence")})
        ranges.append((a, b))
        selected_bytes += b - a
        require(selected_bytes <= MAX_TOTAL, "SELECTED_BYTES_LIMIT")
    return claims, ranges


def compile_page(raw, spec, cat, mapping, temporary, previous=None, manifest_ids=None, replay=False):
    claims, ranges = extract(raw, spec)
    identities, handles = cat
    selected = set()
    for claim in claims:
        evidence = claim["evidence"]
        require(isinstance(evidence, list) and 0 < len(evidence) <= 64
                and all(isinstance(h, str) and h in handles for h in evidence),
                "UNKNOWN_OR_FOREIGN_CITATION_HANDLE")
        selected.update(handles[h] for h in evidence)
    if manifest_ids is not None:
        require(isinstance(manifest_ids, list) and len(set(manifest_ids)) == len(manifest_ids)
                and selected <= set(manifest_ids) <= identities.keys(), "PAGE_MANIFEST_CLOSURE_MISMATCH")
        selected = set(manifest_ids)
    temporary.mkdir(parents=True)
    markdown = temporary / "authored.md"; markdown.write_bytes(raw)
    request = {"claims": claims}
    if previous is not None:
        request.update({k: spec.get(k, []) for k in ("remove", "replace")})
    elif not replay:
        require(not spec.get("remove") and not spec.get("replace"), "REPAIR_REQUIRES_PREVIOUS_MAP")
    claims_path = temporary / "claims.json"; claims_path.write_text(encode(request), encoding="utf-8")
    result = temporary / "bound"
    C["compile_map"]([identities[i] for i in sorted(selected)], markdown, claims_path, result,
                     mapping=mapping, previous=previous)
    body = (result / "index.md").read_bytes()
    require(len(body) <= MAX_FILE and (result / "sources.json").stat().st_size <= MAX_MAP, "BOUND_PAGE_BYTES_LIMIT")
    mapped = json.loads((result / "sources.json").read_bytes())
    urls = {h: source["url"] for claim in mapped["claims"] for h, source in zip(claim["evidence"], claim["sources"])}
    replacements = []
    original = raw.decode("utf-8")
    for match in C["LINK"].finditer(original):
        if match.group(1).startswith("cite:"):
            a = len(original[:match.start(1)].encode()); b = len(original[:match.end(1)].encode())
            replacements.append((a, b, len(urls[match.group(1)[5:]].encode()) - (b - a)))
    for claim, (a, b) in zip(mapped["claims"], ranges):
        require(all(not (left < pos < right) for left, right, _ in replacements for pos in (a, b)),
                "SELECTION_INTERSECTS_CITATION_TARGET")
        start = a + sum(delta for _, right, delta in replacements if right <= a)
        end = b + sum(delta for _, right, delta in replacements if right <= b)
        require(claim["location"]["startByte"] == start and claim["location"]["endByte"] == end
                and body[start:end].decode("utf-8") == claim["text"], "BOUND_SELECTION_LOCATION_CHANGED")
    return {"markdown": body, "map": (result / "sources.json").read_bytes(),
            "checks": (result / "checks.json").read_bytes(), "value": mapped,
            "selectedBytes": sum(b - a for a, b in ranges)}


def paths(page, expanded=False):
    return {"markdown": "markdown/" + page, "inputMarkdown": ("inputs/" if expanded else "markdown/") + page,
            "map": "maps/" + page + ".json", "selection": "selectors/" + page + ".json",
            "checks": "checks/" + page + ".json"}


def original_value(raw):
    def unique_object(pairs):
        require(len(pairs) == len({key for key, _ in pairs}), "INVALID_COMPOSE_INPUT")
        return dict(pairs)
    return json.loads(raw, object_pairs_hook=unique_object)


def compose_pages(request):
    """Shared original validation and byte selection; no evidence acquisition."""
    require(isinstance(request, dict) and set(request) == {"pages"}
            and isinstance(request["pages"], list) and 0 < len(request["pages"]) <= MAX_PAGES, "INVALID_COMPOSE_INPUT")
    pages, specs, folded, total, claims, fragments = {}, [], set(), 0, 0, 0
    canonical = component_call("layout", request["pages"])
    placements = []
    for page_index, page in enumerate(request["pages"]):
        require(isinstance(page, dict) and not set(page) - {"page", "fragments", "remove", "replace"}, "INVALID_COMPOSE_PAGE")
        name = page_name(page.get("page")); folded_name = name.casefold()
        require(":" not in name, "INVALID_PAGE_PATH")  # No Windows drive/stream paths.
        require(folded_name not in folded and not any(
            folded_name.startswith(old + "/") or old.startswith(folded_name + "/") for old in folded), "PAGE_PATH_COLLISION")
        folded.add(folded_name)
        items = page.get("fragments")
        require(isinstance(items, list) and items, "INVALID_COMPOSE_FRAGMENTS")
        raw, selected = bytearray(), []
        for fragment_index, fragment in enumerate(items):
            fragments += 1; require(fragments <= MAX_CLAIMS * 4, "COMPOSE_FRAGMENT_LIMIT")
            if isinstance(fragment, dict) and ("component" in fragment or "componentRef" in fragment):
                text = component_call("fragment", fragment, canonical, name).encode("utf-8")
                raw.extend(b"\n\n")
                start = len(raw); raw.extend(text)
                require(len(raw) <= MAX_FILE, "PAGE_BYTES_LIMIT")
                if "component" in fragment:
                    placements.append({"id": fragment["component"], "page": name,
                                       "startByte": start, "endByte": len(raw), "htmlDigest": sha(text)})
                raw.extend(b"\n\n")
                continue
            require(isinstance(fragment, dict) and not set(fragment) - {"text", "id", "evidence"}
                    and isinstance(fragment.get("text"), str), "INVALID_COMPOSE_FRAGMENT")
            annotated = "id" in fragment or "evidence" in fragment
            if annotated:
                require("id" in fragment and "evidence" in fragment, "UNPAIRED_COMPOSE_ANNOTATION",
                        page=name, fragmentIndex=fragment_index,
                        fragmentPointer=f"/pages/{page_index}/fragments/{fragment_index}",
                        missingFields=[field for field in ("id", "evidence") if field not in fragment],
                        nextAction="Plain headings/navigation: use {text}, omitting both id and evidence. "
                                   "Substantive bound fragments: provide {id,text,evidence} with returned evidence handles.")
                evidence = fragment["evidence"]
                require(isinstance(evidence, list) and evidence
                        and all(isinstance(h, str) and h.strip() for h in evidence)
                        and len(set(evidence)) == len(evidence), "EMPTY_OR_AMBIGUOUS_CLAIM_EVIDENCE")
            start = len(raw); raw.extend(fragment["text"].encode("utf-8"))
            require(len(raw) <= MAX_FILE, "PAGE_BYTES_LIMIT")
            if annotated:
                selected.append({"id": fragment["id"], "evidence": evidence, "startByte": start, "endByte": len(raw)})
        raw = bytes(raw); total += len(raw); claims += len(selected)
        require(total <= MAX_TOTAL and claims <= MAX_CLAIMS, "PAGE_BYTES_LIMIT" if total > MAX_TOTAL else "PAGE_CLAIMS_LIMIT")
        spec = {"page": name, "markdownDigest": sha(raw), "claims": selected,
                **{key: page[key] for key in ("remove", "replace") if key in page}}
        # Reuse exact UTF-8/CRLF, ID, empty-span and literal-occurrence guards.
        extract(raw, spec)
        pages[name] = raw; specs.append(spec)
    generated = {"schema": REQUEST, "pages": specs}
    if component_call("active"):
        generated["components"] = {"binding": component_call("binding"), "placements": placements}
    component_call("validate_markdown", generated.get("components"), pages)
    return pages, generated, claims


def publish_composed(original, pages, generated, output, extra=None):
    output = Path(output)
    require(not output.exists() and not output.is_symlink(), "OUTPUT_ALREADY_EXISTS")
    require(output.parent.is_dir() and not output.parent.is_symlink(), "INVALID_COMPOSE_PARENT")
    # Stage the entire validated result on the destination filesystem. Windows
    # rename refuses every existing target; POSIX replaces our empty reservation.
    with tempfile.TemporaryDirectory(prefix=".clew-compose-", dir=output.parent) as temporary:
        stage = Path(temporary) / "package"; stage.mkdir()
        for name, raw in pages.items():
            path = stage / "markdown" / name; path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("xb") as stream:
                stream.write(raw)
        (stage / "original.json").write_bytes(original)
        (stage / "request.json").write_text(encode(generated), encoding="utf-8")
        for name, raw in (extra or {}).items():
            (stage / name).write_bytes(raw)
        require(all(read(stage / "markdown", name) == raw for name, raw in pages.items()), "COMPOSE_OUTPUT_CHANGED")
        owned = None
        try:
            if os.name != "nt":
                output.mkdir(); owned = output.stat()
                require(not output.is_symlink() and (output.stat().st_dev, output.stat().st_ino) == (owned.st_dev, owned.st_ino),
                        "COMPOSE_OUTPUT_CHANGED")
            # On Windows rename itself atomically refuses a concurrent target.
            # Do not reserve a directory there, or replace/remove someone else's.
            os.rename(stage, output)
        except BaseException:
            if owned is not None:
                try:
                    current = output.lstat()
                    if (current.st_dev, current.st_ino) == (owned.st_dev, owned.st_ino):
                        output.rmdir()  # Nonempty/human-modified directories are retained.
                except OSError:
                    pass
            raise


def compose(input_path, output):
    """Write explicit linear fragments once; native proof remains a bind step."""
    input_path = Path(input_path); original = read(input_path.parent, input_path.name, MAX_MAP)
    pages, generated, claims = compose_pages(original_value(original))
    publish_composed(original, pages, generated, output)
    output = Path(output)
    return {"status": "PAGES_COMPOSED", "pages": len(pages), "claims": claims,
            "markdownRoot": str(output / "markdown"), "request": str(output / "request.json"),
            "original": str(output / "original.json"), "originalSHA256": sha(original)[7:], "nativeProofChecked": False,
            "semanticTruthAssessed": False, "sourceFallbackAllowed": False, "nativeCommands": component_call("native_commands")}


def update_original(input_path, edits_path, output):
    """Splice explicit fragments at base-version indices, never matching prose."""
    input_path, edits_path = Path(input_path), Path(edits_path)
    original = read(input_path.parent, input_path.name, MAX_MAP)
    value = original_value(original); compose_pages(value)  # Validate even removed base fragments.
    edits_raw = read(edits_path.parent, edits_path.name, MAX_MAP)
    request = original_value(edits_raw)
    require(isinstance(request, dict) and set(request) == {"baseSHA256", "edits"}, "INVALID_ORIGINAL_UPDATE")
    require(request["baseSHA256"] == sha(original)[7:], "STALE_ORIGINAL_VERSION")
    edits = request["edits"]
    require(isinstance(edits, list) and 0 < len(edits) <= MAX_CLAIMS * 4, "INVALID_ORIGINAL_EDITS")
    by_page = {page["page"]: page for page in value["pages"]}
    grouped, changed, bindings = {}, [], {}
    for edit in edits:
        require(isinstance(edit, dict) and set(edit) == {"page", "start", "delete", "fragments"}, "INVALID_ORIGINAL_EDIT")
        name = page_name(edit["page"])
        require(name in by_page, "UNKNOWN_ORIGINAL_PAGE")
        start, delete, fragments = (edit[k] for k in ("start", "delete", "fragments"))
        require(type(start) is int and type(delete) is int and 0 <= start <= len(by_page[name]["fragments"])
                and 0 <= delete <= len(by_page[name]["fragments"]) - start
                and isinstance(fragments, list) and (delete > 0 or fragments), "INVALID_ORIGINAL_EDIT")
        grouped.setdefault(name, []).append(edit)
    for name, selected in grouped.items():
        selected.sort(key=lambda e: e["start"])
        require(all(a["start"] < b["start"] and a["start"] + a["delete"] <= b["start"]
                    for a, b in zip(selected, selected[1:])), "OVERLAPPING_ORIGINAL_EDITS")
        page = by_page[name]; old = page["fragments"]
        bindings[name] = {f["id"]: f["evidence"] for f in old if "id" in f}
        current, offset = [], 0
        for edit in selected:
            start, delete, fragments = (edit[k] for k in ("start", "delete", "fragments"))
            current.extend(old[offset:start]); new_start = len(current)
            current.extend(fragments); offset = start + delete
            changed.append({"page": name, "start": start, "delete": delete,
                            "newStart": new_start, "insert": len(fragments)})
        current.extend(old[offset:]); page["fragments"] = current
        # New claims are explicit. Existing IDs/handles cannot silently disappear
        # or rebind; that broader repair uses direct original edits + native bind.
    candidate = encode(value).encode("utf-8")
    require(len(candidate) <= MAX_MAP, "PAGE_ARTIFACT_UNAVAILABLE_OR_OVERSIZED")
    pages, generated, claims = compose_pages(value)
    binding_changes = []
    for name, previous in bindings.items():
        updated = {f["id"]: f["evidence"] for f in by_page[name]["fragments"] if "id" in f}
        for identity, evidence in previous.items():
            if identity not in updated or updated[identity] != evidence:
                binding_changes.append({"page": name, "id": identity, "previousEvidence": evidence,
                                        "requestedEvidence": updated.get(identity)})
    if binding_changes:
        metadata = []
        for name in dict.fromkeys(c["page"] for c in binding_changes):
            page = by_page[name]; changed = [c for c in binding_changes if c["page"] == name]
            metadata.append({"page": name,
                "replace": sorted(set(page.get("replace", [])) | {c["id"] for c in changed if c["requestedEvidence"] is not None}),
                "remove": sorted(set(page.get("remove", [])) | {c["id"] for c in changed if c["requestedEvidence"] is None})})
        raise Stop("ORIGINAL_BINDINGS_CHANGED", exit_code=2, changes=binding_changes,
            recovery={"instruction": "Text-only update preserves evidence. Keep new prose and every newly required handle; edit the original directly and apply explicit page replace/remove metadata, then compose and finalize with the previous checked index and all manifests. Do not remove necessary evidence to make update-original pass.",
                      "pageMetadata": metadata,
                      "compose": "page_citations.py compose --input REVISED_ORIGINAL --output-dir NEW_COMPOSE",
                      "finalize": "reader_export_check.py finalize --compose-root NEW_COMPOSE --manifest EACH_MANIFEST --renderer-command RENDERER --previous-index PREVIOUS_CHECKED_INDEX --output-dir NEW_FINAL"})
    receipt = {"status": "ORIGINAL_UPDATED", "baseSHA256": sha(original)[7:], "originalSHA256": sha(candidate)[7:],
               "editsSHA256": sha(edits_raw)[7:], "changes": changed, "nativeProofChecked": False,
               "semanticTruthAssessed": False}
    publish_composed(candidate, pages, generated, output,
                     {"edits.json": edits_raw, "update.json": encode(receipt).encode("utf-8")})
    output = Path(output)
    return {"status": receipt["status"], "originalSHA256": receipt["originalSHA256"],
            "original": str(output / "original.json"), "markdownRoot": str(output / "markdown"),
            "request": str(output / "request.json"), "receipt": str(output / "update.json"),
            "changes": changed[:16], "omittedChanges": max(0, len(changed) - 16),
            "nativeProofChecked": False, "semanticTruthAssessed": False, "sourceFallbackAllowed": False}


def validate_index(index_path, cat, temporary):
    index_path = Path(index_path); root = index_path.parent; index = json_file(index_path)
    require(isinstance(index, dict) and set(index) - {"components"} == {
        "schema", "pages", "repositories", "repositoriesDigest", "indexDigest"
    } and index["schema"] == INDEX, "INVALID_PAGE_INDEX")
    require(index["indexDigest"] == digest({k: v for k, v in index.items() if k != "indexDigest"}), "INVALID_PAGE_INDEX_DIGEST")
    require(index["repositories"] == "repositories.json", "INVALID_PAGE_INDEX_PATH")
    mapping_raw = read(root, "repositories.json")
    require(sha(mapping_raw) == index["repositoriesDigest"], "PAGE_REPOSITORIES_DIGEST_MISMATCH")
    rows = index["pages"]
    require(isinstance(rows, list) and 0 < len(rows) <= MAX_PAGES, "INVALID_PAGE_INDEX")
    verified, folded, total, count, selected_bytes = {}, set(), 0, 0, 0
    for number, entry in enumerate(rows):
        require(isinstance(entry, dict) and set(entry) == {
            "page", "markdown", "inputMarkdown", "map", "selection", "markdownDigest",
            "inputMarkdownDigest", "mapDigest", "selectionDigest", "claimIds", "checks", "checksDigest"
        }, "INVALID_PAGE_INDEX_ENTRY")
        page = page_name(entry["page"])
        require(page not in verified, "DUPLICATE_INDEX_PAGE")
        require(page.casefold() not in folded, "PAGE_PATH_COLLISION"); folded.add(page.casefold())
        expected = paths(page, entry["inputMarkdown"] != entry["markdown"])
        require(all(entry[k] == v for k, v in expected.items()), "INVALID_PAGE_INDEX_PATH")
        raw = read(root, entry["inputMarkdown"]); body = read(root, entry["markdown"])
        selector_raw = read(root, entry["selection"]); mapped_raw = read(root, entry["map"], MAX_MAP)
        checks_raw = read(root, entry["checks"], MAX_MAP)
        total += len(raw); require(total <= MAX_TOTAL, "PAGE_BYTES_LIMIT")
        require(sha(raw) == entry["inputMarkdownDigest"] and sha(body) == entry["markdownDigest"], "PAGE_MARKDOWN_DIGEST_MISMATCH")
        require(sha(selector_raw) == entry["selectionDigest"], "PAGE_SELECTION_DIGEST_MISMATCH")
        require(sha(checks_raw) == entry["checksDigest"], "PAGE_CHECKS_DIGEST_MISMATCH")
        spec = json.loads(selector_raw); require(isinstance(spec, dict) and spec.get("page") == page, "WRONG_PAGE_SELECTION")
        mapped = json.loads(mapped_raw)
        require(isinstance(mapped, dict) and mapped.get("mapDigest") == entry["mapDigest"]
                and mapped.get("markdownDigest") == entry["markdownDigest"],
                "PAGE_MAP_DIGEST_MISMATCH")
        # Bind checked historical remove/rebind intent. Here validate its shape
        # and reconstruct present bindings, without claiming history approval.
        rebuilt = compile_page(raw, spec, cat, root / "repositories.json", temporary / str(number),
                               manifest_ids=mapped.get("manifests"), replay=True)
        require(rebuilt["map"] == mapped_raw and rebuilt["markdown"] == body, "PAGE_MAP_PROOF_MISMATCH")
        require(rebuilt["checks"] == checks_raw, "PAGE_CHECKS_PROOF_MISMATCH")
        ids = [claim["id"] for claim in rebuilt["value"]["claims"]]
        require(ids == entry["claimIds"], "PAGE_CLAIM_IDS_MISMATCH")
        count += len(ids); require(count <= MAX_CLAIMS, "PAGE_CLAIMS_LIMIT")
        selected_bytes += rebuilt["selectedBytes"]; require(selected_bytes <= MAX_TOTAL, "SELECTED_BYTES_LIMIT")
        verified[page] = {"entry": entry, "input": raw, "markdown": body, "map": mapped_raw,
                          "selector": selector_raw, "checks": rebuilt["checks"], "value": rebuilt["value"],
                          "selectedBytes": rebuilt["selectedBytes"]}
    component_call("validate_markdown", index.get("components"), {name: p["input"] for name, p in verified.items()})
    if "components" in index:
        require(json_file(root / "components.json") == component_call("sidecar", index["components"]), "COMPONENT_SIDECAR_CHANGED")
    return verified, json.loads(mapping_raw)


def bind(manifests, root, request_path, output, *, previous=None, repositories=None):
    output = Path(output); require(not output.exists(), "OUTPUT_ALREADY_EXISTS")
    request = json_file(request_path)
    require(isinstance(request, dict) and set(request) - {"components"} == {"schema", "pages"} and request["schema"] == REQUEST
            and isinstance(request["pages"], list) and len(request["pages"]) <= MAX_PAGES, "INVALID_PAGE_REQUEST")
    cat = catalogue(manifests)
    with tempfile.TemporaryDirectory() as value:
        temporary = Path(value)
        old, old_mapping = validate_index(previous, cat, temporary / "old") if previous else ({}, {})
        chosen = {}
        for spec in request["pages"]:
            require(isinstance(spec, dict), "INVALID_PAGE_CLAIMS")
            name = page_name(spec.get("page")); require(name not in chosen, "DUPLICATE_PAGE_SELECTION"); chosen[name] = spec
        pages = sorted(old.keys() | chosen.keys())
        require(0 < len(pages) <= MAX_PAGES, "INVALID_PAGE_SELECTION")
        require(len({page.casefold() for page in pages}) == len(pages), "PAGE_PATH_COLLISION")
        component_call("validate_markdown", request.get("components"), {name: read(root, name) for name in pages})
        mapping = json_file(repositories) if repositories else old_mapping
        mapping_file = temporary / "repositories.json"; mapping_file.write_text(encode(mapping), encoding="utf-8")
        bundle = temporary / "package"; bundle.mkdir()
        rows, reused, total, claims, selected_bytes, unmapped = [], 0, 0, 0, 0, 0
        for number, name in enumerate(pages):
            raw = read(root, name); total += len(raw); require(total <= MAX_TOTAL, "PAGE_BYTES_LIMIT")
            if name not in chosen:
                old_page = old[name]
                require(raw == old_page["input"], "PAGE_SELECTION_REFRESH_REQUIRED", page=name)
                require(mapping == old_mapping, "PAGE_MAPPING_REFRESH_REQUIRED", page=name)
                result = old_page; selector = old_page["selector"]; reused += 1
            else:
                spec = chosen[name]
                if name not in old:
                    require(not spec.get("remove") and not spec.get("replace"), "REPAIR_REQUIRES_PREVIOUS_MAP")
                result = compile_page(raw, spec, cat, mapping_file, temporary / ("new-" + str(number)),
                                      previous=Path(previous).parent / old[name]["entry"]["map"] if name in old else None)
                selector = encode(spec).encode()
            refs = paths(name, raw != result["markdown"])
            artifacts = {refs["markdown"]: result["markdown"], refs["inputMarkdown"]: raw,
                         refs["map"]: result["map"], refs["selection"]: selector,
                         refs["checks"]: result["checks"]}
            for relative, data in artifacts.items():
                path = bundle / relative; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(data)
            ids = [claim["id"] for claim in result["value"]["claims"]]
            claims += len(ids); require(claims <= MAX_CLAIMS, "PAGE_CLAIMS_LIMIT")
            selected_bytes += result["selectedBytes"]; require(selected_bytes <= MAX_TOTAL, "SELECTED_BYTES_LIMIT")
            unmapped += len(json.loads(result["checks"])["missingLinkMappings"])
            rows.append({"page": name, **refs, "markdownDigest": sha(result["markdown"]), "inputMarkdownDigest": sha(raw),
                         "mapDigest": result["value"]["mapDigest"], "selectionDigest": sha(selector),
                         "checksDigest": sha(result["checks"]), "claimIds": ids})
        mapping_raw = encode(mapping).encode(); (bundle / "repositories.json").write_bytes(mapping_raw)
        index = {"schema": INDEX, "pages": rows, "repositories": "repositories.json", "repositoriesDigest": sha(mapping_raw)}
        if "components" in request:
            index["components"] = request["components"]
            (bundle / "components.json").write_text(encode(component_call("sidecar", index["components"])), encoding="utf-8")
        index["indexDigest"] = digest(index); (bundle / "index.json").write_text(encode(index), encoding="utf-8")
        shutil.copytree(bundle, output)
    return {"status": "PAGE_MAPS_BOUND", "pages": len(rows), "claims": claims, "reusedPages": reused,
            "index": str(output / "index.json"), "missingLinkMappings": unmapped,
            "semanticTruthAssessed": False, "sourceFallbackAllowed": False, "nativeCommands": component_call("native_commands")}


def check(manifests, index):
    cat = catalogue(manifests)
    with tempfile.TemporaryDirectory() as value:
        pages, _ = validate_index(index, cat, Path(value))
    return {"status": "PAGE_MAPS_CHECKED", "pages": len(pages), "claims": sum(len(p["value"]["claims"]) for p in pages.values()),
            "index": str(index), "missingLinkMappings": sum(len(json.loads(p["checks"])["missingLinkMappings"]) for p in pages.values()),
            "semanticTruthAssessed": False, "sourceFallbackAllowed": False, "nativeCommands": component_call("native_commands")}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    locate = sub.add_parser("locations", help="Save bounded line/byte locations and page digests without echoing prose")
    locate.add_argument("--markdown-root", type=Path, required=True)
    locate.add_argument("--page", action="append", required=True)
    locate.add_argument("--output", type=Path, required=True)
    producer = sub.add_parser("compose", help="Write explicit text/evidence fragments into fresh pages and an existing byte request")
    producer.add_argument("--input", type=Path, required=True)
    producer.add_argument("--output-dir", type=Path, required=True)
    updater = sub.add_parser("update-original", help="Update original fragments by base SHA-256 and zero-based fragment indices")
    updater.add_argument("--input", type=Path, required=True)
    updater.add_argument("--edits", type=Path, required=True)
    updater.add_argument("--output-dir", type=Path, required=True)
    binder = sub.add_parser("bind", help="Extract authored text and emit page maps plus a checked index")
    binder.add_argument("--markdown-root", type=Path, required=True)
    binder.add_argument("--request", type=Path, required=True)
    binder.add_argument("--output-dir", type=Path, required=True)
    binder.add_argument("--previous-index", type=Path)
    binder.add_argument("--repositories", type=Path)
    checker = sub.add_parser("check", help="Reconstruct every page map using current retained native evidence")
    checker.add_argument("--index", type=Path, required=True)
    for command in (binder, checker):
        command.add_argument("--manifest", type=Path, action="append", required=True)
        command.add_argument("--clew", type=Path, help="Caller-trusted launcher for SOURCE_NATIVE validation")
        command.add_argument("--root", type=Path, help="Root owning the prepared native evidenceSet")
    for command in (producer, updater, binder, checker):
        if command in (producer, updater):
            command.add_argument("--clew", type=Path)
            command.add_argument("--root", type=Path)
        command.add_argument("--components", type=Path, help="Pre-author native declaration component ticket")
        command.add_argument("--component-set-digest", help="Exact pre-author ticket ID; required with --components")
    args = parser.parse_args(argv)
    try:
        configure_engine(getattr(args, "clew", None), getattr(args, "root", None),
                         getattr(args, "components", None), getattr(args, "component_set_digest", None))
        if args.action == "locations":
            result = locations(args.markdown_root, args.page, args.output)
        elif args.action == "compose":
            result = compose(args.input, args.output_dir)
        elif args.action == "update-original":
            result = update_original(args.input, args.edits, args.output_dir)
        elif args.action == "bind":
            result = bind(args.manifest, args.markdown_root, args.request, args.output_dir,
                          previous=args.previous_index, repositories=args.repositories)
        else:
            result = check(args.manifest, args.index)
        code = 0
    except (Stop, OSError, ValueError, TypeError, KeyError, IndexError) as error:
        result = {"status": error.status if isinstance(error, Stop) else "INVALID_PAGE_INPUT",
                  "details": error.details if isinstance(error, Stop) else str(error), "sourceFallbackAllowed": False,
                  "semanticTruthAssessed": False}
        code = 2
    print(encode(result), end="")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
