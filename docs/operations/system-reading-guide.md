# Evidence-bound system reading guide

A full `docs pages render --publication ... --reading-plan PLAN.json` may add a
system orientation before the frozen overview inventory. In a
`codeclew-question-reading-plans/1.0` collection, add one plan with schema
`codeclew-system-reading-plan/1.0` and subject `system:overview`. It uses the same
bounded title, purpose, sections, blocks, conclusion and exact evidence references
as process reading plans. Block links must resolve to saved service/process
operations; retained claim text digests and source objects are validated.

The view is labelled editorial guidance, not accepted prose or runtime proof.
Original descriptions, source bytes, review states and revisions remain intact.
The native output includes `system-reading-guide.json` with the complete guide
and bound evidence, plus `retained-overview.html` with the original inventory.
Unknown lifecycle or ownership facts must remain unknown; a structural schema or
service name is insufficient to establish them.

The contract reader displays literal `const` and `enum` constraints without
inventing an object type when `type` is absent. Unsupported-schema limits remain
visible. A response-source action locates its exact JSON path in the retained
contract and selects that line span while preserving the full source and digest.
Unsupported, ambiguous or non-JSON locations explicitly fall back to the full
retained source; this is navigation, not additional schema interpretation.
