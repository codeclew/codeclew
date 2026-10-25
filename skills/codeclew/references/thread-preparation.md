# Prepare complete local threads

Use this route for a requested service inventory or a thread that stops at its
controller. Prepare ordinary service Work against one saved snapshot. No
compiler rerun, model call or publication is implicit in the reads below.

1. Read `{"query":{"kind":"THREAD_INVENTORY"}}` through `docs work read`,
   following every `nextCursor`. Record the selected input files, excluded files,
   capture/parse failures and possible unsupported entry surfaces separately from
   discovered triggers. Unknown inventory in an older snapshot stays unknown.
   Check `omitted` on every page: a null cursor alone does not establish full
   delivery. Follow a larger-Work `recovery` only when it increases `maxBytes`
   and the row fits the supported 49152-byte maximum. An `UNAVAILABLE` recovery,
   or a recovery repeating the current maximum (including older releases), is a
   delivery gap: do not prepare identical Work or loop the same remedy. Preserve
   the omitted kind/ID and continue the selected thread with bounded SYMBOL and
   returned SOURCE/DEPENDENCY part reads. These reads do not complete inventory
   delivery, recapture source or establish a complete explanation.
2. For each requested returned `rootDeclarations` ID, read
   `{"query":{"kind":"THREAD_CONTEXT","symbolContains":"EXACT_DECLARATION_ID"}}`
   through the same Work and finish pagination. This Java/Spring slice reuses
   compiler graph traversal or supplies lexical continuation candidates in pure
   SOURCE_SYNTAX. It does not establish universal framework discovery.
3. Read the complete bodies/events using returned `sourceReferences` and
   `dependencyReferences`. Use `docs work read-part` for a large source. A
   candidate is navigation, even if unique. Inspect alternatives, overloads and
   declared interface methods; preserve receiver/runtime dispatch uncertainty.
4. Explain each BODY, EVENT, DATA_EFFECT, CALL and CANDIDATE obligation from its source.
   Follow controller → service → helper → state change/external boundary.
   Keep validation before a try separate from its catch; keep deferred callback
   work separate from an accepted response. Method names such as save/repository
   do not prove durable storage or delivery. A local unread candidate, budget
   cut or missing body is unfinished work.
5. Save progress with `docs work assess-thread --root ROOT --work WORK_ID
   --input assessment.json`. Write one compact account per returned obligation:
   `references` contains 1..16 Work-local references, including its own completely
   recorded SOURCE; `explanation` needs at least 16 UTF-8 bytes after trimming and
   at most 4096 total UTF-8 bytes. Use that obligation's returned source references,
   rather than one accumulated list of every thread reference. Keep other facts
   in their own accounts; reading the controller alone cannot account for downstream bodies. Resume
   with `docs work thread-status --root ROOT --work WORK_ID`. Follow its
   `nextCursor` with `--cursor`; this status is
   separate from agent-run status and from publication completeness.

Example assessment (use exact IDs/references from this Work):

```json
{
  "schema": "codeclew-thread-assessment/1.0",
  "rootDeclaration": "EXACT_DECLARATION_ID",
  "accounts": [{
    "obligation": "BODY:EXACT_DECLARATION_ID",
    "explanation": "The controller delegates; downstream effects and failures remain to explain.",
    "references": ["WORK_LOCAL_SOURCE_REFERENCE"]
  }]
}
```

Accounts are saved explanations with `meaningReview: UNASSESSED`, not semantic
approval or runtime proof. Inspect remaining open obligations; a gap does not
become a completed thread by being recorded. Existing proposal/review and
publication commands remain the authority for accepted documentation.

## Saved Work continuations without Java THREAD_CONTEXT

For Kotlin or another profile without that Java/Spring route, use the existing
recorded Work queries. After finishing initial/default pages, find a receiver or
helper declaration with a targeted SYMBOL query:

```json
{"query":{"kind":"SYMBOL","symbolContains":"ExampleService"}}
```

Keep that exact query when adding its returned cursor. Inspect receiver
declarations/imports and returned candidate records, then read their actual
`sourceReferences` in a new selection:

```json
{"references":["EXACT_RETURNED_SOURCE_REFERENCE"]}
```

Select the returned source references to request those exact bodies. These reads
can also return retained narrative; use the matching SOURCE text as code evidence
and keep retained prose separate. SOURCE inventory has no `symbolContains` search:
choose exact SOURCE references delivered by SYMBOL/source inventory instead.
A zero result describes that search, not absence from the entire snapshot.

Use the body-to-helper/effects loop above. A source-grounded candidate remains
distinct from a compiler-resolved call edge; reading its body does not prove
runtime dispatch. Preserve that uncertainty while describing the stages, guards,
returns and effects the available body does establish. Do not stop at delegation
or claim a sequence ended at a call when delivered facts include later returns.

## Choose and diagnose a capture profile

Use read-only Java compiler profiles for builds that preserve source. A Maven
plugin that intentionally rewrites sources needs the admitted
`java-17plus-maven-writable-then-seal` profile, which analyzes the transformed,
sealed source view. Do not disable read-only guards to make a transformer pass.
Keep selected modules/test scopes explicit; missing modules are not absent code.

For missing generated getters, constructors or `log` fields, inspect the actual
JDK, Maven compiler configuration and annotation-processor admission. Configure
only the project's admitted `annotationProcessorPaths`/processors; do not infer
Java/Kotlin build order or enable arbitrary processors from a missing symbol.
For `TypeTag UNKNOWN` or a generic unsupported-provider error, retain the JDK
version, compiler arguments and cause chain before choosing a fix. A generic
error alone does not establish unsupported language or project corruption.

For a saved failed Check, use `clew support collect` with the relevant docs root
and cache sharing scope, then follow the [evidence/support route](documentation-evidence.md).
Preserve saved checkpoints and use their supported recovery commands; do not
modify private cache objects or repeat capture as a recovery shortcut.

For requested end-to-end service documentation, apply [boundary/domain closure](boundary-completeness.md): complete available downstream calls, register participants, reconcile service-wide egress, and separate domain concepts from DTO contracts and typed DataFlow. Saved thread obligations alone do not establish this semantic completeness.
