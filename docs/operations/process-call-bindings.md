# Retained process call bindings

`docs process graph` and `docs process extract` can expose and traverse admitted
K2 `CALL_RELATION` records in addition to existing FLOW calls. The policy is
`retained-k2-process-call-bindings/0.1`. A relation is retained compiler evidence,
not an observed invocation or an exact runtime dispatch receiver.

## Exact association

The existing K2 bridge resolves the caller occurrence against one compiler
function descriptor and validates the target callable ID/JVM descriptor and
argument mapping. The graph consumes that projection; it does not resolve names
from source, choose a same-named overload, infer an implementation of an interface,
or infer an HTTP receiver. The caller association and callee lookup are scoped
by service, compilation and full descriptor identity. Duplicate declarations and
unscoped/foreign targets remain frontiers. A qualified call relation can discover
a callee even without a separate FLOW call. Relations and FLOW observations are
retained separately: a matching target does not establish that they are the same
callsite, so they are not deduplicated or given a fabricated execution order.

`methods[].callBindings` and relation-backed `edges[].callBinding` preserve the
relation ID/digest, selected target, exact call source, parameter mapping and
explicit unknowns. Such edges carry `relationId` and `eventId: null`; no FLOW event
is synthesized. `callBindingCount` is separate from FLOW event count. Extraction
preserves the relation authority instead of replacing it with a generic local
association label. Its implementation digest includes the call-binding module.

## Arguments, receiver and return

Actual source expressions come from validated byte subranges of the retained
call source. Mapping rows stay in source order and keep their exact formal
parameter indices, including named arguments. A value such as `text = lookup(id)`
is an unevaluated expression; mapping it to a formal does not prove its value,
purity, successful evaluation or control position. Both byte boundaries must be
UTF-8 boundaries. Default-omission indices are retained; the actual default
values remain UNKNOWN. Older captures without mapping keep `argumentStatus:
UNKNOWN`, rather than gaining a mapping from an empty array or a guessed arity.

The bridge also retains available receiver selection/type and declared result
type. Receiver object identity, runtime implementation selection, return-value
transfer, caller continuation and exception/effect transfer remain explicitly
unknown. The existing qualified Java static-value-call composition is separate
and continues to handle its closed profile. This slice does not qualify Kotlin
interprocedural value transfer or promise transaction, retry or delivery behavior.

The English/Russian extraction reader exposes these bindings in a collapsed
section with retained source links. Argument text, identities and parameter names
are escaped data. Old pages without call bindings keep their existing reader.
The machine-readable contract is
[process-call-binding.schema.json](../../schemas/documentation/process-call-binding.schema.json).

## Admission and budgets

Projection requires an admitted retained Check. Field labels alone cannot
authenticate arbitrary submitted JSON. The consumer additionally checks normalized
fact digest, descriptor identities, service/scope/caller binding, source revision,
file/range/digests and the shared argument validator. Damaged retained records fail
closed. Index lookup reuses the graph's existing scoped callable index and indexes
call relations once; it does not scan the entire evidence set per call.

Both call relations and FLOW slots consume the extraction event budget; all call
sources participate in the unique source-byte budget before a method is admitted.
Method cuts remain explicit. A source-linked callee may still have incomplete or
unavailable body evidence: following its declaration never closes that frontier.
No source recapture, provider authoring or model call occurs during projection.
