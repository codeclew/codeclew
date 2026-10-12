# Linked process preview

`./clew docs process linked-preview --input selection.json --output-dir preview`
creates a local preview that links selected service pages. It makes no provider
calls or source captures and generates no internal explanation for a service.
The page uses the operation reader's native step containers and offline styles.

This is a bounded demonstration adapter. Durable process composition continues
to use `process.thread`; the preview does not replace its evidence or publication
pipeline. A service fragment should first exist as a native rendered process.

The closed `codeclew-linked-process-preview/1.0` input contains a title, requested
scope, two to 32 process references and bounded nested steps. Steps contain only
an ID, a process reference, an optional existing page anchor and children. They
cannot contain a newly authored body. Each process names its service and exact
source revision, with one of these bindings:

- `CATALOGUE_PAGE`: existing export directory, byte digests of `export.json` and
  the process page, its `processes/*.html` route, operation and operation digest.
  The selected export record must retain the matching accepted service revision.
- `NATIVE_SCENARIO_PAGE`: existing generated directory, publication byte digest,
  scenario ID and scenario JSON byte digest. Its publication must bind both JSON
  and HTML, and the source-bound thread root must match the selected service and
  revision. This binding preserves source-only authority; it does not invent an
  accepted meaning review.

The output is a new directory with `index.html` and `manifest.json`. Existing
service pages remain unchanged. Local references use encoded absolute file URLs;
this adapter is not a public website export. Missing or duplicate anchors,
changed bytes, foreign operation/revision bindings and symbolic page files fail
before output is published. Declared nesting does not prove runtime routing,
delivery, persistence or completion of side effects.
