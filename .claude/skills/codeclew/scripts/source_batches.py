"""Serialized native SOURCE reference batches; original per-source proofs survive."""
import copy
from pathlib import Path
import runpy

C = runpy.run_path(str(Path(__file__).with_name("citation_evidence.py")))


def prefetch(owner, choices):
    size = getattr(owner.args, "source_batch_size", 1)
    if type(size) is not int or not 1 <= size <= 8:
        raise owner.Stop("INVALID_SOURCE_BATCH_SIZE", exit_code=2)
    if size == 1:
        return
    pending, seen = [], {}
    for row in choices:
        pair = (row["id"], row["reference"])
        if pair in seen:
            if seen[pair].get("revision") != row.get("revision"):
                raise owner.Stop("SOURCE_BATCH_REVISION_CONFLICT", exit_code=2)
            continue
        seen[pair] = row
        key = (owner.work, owner.args.snapshot, owner.args.service, *pair)
        if key not in owner.source_cache and key not in owner.source_batch_proofs:
            pending.append(row)
    groups = [pending[start:start + size] for start in range(0, len(pending), size)]
    owner.prefetch_work_pages([{"references":[r["reference"] for r in group], "projection":"SOURCE_ROWS"} for group in groups])
    for group in groups:
        selection = [{"id":r["id"], "reference":r["reference"]} for r in group]
        pages, names, cursor, cursors = [], [], None, set()
        try:
            while True:
                request = {"references":[r["reference"] for r in group], "projection":"SOURCE_ROWS"}
                if cursor is not None:
                    request["cursor"] = cursor
                page, receipt = owner.work_page("source-batch", request)
                pages.append(page); names.append(receipt["stdout"])
                if type(page.get("total")) is not int or len(pages) > page["total"] or not (page["items"] or page["omitted"]):
                    raise owner.Stop("SOURCE_BATCH_NO_PROGRESS", exit_code=2)
                cursor = page["nextCursor"]
                if cursor is None:
                    break
                if not isinstance(cursor, str) or not cursor or cursor in cursors:
                    raise owner.Stop("NON_PROGRESSING_NATIVE_CONTINUATION", exit_code=2)
                cursors.add(cursor)
            try:
                found, coverage = C["source_batch_pages"](pages, owner.scope(), owner.work, selection)
            except C["Stop"] as error:
                raise owner.Stop(error.status, exit_code=error.exit_code, **error.details) from error
            if owner.coverage and owner.coverage.get("record", {}).get("revision") != coverage["revision"]:
                raise owner.Stop("SOURCE_BATCH_COVERAGE_CHANGED", exit_code=2)
            owner.coverage = {"record":coverage, "authority":pages[0]["authority"], "omitted":[]}
            for row in group:
                source, index, pointer = found[row["reference"]]
                if row.get("revision") is not None and row["revision"] != coverage["revision"]:
                    raise owner.Stop("SOURCE_BATCH_REVISION_CHANGED", exit_code=2)
                key = (owner.work, owner.args.snapshot, owner.args.service, row["id"], row["reference"])
                proof = {"kind":"NATIVE_SOURCE_BATCH_ROW" if source is not None else "NATIVE_SOURCE_BATCH_OMISSION",
                         "choices":selection, "pages":names,
                         "raw":{"stdout":names[index], "pointer":pointer},
                         "recordDigest":C["digest"](source) if source is not None else None}
                owner.source_batch_proofs[key] = copy.deepcopy(proof)
                if source is not None:
                    owner.source_cache[key] = (source, [])
        except owner.Stop as error:
            for row in group:
                owner.source_batch_errors[(owner.work, owner.args.snapshot, owner.args.service, row["id"], row["reference"])] = error


def selected_choices(rows, selections):
    """Only choices already returned by discovery, with unambiguous selection."""
    choices = []
    for selection in selections:
        if not isinstance(selection, dict) or set(selection) - {"declarationId", "sourceId", "sourceReference"}:
            continue
        row = rows.get(selection.get("declarationId"))
        if not row:
            continue
        ids, refs = row.get("sourceIds", []), row.get("sourceReferences", [])
        identity = selection.get("sourceId")
        reference = selection.get("sourceReference")
        if identity is None and len(ids) == 1:identity = ids[0]
        if reference is None and len(refs) == 1:reference = refs[0]
        if isinstance(identity, str) and isinstance(reference, str) and identity in ids and reference in refs:
            choices.append({"id":identity, "reference":reference, "revision":row["revision"]})
    return choices
