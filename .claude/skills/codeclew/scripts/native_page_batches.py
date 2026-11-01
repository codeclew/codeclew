"""Amortize launcher/immutable Work loading; retain exact native bulk stdout."""


def prefetch(owner, selections):
    size = getattr(owner.args, "native_batch_size", 1)
    if size == 1:
        return
    if type(size) is not int or not 1 <= size <= 64:
        raise owner.Stop("INVALID_NATIVE_BATCH_SIZE", exit_code=2)
    encode = owner.encode
    pending, seen, progress = [], set(), {}
    for selection in selections:
        key = encode(selection)
        if key not in seen and key not in owner.page_cache and key not in owner.page_errors:
            seen.add(key); pending.append(selection)
    while pending:
        group, pending = pending[:size], pending[size:]
        value, receipt = owner.call("work-batch", ["docs", "work", "read-batch", "--root", owner.args.root,
            "--work", owner.work], input_value={"schema":"codeclew-documentation-work-batch-request/1.0", "selections":group})
        results = value.get("results")
        if (value.get("schema") != "codeclew-documentation-work-batch/1.0"
                or value.get("work") != owner.work or value.get("snapshot") != owner.args.snapshot
                or value.get("receiptDigest") != owner.digest({k:v for k,v in value.items() if k != "receiptDigest"})
                or not isinstance(results, list) or len(results) != len(group)):
            raise owner.Stop("NATIVE_BATCH_ENVELOPE_MISMATCH", exit_code=2, rawReceipt=receipt)
        for index, (selection, result) in enumerate(zip(group, results)):
            key = encode(selection)
            if not isinstance(result, dict) or result.get("selection") != selection:
                raise owner.Stop("NATIVE_BATCH_SELECTION_CHANGED", exit_code=2, rawReceipt=receipt)
            if set(result) == {"selection", "error"} and isinstance(result["error"], dict):
                owner.page_errors[key] = owner.Stop("NATIVE_SELECTION_ERROR", exit_code=2, nativeOutput=result)
                continue
            if set(result) != {"selection", "page"}:
                raise owner.Stop("NATIVE_BATCH_RESULT_MALFORMED", exit_code=2, rawReceipt=receipt)
            page = result["page"]
            if not isinstance(page, dict):
                raise owner.Stop("NATIVE_BATCH_RESULT_MALFORMED", exit_code=2, rawReceipt=receipt)
            owner.validate_page(page, "codeclew-documentation-work-page/1.0")
            if page.get("work") != owner.work:
                raise owner.Stop("NATIVE_WORK_MISMATCH", exit_code=2)
            chain = encode({k:v for k,v in selection.items() if k != "cursor"})
            total, count = progress.get(chain, (page.get("total"), 0))
            count += len(page["items"]) + len(page["omitted"])
            if (type(total) is not int or total < 0 or page.get("total") != total or count > total
                    or page["nextCursor"] is not None and not (page["items"] or page["omitted"])):
                raise owner.Stop("NON_PROGRESSING_NATIVE_CONTINUATION", exit_code=2)
            progress[chain] = (total, count)
            owner.page_cache[key] = (page, {"stdout":receipt["stdout"] + "#" + str(index)})
            cursor = page["nextCursor"]
            if cursor is not None:
                follow = {**selection, "cursor":cursor}; follow_key = encode(follow)
                if follow_key in seen:
                    raise owner.Stop("NON_PROGRESSING_NATIVE_CONTINUATION", exit_code=2)
                seen.add(follow_key); pending.append(follow)
