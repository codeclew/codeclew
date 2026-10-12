# Bounded direct value calls

The Java retained value query composes an explicitly closed static-call profile
inside one authenticated compiler index. The emitter records exact resolved JVM
targets and positional argument definitions. It admits source `static` methods
with ordinary arity, called without an effectful receiver expression; varargs,
virtual/interface dispatch, external declarations and unknown bodies do not gain
this transfer. Kotlin call transfer is not qualified by this addition.

`thread_value_calls::expand` is the shared equation-composition API. Its supplied
functions must share every binding field. It validates closed parameter/call
references and uses distinct contexts for each call occurrence. Formal inputs
copy the caller's actual definitions; call results copy the instantiated callee
return. Exact descriptor identities keep overloads separate.

## Summary and query scope

The reusable template holds the computed relevant node and call lists, exact
parameter definitions, complete function equations and a freshness key. The key
binds the complete supplied program, compiler/source/configuration binding,
composition policy, solver and composition implementations, function and selected
target. Templates are cached within the current query; there is no persistent
summary cache yet. `summaryCacheHits` counts reuse of a prepared template, not
compiler executions avoided or measured agent-token savings. Instantiated
contexts retain separate definition identities and original source provenance.

The entry template starts at the requested definition, including local values
not needed by the return. Callee templates start at `result`. Recurrence identity
includes the selected target, so a local-query context cannot substitute for a
missing return context. Unsupported operations anywhere in a callee's model,
or calls outside the closed profile, invalidate its value-only template. Such a
call remains unknown even when the opaque operation is outside return relevance.

This is a normal-value transfer profile, not a runtime purity proof. Original
effect/exception boundaries remain in every composition. Unselected equations
are irrelevant only under the declared value model; omitted effects, class
initialization, nontermination and exceptions are not shown to be harmless.

## Cycles, budgets and evidence

The exact call graph has deterministic strongly connected components. Recursive
backedges reuse an ancestor return context and join recursive actual arguments
with its incoming seeds. This finite context widening is reported as
`VALUE_RECURSIVE_CONTEXT_JOINED`. The existing monotone nullness solver computes
the bounded equation fixed point; an unseeded recursive return stays unknown.
Convergence is not evidence that recursion terminates or that a return is reached.

The retained contour bounds discovery to 256 functions, composition to 32 call
contexts and 4,096 equations. Context/node/discovery cuts retain explicit budget
frontiers. Unknown target or value-only profile gaps retain semantic frontiers.
Subsequent selection and transfer budgets remain independent. A missing query
target returns discovery unknown, not an invalid empty model.

The authenticated loader verifies the same saved generation, configuration and
source closure as the local-value query. It binds every selected callee occurrence
to its original retained bytes and exact method owner. Common index/source
bindings are prepared once; declaration and occurrence lookups do not rescan the
complete fact set per node. Source-coordinate verification scans each selected
file once. The packet materializes selected equations and occurrences with their
context identities, source CAS references and complete expanded model digest.

## Verification scope

Component fixtures distinguish helper calls with null and non-null arguments,
template reuse, local targets, overload owners, foreign bindings, opaque helpers,
recursive SCCs and local-query recurrence contexts. The managed Maven/javac CLI
fixture exercises compiler extraction through a retained generation and the
public query. Its helpers live in a second source file. Overload return values
are deliberately different, so collapsing the string and integer targets cannot
pass by coincidence. Virtual calls remain unknown. No test qualifies distributed
effects, fields, concrete enum-value analysis or full program closure.
