# Connected computation view

## C# source regions

For a new capture exposing `CSHARP_SOURCE_REGIONS`, select returned observation
IDs as `[{"regionId":"RETURNED_ID"}]` and use `scripts/source_regions.py` with
the same `--clew`, `--root`, `--service`, `--snapshot`, `--work`, `--selection`
and `--output-dir` arguments shown below. This is a separately validated Roslyn
source contract; it does not carry Kotlin CFG authority or reconstruct old Work.

The view keeps each original callable once, with reversible short comments for
joins, loop exits, await protection and selected exact call targets. A join means
where control continues **if** the region completes normally. Ordered catches,
types and filters remain in the original code; dispatch and exception completeness
are unknown. Await success and interprocedural completion are not established.
Finally and disposal may be opaque at the affected exit; supported nested bodies
remain visible. Budget gaps retain their source and do not imply normal exit.
Original records, source, bindings, ranges and display edits remain in
`metadata.json`; `measurement.json` records selected body/region/site counts,
delivered bytes and native read time. Bytes are not model tokens.

## Kotlin computation

For an explicitly selected Kotlin computation in an immutable Work captured by a
producer with `codeclew-kotlin-source-structure/1.0`, use
`scripts/computation_view.py`. Older Work is not reconstructed from prose labels.
This is an optional bounded view, not a whole-service authoring prerequisite.

The author view is a compact source reading surface. Keep original code central;
do not replace branches, conditions or failure paths with a summary. Auxiliary
structure should earn its space by locating code, connecting an exact target or
showing included/missing coverage. Use one short legend and stable, unambiguous
labels; prefer familiar source syntax over another notation the author must learn.
Keep repeated inventories, byte ranges, digests, JVM signatures and raw origin
enums in the lookup/metadata sidecar. Surface a consequential gap briefly beside
the affected code; full diagnostics remain available separately. Compactness is
measured including headings, legends and lookup overhead, against the same source
coverage, not obtained by hiding meaningful source distinctions.

Supply a JSON selection array with `declarationId` and, when source choice is
ambiguous, the returned `sourceId` and `sourceReference`. These are native Work
identities, never inferred selectors. Run:

```sh
python3 -I -S scripts/computation_view.py \
  --clew /path/to/clew --root /path/to/disposable-docs-root \
  --service SERVICE --snapshot SNAPSHOT --work WORK \
  --selection selected.json --output-dir new-computation-view
```

The helper reads only the selected dependencies and source through supported Work
APIs. It writes native read receipts in the selected documentation root; it never
captures, starts an author, changes source, publishes or installs a runtime.

`author.txt` defaults to compact retained source: each selected body appears
once. Letter headings identify callables; synthetic `/*→B*/` comments bind an
exact compiler call/return target, and `/*a1*/` names a callback targeted by a
return. The comments are display annotations, never original source or an
execution trace. Normal source syntax already shows conditions, branches,
exceptions, SQL and configuration; the default source view emits no second PSI table.

`metadata.json` retains untouched original source, full compiler records,
source/revision/digest identities, direct parents/body roles, gate origins,
argument/return bindings, unsupported facts and native receipts. `compactAnchors`
and each callable's `displayEdits` resolve every short marker to its exact native
record/pointer and original UTF-8 range; reversing those insertions reconstructs
the original bytes exactly. Byte ranges and JVM signatures stay out of the
compact author text. The older diagnostic projection remains available with
`--view-format connected`.

`direct-source.txt` contains the same selected source regions without the view.
`measurement.json` separates author bytes, local metadata and native read cost.
Use the same explicit tokenizer for both files; bytes do not establish model-token
savings. Acquisition, self-review and later expansions all count in author cost.

Containment does not prove callback invocation/count/order, receiver identity,
exception dispatch, transactions or successful external effects. A declared Kafka
topic establishes only the send boundary. Unselected callees, configuration,
resources, entities and dependencies remain outside this view and must be
acquired separately when the question requires them. Original metadata stays
available; this projection neither relaxes strict dataInput nor repairs CFG.

For author delivery, follow [complete delivery and retained IDs](compose-original.md#complete-delivery-and-retained-ids).
A saved compact view and its source map are available artifacts until their
complete visible carriers arrive. Send large context and provenance lookup
separately, retain exact sidecars, and recover any truncated part. Existing grouped
inspection and ID-based page associations preserve bindings; purposeful rereads,
local materialization and complete final semantic/export review remain allowed.

## Saved structural self-review

After drafting and before READY, the same author can consult an optional review
of already retained metadata, without acquiring source again:

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/computation_view.py" review-saved --input SAVED_METADATA.json --output-dir NEW_REVIEW
```

Stdout and `NEW_REVIEW/review.txt` contain the review; `review.json` retains native
pointers/spans/origins and exact saved inputs. The compact source view is unchanged.
Optionally repeat `--entrypoints NATIVE_THREAD_ENTRY_PAGE.json` for declared trigger
facts and `--context-receipt SAVED_READ.command.json` for native thread frontiers.
The latter reads its original saved request and hashed stdout, never executes the
recorded command. Work/snapshot and owner/source identities must match; receipts
remain caller-trusted provenance, not producer authentication. Missing context
leaves thread closure unasserted. A selected body never closes a retained budget gap.

Use conditions, body/exit references, lexical links and declared configuration to
notice omitted meaning, then return to the retained source. Explain relevant
alternatives/outcomes/configuration in reader language and self-correct before
finalization. Keep the full question/resource inventory and evidence bindings.
Do not paste the technical review into human pages or treat rows as semantic
coverage proof. Same-symbol reads are not value flow; runtime effects, HTTP
contracts and effective deployed configuration remain unasserted.
