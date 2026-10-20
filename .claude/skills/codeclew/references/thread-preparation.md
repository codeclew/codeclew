# Prepare complete local threads

Use this route for a requested service inventory or a thread that stops at its
controller. Prepare ordinary service Work against one saved snapshot. No
compiler rerun, model call or publication is implicit in the reads below.

1. Read `{"query":{"kind":"THREAD_INVENTORY"}}` through `docs work read`,
   following every `nextCursor`. Record the selected input files, excluded files,
   capture/parse failures and possible unsupported entry surfaces separately from
   discovered triggers. Unknown inventory in an older snapshot stays unknown.
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
   --input assessment.json`. It requires recorded source reads for each account;
   reading the controller alone cannot account for downstream bodies. Resume
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
