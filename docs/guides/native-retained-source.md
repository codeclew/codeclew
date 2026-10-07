# Native documentation from retained source

Draft for the next release. The shared source reader, Kotlin data-state and
constructor wiring additions described here are not included in v0.13.17.

Capture an explicit service scope once, then select exact declaration IDs from
`clew docs context` using the returned Check. `clew docs pages render` produces
offline HTML and MDX from that saved evidence. Rendering needs neither the source
checkout nor a model call. Each render writes a new or empty output directory.

Java, Kotlin, C#, TypeScript and Rust use the same page and source-navigation
pipeline. Their evidence capabilities remain distinct:

| Language evidence | Available native views | Explicit limits |
| --- | --- | --- |
| Java compiler facts | Source structure, calls, guarded data state, bounded constructor/queue wiring | External implementations and runtime effects remain unknown |
| Kotlin compiler facts | Function sources, compiler control flow when retained, exact calls/constructions, source structure | Typed data and constructor storage require the retained facts supplied by the 2.4.10 analyzer; older Checks do not acquire them automatically |
| C# Roslyn facts | Method sources and bounded structure, exact call/construction evidence when retained | Read-only preview; shared data-state and queue-wiring expansion are unavailable |
| TypeScript compiler facts | Function/method sources, bounded structure, exact calls when retained | Shared data-state and queue-wiring expansion are unavailable |
| Rust syntax facts | Function sources and bounded syntax structure | Call targets remain unresolved; syntax does not establish a callee implementation |

For a single function, use its exact declaration ID for both required declaration
slots. Those slot names alone establish no endpoint or worker role. To document
an endpoint and worker together, select their separate IDs. Add an exact
`wiringDeclaration` only when the saved source contains the composition that
constructs them.

```json
[
  {
    "id": "submission",
    "service": "orders",
    "endpointDeclaration": "RETURNED_SUBMIT_DECLARATION_ID",
    "workerDeclaration": "RETURNED_RUN_ONCE_DECLARATION_ID",
    "wiringDeclaration": "RETURNED_ASSEMBLY_DECLARATION_ID",
    "expandSourceCalls": true,
    "expandDataState": true
  }
]
```

Use `expandDataState` only with supported Java or Kotlin compiler evidence.
Kotlin data-state requires `expandSourceCalls: true`. Missing or unsupported
typed input is reported explicitly; it never means that a function has no state
changes. Omit both expansion options for an initial declaration view.

The project's compiler version is distinct from the analyzer version. The core
installation uses the 2.4.10 analyzer for supported baseline projects and records
both versions, the project's language/API settings and the selected engine in
retained evidence. Kotlin 1.9 analysis keeps
`KOTLIN_ANALYSIS_LANGUAGE_UPGRADED_FROM_1_9_TO_2_0` visible. Analysis under this
engine does not establish equivalence to the project's native compiler semantics
or enable mutation outside a separately qualified capability.

The shared transfer engine links exact compiler parameter/local identities,
ordinary Kotlin backing properties or Java fields, writes, guards, call
arguments and conditional return definitions. Named arguments retain source
order and compiler formal slots. Omitted defaults remain unavailable values.
String transformations and external call results remain symbolic; the document
does not execute them. Repeated calls retain separate source occurrences.

A worker may therefore show that `lastState` receives `"rejected"` when a declared
normal return status differs from zero, and `"sent"` on the other source branch.
Those are conditional source writes. They do not prove that a call completes,
that an external gateway accepts the request, or that a deployed worker ran.
Unknown call interference remains a visible limitation.

Kotlin queue wiring uses compiler proof that a primary-constructor parameter
initializes an ordinary backing property, together with exact constructor actual
slots and a straight-line local allocation. It proves that the selected endpoint
and worker receive the same source-declared queue allocation. Identical names or
types, custom accessors, delegates, unsupported initializers, omitted defaults,
ambiguous constructions and reassignment cannot supply that proof.

Constructor dependencies retain their proof for later freshness comparison.
They do not promote constructor bodies into executable data-flow evidence.
Queue identity establishes neither offer success nor scheduling, consumption,
delivery or runtime completion. A missing proof remains an explicit local gap.

To hide an infrastructure endpoint from publication, use `clew docs endpoint
list`, then `exclude` with the returned exact declaration or endpoint ID and an
explicit saved Check. Keep the original selection file. A later `include`
restores its pages and retained explanations. The policy acts on one callable's
registrations in one service and compilation scope; it preserves saved analysis
and historical publications. It is not redaction.

The native manifest describes only the effective selected pages and their
dependencies. An excluded callable may still appear as an examined callee of a
different selected function. Unsupported declarations and malformed requested
inputs are validated before publication filtering; exclusion cannot conceal an
invalid input.

The original Java/Kotlin Pipeline examples are public synthetic fixtures. Their
paired source, byte/citation checks and offline HTML/MDX qualification establish
bounded generator behavior, not coverage of a production service. The installed
Kotlin version matrix must be recorded separately before claiming its release
qualification.
