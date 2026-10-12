# Kotlin property accessor attempts

The Kotlin 2.4.10 documentation producer retains source-bound attempts to read
or assign a property whose backing storage is not qualified. This includes
virtual, overridden and custom accessors. Resolving the static property symbol
does not resolve the accessor implementation that runs.

The closed `codeclew-kotlin-documentation-data/1.2` input adds
`accessorAttempts` and `ACCESSOR` syntax nodes. Each attempt binds the exact
owner, compilation scope, original byte range, static property identity and
retained property declaration. The authority is
`K2_RESOLVED_PROPERTY_SYMBOL_NOT_ACCESSOR_DISPATCH`. Version 1.0 and 1.1 inputs
remain readable without these rows. Ordinary property-storage qualification is
unchanged; extension properties, context receivers and constant-property substitution
remain outside this accessor profile.

The shared `codeclew-source-data-state/1.2` output keeps these attempts separate
from storage definitions. A setter records its argument and source guards
only after receiver and right-hand expression evaluation have been transferred.
A plain assignment does not invoke a getter. A getter returns a symbolic
`ACCESSOR_RESULT` with an unknown result. Both retain per-occurrence normal
completion prerequisites; subsequent attempts are conditional on that completion.
Unsupported expression effects still stop transfer.

Accessor calls invalidate previously known field/property values through the
same interference frontier used for ordinary opaque calls. They never establish
backing storage, receiver aliases, actual dispatch, successful assignment or a
database commit. Repeated property reads are distinct occurrences; the static
property identity alone does not imply equal values.

Source data packets preserve attempt citations and property dependencies. Native
source-call pages show attempts in a separate section with the dispatch and
storage limits. This bounded profile does not execute exceptions or prove
runtime transaction or delivery behavior.

The native Kotlin compiler tests include open properties, throwing overrides,
custom getters, enum setter arguments and original BOM/CRLF/Unicode coordinates.
Shared projection tests reject binding drift and withhold attempts after opaque
receiver or argument evaluation. A separate interference test checks that getter
completion cannot preserve a previously known field value.


Version 1.3 adds separately authenticated literal `String` constants from source
`const val` declarations. These values require the compiler constant flag,
literal kind, exact value and retained declaration; they do not invoke accessors
or establish mutable storage. A Java static call may omit runtime receiver
transfer only when K2 resolves a static Java function and its matching class
qualifier. The call still requires normal completion, and class initialization
remains unknown. Kotlin object calls, including `@JvmStatic`, keep their receiver.

Compiler occurrences nested inside opaque syntax are not promoted to transfer
nodes. The opaque expression remains an unsupported frontier, preserving the
unknown evaluation order and effects.
