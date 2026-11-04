# Complete recorded SOURCE reads

Use one admitted runtime and one immutable Work. Discovery, source delivery,
thread accounts and accepted explanations are different states. These commands
never capture a newer source snapshot or invoke a model.

The installed skill includes closed, self-contained portable schemas:

| Input | Response |
| --- | --- |
| [source-part-request](../schemas/source-part-request.schema.json) | [source-part](../schemas/source-part.schema.json) |
| [work-batch-request](../schemas/work-batch-request.schema.json) | [work-batch](../schemas/work-batch.schema.json) |

They describe transport shapes. Native validation still checks Work membership,
cursor identity, UTF-8 ranges, read receipts and semantic selection constraints.
Do not send a response envelope as a request.

## One large source

Copy a SOURCE reference returned by this Work. Save this input as `part.json`,
replacing `RETURNED_SOURCE_REFERENCE`:

```json
{
  "schema": "codeclew-documentation-source-part-request/1.0",
  "reference": "RETURNED_SOURCE_REFERENCE"
}
```

```sh
clew docs work read-part --root ROOT --work WORK_ID --input part.json
```

A response uses `codeclew-documentation-source-part/1.0`, with `source` metadata,
`text`, byte range, record/fragment digests, receipt and `nextCursor`. It is not
an input. There is no `projection` field on a part request.

When `nextCursor` is non-null, repeat the same command with the same reference
and that exact cursor:

```json
{
  "schema": "codeclew-documentation-source-part-request/1.0",
  "reference": "RETURNED_SOURCE_REFERENCE",
  "cursor": "COPY_NEXT_CURSOR_FROM_THIS_SOURCE_PART"
}
```

Continue through the terminal part, preserving all responses. Each receipt is
bound to this Work, snapshot, source and record digest; a cursor from another
source, Work or API is invalid. Do not invent offsets or splice receipt records.
An empty final fragment is not a shortcut to complete delivery. A native
`SOURCE_PART_NO_PROGRESS` explains a metadata/page-budget limitation; use the
supported repair, without repeatedly preparing identical Work at the maximum.

## Independent SOURCE selections in one call

Replace each placeholder with a SOURCE reference from this Work:

```json
{
  "schema": "codeclew-documentation-work-batch-request/1.0",
  "selections": [
    {"projection": "SOURCE_ROWS", "references": ["FIRST_SOURCE_REFERENCE"]},
    {"projection": "SOURCE_ROWS", "references": ["SECOND_SOURCE_REFERENCE"]}
  ]
}
```

```sh
clew docs work read-batch --root ROOT --work WORK_ID --input batch.json
```

Use 1..64 selections. Each selection accepts up to eight references. `SOURCE_ROWS`
is a selection-level projection, not `query.projection`. It returns source
service coverage and the registered source rows, without expanding narratives.
A SOURCE query such as `{"query":{"kind":"SOURCE","projection":"FILES"}}`
is also supported; it discovers inventory and is not a complete body read.
SOURCE inventory does not accept a nonempty `symbolContains`.

Inspect every `results` element independently:

- `page.nextCursor`: continue only that exact selection with its own cursor.
  For example, add `cursor` beside `projection` and `references` on the first
  selection while keeping the second selection's separate cursor. Do not share
  one cursor across selections, switch queries or transfer Work-local handles.
- `page.omitted`: retain every delivery gap. `nextCursor: null` does not make an
  omitted row delivered. Use the exact returned source reference with `read-part`
  if its body exceeds the ordinary page budget; do not broaden to full inventory.
- `error`: that selection failed. The envelope can succeed while another
  selection fails; fix or report that result rather than claiming all sources
  were read. Malformed nested selection values are retained beside their errors.

Batching loads Work once and records ordinary per-page receipts. It does not
raise the 49152-byte page cap or establish full thread coverage by itself.

## Root identity and partial progress

`THREAD_INVENTORY.rootDeclarations` contains exact declaration IDs. Copy one as
`rootDeclaration`; do not add `BODY:`. `THREAD_CONTEXT` returns obligation IDs,
such as `BODY:EXACT_DECLARATION_ID`. Copy that complete row ID into
`accounts[].obligation` and use its returned source references.

Finish that obligation's own SOURCE before accounting for it. Reading a
controller, the inventory, or another helper does not account for a different
BODY. A source-part sequence can satisfy a complete own-source read; a
source-only account still has `meaningReview: UNASSESSED`.

A controller-only explanation may legitimately be `IN_PROGRESS`. Inspect its
open obligations, `inProgressCount` and `accountedCount` separately. Accounting
for a helper under its own root does not increase the controller entrypoint's
accounted count. Foreign obligations under the controller are rejected. Neither
bounded accounting nor a null cursor implies accepted meaning, runtime closure
or complete published documentation.

## Kotlin and frozen captures

New Kotlin SOURCE captures retain bounded lexical call names for local candidate
navigation. A straightforward controller call to a separately retained helper
can expose its BODY as `NAVIGATION_ONLY`; even a unique name is not binding or
compiler resolution. Overloads stay alternatives, unsupported receiver shapes
stay boundaries, and shadowed callable values do not select unrelated helpers.

Public 0.14.5 Kotlin SOURCE captures did not retain these local call-name hints.
A frozen snapshot keeps that limitation when read by a newer runtime. To gain
new hints, explicitly run SOURCE capture with the new producer and prepare Work
on its returned snapshot; this runs no A build. No read silently recaptures or
rewrites an older Work. If the checkout is unavailable, retain the actionable
boundary; a targeted helper SYMBOL/source read may support a candidate explanation
but cannot be submitted as a foreign controller-thread obligation.
