# Retained JVM value queries

`session values` reads a previously saved Java or Kotlin 2.4.10 generation. It does not run a
compiler, capture the current checkout, invoke an author, or publish a new root.
Create the session and its compiler-backed context through the normal admitted
source launcher before querying values:

```sh
./clew session values --session <SESSION> --compilation :/main \
  --owner 'method:class:example.Service#choose(Ljava/lang/String;)Ljava/lang/String;' \
  --target result --max-nodes 128 --max-transfers 4096
```

The owner is an exact compiler method identity, including the descriptor. The
target is a definition ID in that method's projection; `result` selects its
normal return value. Names and callable families do not select exact overloads.
Qualified same-snapshot static calls compose argument and return dependencies;
see [direct value call summaries](direct-value-call-summaries.md). This call
composition is qualified for Java. Kotlin supports the immutable-local profile
in [K2 extraction](kotlin-value-extraction.md), using an owner such as
`callable:values/Values.choose#jvm:(Ljava/lang/String;)Ljava/lang/String;`.

## Authority and bindings

The public command accepts selectors and budgets, never arbitrary model or
compiler-index JSON. It loads the open session and its immutable generation,
then checks the native Java provider, compilation, configuration, extractor key,
compiler attempt scope, fact identities, canonical CAS closure, source membership
and content digests. Selected occurrence coordinates must match the retained
UTF-8 bytes, UTF-16 positions and original line numbers.

The Kotlin loader also checks the exact active worker and its adapter/store key,
provider semantics, compiler owner and source membership. Metadata 2.3 retains
the descriptor/relation hashes needed to recompute the compiler completion scope;
the scope and completeness receipt must match. Missing hashes reject the query.
Translation authority 3.4 changes the extractor key; older retained metadata is
not silently upgraded. K2 compiler positions account for BOM removal and line
separator normalization, while byte anchors refer to original retained UTF-8.

An available projection carries
`AUTHENTICATED_RETAINED_COMPILER_PROJECTION`. The result's separate
`DECLARED_MODEL_EQUATIONS` authority describes deterministic calculation over
the extracted equations. Neither label establishes complete program semantics:
`programSemanticClosure` remains false and effect/exception frontiers remain.

Correct historical bindings stay readable when the live checkout changes.
An incompatible selected generation, configuration or extractor is different
from an ordinary later edit to the repository. The command never substitutes
live source bytes for the retained snapshot.

## Bounded packets and unknown results

The packet contains the complete model digest and retained definition count,
but materializes only equations and occurrences selected by backward relevance.
It is a projection, not a new closed `ValueModel`. References to omitted
operands are accompanied by budget frontiers in the result. Shared source CAS
references identify the exact retained bytes used by the occurrences.

Missing generations, projections or targets and incompatible extractors return
explicit `UNKNOWN` results. Unsupported backends and transformed Java source
profiles do not gain this operation through registry membership. Kotlin workers
other than 2.4.10 and transformed Kotlin inputs are unqualified. Corrupt
selected source/fact bindings reject the query; resource limits remain explicit
errors. A budget cut cannot expose a partial fixpoint as a proved value.

Retained fact/source closure is limited to 64 MiB and 262,144 facts; stdout is
limited to 64 KiB. Reducing `--max-nodes` reduces materialized equations and
occurrences. Fixed binding/frontier overhead may still exceed the packet limit;
in that case select a smaller compilation scope rather than assuming a smaller
transfer budget can reduce those bytes.

## Qualification

Focused component tests cover foreign generation/configuration, incompatible
extractors, unavailable closure, changed source digests and inconsistent source
coordinates. The managed Java/Maven CLI fixture executes javac, retains the
result through normal session/context creation, checks deterministic queries,
budget cuts and unavailable owners/targets, and queries with no analysis tools
on PATH. Long owner/path fixtures check that irrelevant definitions do not leak
into a bounded packet. The source-only native javac fixture qualifies extraction
separately. Other workers, transformed-source profiles, fields, calls and
effect/exception semantics require their own qualification, apart from the
closed static-call profile documented above.

The native Kotlin source-launcher fixture passes for the immutable-local 2.4.10
profile, including original BOM/CRLF and historical bytes. It does not qualify
Kotlin call composition, enum-value analysis, field writes or branch transfer.
