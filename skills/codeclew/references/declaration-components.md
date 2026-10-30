# Mandatory retained declaration components

Use this route when selected JSON/OpenAPI or PostgreSQL declarations must survive
authoring as factual tables. One author still explains meaning and behavior.
Tables establish retained declarations, not runtime enforcement, successful
defaults, object identity or complete semantic documentation.

## Select and freeze before authoring

Use an admitted source launcher/runtime. Prepare an explicit bounded selection
from already retained sources; do not capture again or fabricate source IDs.
For `SOURCE`, use the exact returned Check/Work source ID and metadata. For
`SOURCE_NATIVE`, copy the real `evidenceSet` ticket and native record metadata
from the supported retained packet. No synthetic native publication is needed.

```json
{"schema":"codeclew-declaration-selection/1.0","snapshot":"SAVED_SNAPSHOT","language":"en","components":[{"id":"payload","title":"Payload declaration","format":"JSON_SCHEMA","source":{"kind":"SOURCE_NATIVE","service":"SERVICE","id":"RETURNED_NATIVE_ID","revision":"RETURNED_REVISION","file":"contracts/payload.json","textDigest":"RETURNED_TEXT_DIGEST","evidenceSet":{"schema":"codeclew-prepared-source-evidence/1.0","reference":{"schema":"COPY_RETURNED_SCHEMA","digest":"COPY_RETURNED_DIGEST","size":0}}},"branchPointer":""}]}
```

The ticket shown above is explanatory notation: copy the **complete real**
`evidenceSet` object, not these placeholders. `SOURCE` omits `evidenceSet` and
uses its actual source ID. Formats are `JSON_SCHEMA`, `OPENAPI` and
`POSTGRES_CREATE_TABLE`; DDL requires an empty `branchPointer`. Select the full
intended scope before the author begins; a smaller pointer is a scope change.
One ID names one component. Repeated/incompatible IDs or duplicate source scopes
are refused. A SQL component preserves all supported CREATE TABLE declarations
in its selected file by default, with separate database/search-path context and
occurrences. An explicit whole-database selection can narrow this before freezing.

```sh
clew docs evidence prepare-declarations --root ROOT --input SELECTION.json > components-ticket.json
clew docs evidence read-declarations --root ROOT --input components-ticket.json
```

Keep the returned ticket and its exact `id` outside authored content. That ID is
the required `--component-set-digest` for every compose/bind/check/finalize stage.
The engine stores an immutable set, replays original SOURCE/SOURCE_NATIVE bindings
and reconstructs the projection. Same-snapshot native replay uses registered
retained bytes; no private store edits, source recapture or Check-source promotion
is involved. Source records, revision/file/digest/range, branch, projection and
unsupported frontiers remain in the checked `components.json` sidecar.

## Explicit SQL database scope

For a confirmed declared service/database relationship, add `sqlScope` to the
SQL component. Select every complete original section of that database, in source
order; do not list preferred tables or rewrite a shortened SQL source. Example
scope shape (replace every source identity and coordinate with returned values):

```json
{"database":"ledger","ranges":[{"coordinateDomain":"SOURCE_RECORD_UTF8_BYTES","byteStart":120,"byteEnd":420}],"reason":"The retained deployment connection and local connection code select ledger.","evidence":[{"kind":"SOURCE","service":"ledger","id":"RETURNED_SOURCE_ID","revision":"RETURNED_REVISION","file":"src/Connection.cs","textDigest":"RETURNED_TEXT_DIGEST"}]}
```

Ranges start at a supported psql `\c`/`\connect` command and end at the next
psql command or original source end. Repeated sections for the same database must
all be included. Coordinates are UTF-8 bytes in the original retained source
record, including any original prefix. Missing/truncated/overlapping ranges,
wrong contexts and an omitted same-database section refuse selection. Up to 64
sections/ranges and eight bounded evidence sources are supported.

The caller supplies the rationale and genuine SOURCE/SOURCE_NATIVE selectors
supporting the relationship. The engine authenticates each evidence source in
the same saved Check; it does not infer semantic ownership from a database name,
missing usage edge or the evidence's existence. Selection, rationale and all
source bindings become part of the immutable expected component set.

The existing projector reads the full original once. Only complete declarations
in the selected database sections enter the human table. Included/excluded
sections retain their original contexts, coordinates and full table projections
in `projection.scope`; the entire source remains in `binding.source`. A preamble
without tables is retained as a preamble, never labelled another database.

Narrowing fails closed if any source frontier remains (including unsupported
constraints, foreign dependencies, unknown commands, lexical errors or budgets),
or any table has unknown context. Use the unchanged full unscoped projection and
its visible frontier in that case. No unresolved declaration is silently dropped
as irrelevant. Declared database separation does not establish absence of runtime
writers, custom type/function behavior or deployment overrides. Reevaluate the
explicit rationale when those inputs change. A changed/narrower scope produces
a different ticket and cannot replace the independently pinned expected set.

## Place components beside real explanations

The author supplies location and prose, never cell values:

```json
{"pages":[{"page":"guide.md","fragments":[{"id":"purpose","text":"Explain the supported purpose here.\n\n","evidence":["REAL_RETURNED_HANDLE"]},{"component":"payload"},{"text":"\n"},{"componentRef":"payload","text":"Payload declaration"}]}]}
```

Every selected component requires exactly one canonical `{component:ID}`.
`{componentRef:ID,text:LABEL}` links to it from any page. Both forms are closed;
adding authored HTML/cells to a component fragment is refused. Existing page
rules still require a genuine bound prose claim on each page; component-only
pages are outside this slice. Place tables beside real explanations rather than
inventing a claim, operation or publication to satisfy a gate.

```sh
python3 -I -S "$CODECLEW_SKILL_DIR/scripts/page_citations.py" compose \
  --clew CLEW --root ROOT --components components-ticket.json \
  --component-set-digest EXACT_RETURNED_ID --input ORIGINAL.json --output-dir NEW_COMPOSE

python3 -I -S "$CODECLEW_SKILL_DIR/scripts/reader_export_check.py" finalize \
  --clew CLEW --root ROOT --components components-ticket.json \
  --component-set-digest EXACT_RETURNED_ID --compose-root NEW_COMPOSE \
  --manifest EACH_CITATION_MANIFEST --renderer-command EXISTING_RENDERER.json \
  --output-dir NEW_FINAL
```

Use the same flags for direct `bind`, `check`, `update-original` or export check.
Do not omit the independently selected ticket/digest from the final gate.
Canonical HTML is generated natively and inserted as a raw HTML block; the
existing trusted Markdown renderer must preserve that block. The checker rejects
a missing/changed native block independently of rerendering the current Markdown.
Whole-component removal, changed rows/types/defaults/frontier, altered bindings,
duplicate placement and a substituted/narrower ticket do not pass. A component
reference is navigation, not a second copy of its facts.

## Supported declaration limits

JSON/OpenAPI uses the existing display projection, without a second parser or
reference resolver. Requiredness is parent/branch conditional and separate from
nullability. Composition and arrays stay explicit; references and unsupported
rules remain frontiers. Bounds are inherited from that projection. Schema
defaults are declared annotations; no default application is inferred.

The PostgreSQL reader supports `CREATE [TEMP|TEMPORARY|UNLOGGED] TABLE
[IF NOT EXISTS] qualified_identifier (...)`, exact declared types and default
expressions, NULL/NOT NULL, named/unnamed column/table primary keys and composite
keys. It handles comments, quoted identifiers/strings, dollar/E strings and
bounded nesting, plus declared psql `\c`/`\connect` and SET search_path context.
Types may be multiword, parameterized or arrays. PK-implied NOT NULL is labelled
separately from an explicit NOT NULL clause. No runtime SQL is executed.

Other statements/constraints (including ALTER, CHECK and foreign keys), ambiguous
keys and budgets produce source-bound frontiers. They are not assertions that
constraints are absent. Multiple database/table sections remain distinct. The
reader-visible partial notice and limits cannot be removed while retaining the
required component. Exact SQL expressions are shown without JSON string quoting.
Limits include 256 KiB/source, 32 components, 4 MiB selected source, 64 tables,
256 columns/table, 32 nesting levels and 32,768 SQL tokens; overflow is explicit.

DDL PK/default/type declarations do not establish live database state. JSON and
SQL do not establish fresh UUID generation, canonical domain meaning, wire/code
identity or process completeness. Explain these from the separately retained
behavior/type/mapping evidence. Technical component preservation does not replace
the subsequent full author and reader acceptance.
