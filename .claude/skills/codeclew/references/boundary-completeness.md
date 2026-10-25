# Complete boundaries, domain concepts and DataFlow

Use this route when a requested thread stops at helpers, egress is incomplete,
or a domain overview contains DTO/mapper catalogues. It supplements the existing
Work/read/proposal workflow; it does not replace it with a coordinator.

## Establish source closure before authoring

Build a caller-owned boundary inventory from the complete selected source area,
not just sources cited by old prose. Follow each entrypoint through available
bodies, alternatives, callbacks and effects. Reuse exact Work SOURCE reads and
parts. Record searched coverage, exclusions, missing SDK definitions and bounded
search gaps. A missing runtime bean proof does not erase a source-proven HTTP
call. Candidate membership is not runtime registration or path reachability.

Each inventory row records a stable callsite/interaction ID, logical destination,
source revision/range, guard, thread applicability, request payload, awaitedness,
supported normal/absence/error outcome and unresolved continuations. A service
inventory includes other triggers as well as the selected thread. Aggregate a
destination's callsites without collapsing different guards, endpoints or returns.
Unknown vendor/transport uses a stable logical identity, not disappearance.

For operation selection, follow [retained entrypoint discovery](operation-selection.md).
An inventory SYMBOL/dependency ID is not a Work request's native entrypoint ID;
an empty source scope does not admit the separate `process-graph-v1` profile.

Use `declaration.origin: "agent-proposal"` for external destinations and
interactions authored or inferred by an agent from source, even when a human
supplied that source or requested the documentation. Use `human` only for the
exact declaration actually supplied by a human; cite that declaration separately
from source interpretation. Never change origin to gain traversal or acceptance.
For example, the declaration fragment on an agent-authored record is:

```json
{"origin":"agent-proposal","rationale":"Agent-authored interpretation of selected source; receiver behavior and runtime delivery remain unverified."}
```

Register destinations with `docs external put` and declarations with `docs
interaction put`, using current catalogue `inputDigest`. Recompose retained source
and prepare a fresh Work after structure changes. Select interactions in saved
processes and read delivered `DECLARED_INTERACTION` references. Author additional
participants using `externalService`; implicit service/caller actors stay implicit.
Use directed `declared-boundary` events. SOURCE behavior and declaration/runtime
authority remain distinct. Both sequence actors and egress link to local external
pages. Review all inventory rows against registry → service-wide egress → each
applicable thread, including unresolved boundaries.

`SAVED` confirms catalogue persistence, not source binding. Copy `from.selector`
fields from the exact delivered declaration/Work, including full `parameterTypes`
strings. Preserve historical SOURCE_SYNTAX strings as delivered: a synthetic
`"itemId: Long"` or `"enabled: Boolean"` must not be shortened to `"Long"` or
`"Boolean"` by the author. Do not invent scope, normalize signatures or drop
parameter constraints to make a match. After recomposition, inspect each
interaction's `from` status and its `callSite` separately; repair a mismatched
selector using exact evidence or retain `MISSING`/`AMBIGUOUS` visibly. A
`SOURCE_MATCH` remains source authority, not compiler/runtime proof. An external
receiver's unavailable source is separate from a missing caller selector.

When selected source receives a normal HTTP response, its explicit reverse
`declared-boundary-return` is required inside the normal-call branch, before local
status/body handling. Unknown remote schema does not remove this response or
justify replacing it with a note. HTTP return is separate from local fallback/error
mapping. Pair the response with its request, interaction and reversed endpoints. A broad
catch does not prove remote 404 or timeout. Kafka submission, broker ack and
consumer result are distinct; `declared-boundary-return` is forbidden for Kafka.
An actual reply event needs its own interaction. An unawaited SDK/background call
must not gain a synchronous completion arrow.

For HTTP responses and synchronous logical API ports, follow [explicit returns and failure paths](logical-call-outcomes.md).
Each supported normal local API outcome needs its reverse declaration-only
boundary arrow plus a separate source-bound outcome; notes alone do not complete
that sequence. Keep boolean/void, async dispatch and HTTP/error guards distinct.

The isolated native candidate adds `transport.kind: logical` for SDK/storage calls
whose physical transport is unknown. It requires a registered external identity
and rejects HTTP method/path, topic, address and wire-contract claims. Its explicit
return means only a supported local API/driver outcome; it establishes neither
remote HTTP response nor durability. Official 0.13.53 rejects this kind. Preserve
that refusal on the installed profile; use only an explicitly admitted isolated
candidate for its experimental qualification, never a global runtime substitution.

The conservative semantic repair helper cannot add root participants or change
catalogue structure. Do not hide a missing boundary in a self-call to fit its patch
allowlist. Perform supported catalogue/process edits, new immutable Work and a full
Proposal through `work_session.py`; retain the old Work/Proposal and compare
unaffected components. Then obtain a new full independent meaning review. No
external review sidecar can mint native meaning approval.

## Keep three meanings separate

Domain overview: evidenced business concepts, identity, relationships, supported
rules/lifecycle and explicit unknowns. Register meaningful entity IDs using the
[supported entity record shape and examples](domain-entities.md) with `docs entity
put`; service ownership is not inferred from a DTO name, mapper or save.
Representation DTOs, payload fields and persistence shapes belong in contracts
and details. Review source/business evidence for each domain claim; do not require
an arbitrary number of entities.

Sequence: who calls whom, guards, effects and evidenced outcomes. Dependency
topology alone establishes neither order nor data transformation.

DataFlow: use the existing [typed entity-dataflow view](documentation-views.md).
Register domain inputs, save a separate view with a stable ID, prepare its Work
and author `dataflow.nodes/edges` with explicit representation and edge meanings.
Leave its sequence steps empty. Link it from service/process context. A renamed
dependency map is not DataFlow. Preserve prior useful DTO mappings here or in
representation details without presenting them as domain ownership.

After declaration recomposition, use the [explicit retained-view Work route](retained-view-work.md)
with the returned snapshot. The existing `docs view prepare` shortcut selects
only latest; it cannot select a recomposed snapshot. The generic Work route
avoids recapture when the original checkout is unavailable.

## Reconcile native output and review semantics

`scripts/boundary_coverage.py` compares a separately reviewed expectation manifest
with exported native service JSON and optional exported typed view JSON. It does
not call models, native commands, publish or infer source semantics. Do not derive
the inventory solely from the candidate, or delete expectations when an actor is
missing. `PASS_MECHANICAL_CLOSURE` requires registry, whole-service egress, thread
actors/arrows, reversed request/return consistency and external-call projection.
It can also enforce independently reviewed domain IDs and typed DataFlow links.
Guards, applicability, multiple calls, response attribution, awaitedness and domain
meaning remain mandatory independent semantic review.

Expectation shape (IDs below are synthetic):

```json
{"schema":"codeclew-boundary-expectations/1.0","service":"orders","snapshot":"sha256:EXACT_SAVED_CAPTURE/SIZE","sourceScope":"Selected retained source and explicit SDK gaps","coverageGaps":["Remote HTTP schema and storage implementation unavailable"],"boundaries":[{"id":"risk-check","destination":"risk-api","transport":"http","operations":[{"id":"reserve","response":"explicit"}]},{"id":"save-reservation","destination":"reservation-store","transport":"logical","operations":[{"id":"reserve","response":"explicit"}]}],"domain":{"requiredConcepts":["reservation"],"representationOnly":["reservation-request"]},"dataflowViews":["reservation-lineage"]}
```

Response mode is `explicit`, `unresolved`, or `none`. Select it from independently
reviewed source/contract evidence before checking the candidate. Use `explicit`
for a supported normal synchronous HTTP response or logical API completion,
including void completion. Unknown receiver schema, delivery or durability alone
does not justify `unresolved`: those gaps remain beside the explicit local return.
Use `unresolved` only when available evidence does not establish that response;
the missing arrow then needs precise visible unknown-outcome prose checked by
the reviewer. `none` forbids a synchronous return. Retain coverage gaps on PASS.

For local checking, export the full saved publication with its exact selected
snapshot into a new local directory:

```sh
clew docs render --root DOCS_ROOT --snapshot EXACT_SAVED_SNAPSHOT \
  --publication EXACT_SAVED_PUBLICATION --source-mode embedded \
  --output NEW_FROZEN_EXPORT
```

This supported frozen route does not capture source or change publication history.
Keep the export local. It does not require remote repository mappings or
`--export-subject`; that flag belongs to remote subject selection. A failed remote
subject export is not a blocker for this local full embedded route. Use the
exported `services/SERVICE_ID.json` and applicable `scenarios/VIEW_ID.json`, which
carry `selectedSnapshot`. Bare working `docs/generated` JSON can lack that binding
and is not an interchangeable helper input. Never add `selectedSnapshot` manually
or weaken the helper's snapshot check. The expectation snapshot and every selected
export must match exactly.

```sh
python3 -I -S SKILL/scripts/boundary_coverage.py \
  --expectations reviewed-expectations.json \
  --page NEW_FROZEN_EXPORT/services/orders.json \
  --view NEW_FROZEN_EXPORT/scenarios/reservation-lineage.json --output closure-01
```

Before claiming a complete service/thread result, save the reviewed expectation
JSON and run this helper against the exact selected native export (and required
typed views). Retain its command, inputs and result. A Markdown inventory, native
READY or an unexecuted command is not this check. If it fails, preserve that first
failure and repair the artifact; do not weaken `response: explicit`, remove an
expected boundary or substitute a note to obtain PASS. If the gate cannot run,
report completion as blocked. Independent review must still check normal/error
branch placement, status/body guards and remote/local outcome attribution: helper
PASS only establishes the mechanical checks it reports.

Normal invocation: "Complete this service's boundary/domain documentation."
`debug=true` uses identical decisions, adds `--debug` to the mechanical helper and
saves source coverage, exact candidate/native identities, first model attempts,
all failures and semantic/reader verdicts in a fresh caller-owned directory.
Debug grants no extra model budget or external permissions. Exercise the
[owned adversarial case](../examples/boundary-completeness/README.md) in normal and
debug modes with fresh Luna Max before claiming behavioral success. Mechanical
tests and native READY alone cannot establish completion.
