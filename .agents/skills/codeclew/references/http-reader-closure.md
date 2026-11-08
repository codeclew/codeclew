# HTTP entries and bounded explanation closure

Use this reference for human service/process pages with HTTP entries. Reuse the
existing compose/bind/finalize route and retained source; this adds no capture,
critic, approval stage or separate completeness report.

## From an endpoint to its thread

Keep an inventory row for every evidenced **method + path**, with links to its
own page and exact handler. Distinct versions/methods remain distinct entries,
even when they invoke one shared service. Use the packaged renderer's existing
service `starts` navigation to reach those pages. A page must explain that entry's
input/version, validation, conditional work, side effects, response and failures;
a title with links to a generic process is not a completed endpoint page.

Start the sequence with the client and that precise HTTP handler. Expand
significant local helpers through the actual external/DB/queue calls. Link to
the shared process at the correct entry branch, with a return link to the
endpoint; do not silently replace the handler with an internal service method.
Describe each external participant locally and link to its integration section.
Reconcile that catalogue with participants across the requested threads.

Use [the packaged sequence format](../examples/markdown-renderer/README.md#authored-sequences)
for the default Markdown preview. It renders a diagram and the same authored
steps as a narrow text trace. Keep each semantic sequence row, its condition and
execution mode evidence-bound in ordinary compose fragments; plain fence/JSON
scaffolding may be unbound. Each row may link to its explained local step.
Source maps retain exact clew bindings; links or schema validation never prove
truth. Put provenance, source excerpts and full original descriptions in
secondary details, outside the short primary explanation.

Only show call/return, `await`, parallel waiting or background execution when
evidenced. A normal call does not prove waiting. `async` means a start without
waiting at that point, not guaranteed completion or delivery. Put explicit
conditions on each affected step, including else/skip and error outcomes; a
condition is not inherited by later rows. Keep local errors beside the step
that fails and explain which effects already occurred and what reaches the
caller. Do not invent collection order, rollback, retry, broker ACK or durability.
For an unordered loop, say it repeats per item; a diagram's vertical placement
must not imply a source-unproved item ordering. Split long sequences at meaningful
boundaries without dropping middle/end steps, returns or deferred work.

## Same-author closure before finalization

Review the actual draft against the entry/body/resource inventory already used:

1. For each requested route, follow handler → meaningful helpers → effects →
   response/errors. Check route/version-specific branches and shared-service
   entry differences. Correct omissions in the original fragments.
2. For every referenced constant, predicate or configuration/feature flag that
   changes the explanation, follow its returned supported refs. Read the missing
   relevant definition/body through the existing inspector/Work read route.
   A previously selected quote is not the whole explanation. Read a flag's key,
   default, provider/wrapper chain and branch consequences when available; a
   predicate's name does not establish its rule. Resolve logical topic/path/key
   values even if the environment-specific host or override is unavailable.
3. Distinguish **available but unread**, **read but unresolved**, and **not retained
   or unavailable**. An available unread definition is unfinished work, not an
   evidence limitation. Recover missing delivery parts before claiming review;
   do not scan private caches or recapture source to manufacture closure.
4. Reconcile HTTP inventory ↔ endpoint pages ↔ handler/thread starts and sequence
   participants ↔ integration catalogue. Check field/default/condition explanations
   in both directions against the selected source. Reuse exact delivered handles;
   do not fetch whole files merely to recover an already known cite.

Keep this bounded to the requested scope and consequential references. No blanket
full reread, global cache scan, mandatory new model or production external critic
is required. Intentional rereads remain allowed. Then run the existing full
compose/bind/export/link checks on the changed output and inspect its actual
desktop/narrow reading states. Mechanical checks do not assess semantic closure.

## Explain objects and interactions

Lead domain pages with business objects, their role and supported actions. Use
English source field identifiers with meanings in the requested language (Russian
for Russian pages). Keep API/DTO/storage representations in a separate section;
a DTO-path inventory is not a domain model. Combine aliases only with evidence
of that identity/mapping, and retain representation-specific fields separately.

Use short numbered actions and local condition/outcome tables instead of dense
method-name paragraphs. Known logical HTTP endpoints, topics, config keys/defaults
and provider calls stay visible even when physical deployment is unknown. Keep
one shared scope legend and only consequential local uncertainty in the primary
text. Preserve original prose and provenance in secondary details; shortening is
authored, never heuristic text stripping.
