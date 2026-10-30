"""Optional lossless projection of selected retained UTF-8 source occurrences.

No reads, source acquisition, semantic inference or canonical packet mutation.
Only exact identity/coordinates and agreeing overlap permit a shared text region.
"""
from __future__ import annotations

import hashlib


def compact(rows, texts, scope, *, include_reused=False):
    groups, separate = {}, []
    selected_bytes = 0
    for index, (record, outcome) in enumerate(rows):
        source = outcome["source"]
        raw = texts[outcome["textKey"]].encode("utf-8")
        if "sha256:" + hashlib.sha256(raw).hexdigest() != source["textDigest"]:
            raise ValueError("SOURCE_UNION_TEXT_DIGEST_MISMATCH")
        selected_bytes += len(raw)
        item = {"index": index, "record": record, "outcome": outcome, "raw": raw}
        occurrence = source.get("occurrence")
        identity = [scope.get("snapshot"), source.get("service"), source.get("file"), source.get("revision")]
        if not isinstance(occurrence, dict):
            separate.append(([item], "MISSING_COORDINATES")); continue
        identity += [occurrence.get("snapshot"), occurrence.get("blob")]
        if not all(isinstance(value, str) and value for value in identity):
            separate.append(([item], "INCOMPLETE_SOURCE_IDENTITY")); continue
        start, end = occurrence.get("startByte"), occurrence.get("endByte")
        if type(start) is not int or type(end) is not int:
            separate.append(([item], "MISSING_COORDINATES")); continue
        if not 0 <= start <= end or end - start != len(raw):
            separate.append(([item], "INVALID_BYTE_SPAN")); continue
        item.update(start=start, end=end)
        groups.setdefault(tuple(identity), []).append(item)

    regions = []
    for items in groups.values():
        # Stable coordinate order, with larger enclosing spans first at a tie.
        items.sort(key=lambda item: (item["start"], -item["end"], item["index"]))
        component, start, end, body, conflict = [], None, None, b"", False

        def flush():
            if not component: return
            if conflict:
                # Preserve every member, including apparently consistent ones:
                # do not choose an authority within an inconsistent component.
                separate.append((list(component), "CONFLICTING_OVERLAP"))
            else:
                regions.append({"items": list(component), "start": start, "raw": body, "reason": None})

        for item in items:
            if not component or item["start"] >= end:
                flush()
                component, start, end, body, conflict = [item], item["start"], item["end"], item["raw"], False
                continue
            overlap_end = min(end, item["end"])
            offset = item["start"] - start
            overlap = overlap_end - item["start"]
            if body[offset:offset + overlap] != item["raw"][:overlap]: conflict = True
            if item["end"] > end:
                body += item["raw"][end - item["start"]:]
                end = item["end"]
            component.append(item)
        flush()

    for items, reason in separate:
        for item in items:
            regions.append({"items": [item], "start": item.get("start", 0), "raw": item["raw"], "reason": reason})
    regions.sort(key=lambda region: min(item["index"] for item in region["items"]))
    projected, pool, fallbacks = [None] * len(rows), {}, []
    projected_bytes = 0
    for number, region in enumerate(regions, 1):
        key, raw = "u" + str(number), region["raw"]
        projected_bytes += len(raw)
        emit = include_reused or any(not item["outcome"].get("evidenceReused") for item in region["items"])
        if emit: pool[key] = raw.decode("utf-8")
        for item in region["items"]:
            offset = item.get("start", region["start"]) - region["start"] if not region["reason"] else 0
            end = offset + len(item["raw"])
            # Check exact bytes AND code-point boundaries for every original
            # binding. A projection never substitutes a superset as its citation.
            if raw[offset:end] != item["raw"] or raw[offset:end].decode("utf-8") != item["raw"].decode("utf-8"):
                raise ValueError("SOURCE_UNION_RECONSTRUCTION_MISMATCH")
            record = {k: v for k, v in item["record"].items() if k != "retainedText"}
            record.update(text=key, byteSlice={"startByte": offset, "endByte": end})
            if not emit: record["retainedText"] = True
            if region["reason"]: fallbacks.append({"cite": record["cite"], "reason": region["reason"]})
            projected[item["index"]] = record
    canonical_bytes = sum(len(text.encode("utf-8")) for text in texts.values())
    return projected, pool, {
        "schema": "codeclew-author-source-union/1.0", "coordinateUnit": "UTF8_BYTES_HALF_OPEN",
        "metrics": {"selectedBindings": len(rows), "selectedBindingBytes": selected_bytes,
                    "canonicalTextBytes": canonical_bytes,
                    "selectedRegions": len(regions), "projectedUniqueBytes": projected_bytes,
                    "overlapBytesRemoved": selected_bytes - projected_bytes,
                    "canonicalToProjectedByteDelta": canonical_bytes - projected_bytes,
                    "deliveredRegions": len(pool), "deliveredUniqueBytes": sum(len(text.encode("utf-8")) for text in pool.values())},
        "fallbacks": fallbacks,
        "coverage": "Every selected binding reconstructs exactly; selected bytes only, not whole-file completeness.",
        "reconstruct": "UTF-8 encode texts[record.text], slice record.byteSlice [startByte,endByte), then UTF-8 decode. Reused regions remain local; inspect --source-union restores them.",
        "authority": "Citation identities and per-record syntax/compiler limits unchanged; no semantic inference or token-savings claim."}
