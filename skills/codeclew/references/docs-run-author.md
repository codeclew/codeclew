# Prepared page author

The caller supplies one explicit handoff into a fresh inference context. The model,
complete task and admitted source scope stay fixed. Old conversations, costs,
reader pages and notes remain saved; unrelated tool history is not your input.
Work on the current page, then end the turn. Do not start runners, other models,
capture, refresh, export or publication. Use the prepared profile and local ./docs.

## Page loop

1. Read the page materials: complete selected source views, current draft, own
   notes and open decisions. The compact full-scope index includes every required
   page and available view in every service, including uncited source. Requirements
   define required input; draft citations are not a completeness checklist. Inspect
   consequential helpers, contracts and cross-service dependencies when needed.
   Use ./docs show --review-context --page PAGE.md [--fragment ID], then select
   --view SERVICE/REGION. A shared page can use --service and an explicit small
   claims selection through --input. Source views are complete; do not crop away
   branches or infer absence from unread or unavailable evidence. Own notes are
   preserved; --include-notes explicitly adds relevant saved notes to a comparison.
2. Write reader-facing Markdown with ./docs put, or a semantic diagram with
   ./docs sequence. Lead with purpose, inputs and consequences. Explain steps in
   causal order, put errors beside the failing step, and disclose technical detail.
   Keep each independent start, origin, reply/completion boundary and meaningful
   diagram. Link shared detailed phases instead of copying downstream chains.
   Reuse the stable fragmentTarget/routeTarget returned by helpers. Keep evidence
   bindings; never guess a view from a cite or edit managed anchor HTML.
3. Compare the source with the text/diagram, then each claim back with the source.
   Preserve guards, branch/default/parse variants, before/after state, side effects,
   failure/skip paths, outputs, schemas and proven ordering. Distinguish absent
   input from malformed input, producer transformation from consumer predicate,
   declared constraints from actual enforcement, and application calls from their
   external effects. Do not invent retries, rollback, acknowledgement, durability,
   dispatch or order. Asynchronous work and a response need not form a total order.
   Field tables retain exact names, meaning, type/requiredness/nullability, units
   and evidenced aliases. Group representations only with an evidenced mapping.
   Current citations, a valid table or a successful command do not prove meaning.
4. Repair the affected fragments locally. When a change affects another page's
   meaning, inspect that dependency through the index and keep its decision open;
   do not silently drop it or reread every completed page. Shared narratives must
   retain a complete causal path through their linked phases. Keep proof metadata
   out of reader prose. Private notes are optional; do not create a second body
   that merely restates canonical reader text.
5. Set the page-info title/summary before the final page review. After any prose,
   diagram or metadata change, repeat the affected page review before check.
   Obtain ./docs review-page --page PAGE.md, fill explicit claim/requirement/topic
   and discovery judgments, then record with --input FILE. Retain genuine canonical
   sourceNotes references or valid own notes. Null is pending, not approval.
   Independent quality rejection requires fresh judgments even for preserved text;
   source references/locations are hints, not inherited approval. Discovery needs
   an explicit full-file finding list and disposition; an empty registry is not a
   no-additional-starts result. Preserve all source, topic and coverage obligations.
6. Run ./docs check --page PAGE.md and repair named failures. The caller consumes
   this one handoff and records PAGE_COMPLETED or a local failure without launching
   another page. This is not READY or a semantic verdict. The caller still owns
   the complete source/review/route/navigation/export gate and public acceptance.
   If this is the final assigned page and every pending page decision is closed,
   record explicit finish-review decisions before ending; never infer a visual
   review or a global approval from page checks.

For command syntax, review JSON fields, source parts, data registration and route
repair, read only the relevant section of docs-run-reference.md or ./docs --help.
Keep the full request and unresolved scope discoverable; do not restart the generic
installed acquisition workflow or read old conversation/tool logs to reconstruct it.
