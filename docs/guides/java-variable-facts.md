# Java variable facts

The Javac producer emits two additive compiler fact kinds through the existing
immutable generation and documentation evidence snapshots. This contract supplies
storage identity and individual source occurrences; it does not establish value
lineage, receiver aliasing, interprocedural state, runtime order or execution.

`VARIABLE_DECLARATION` describes a `PARAMETER` or `LOCAL_VARIABLE`. Parameters use
`parameter:<exact-callable>/slot/<zero-based-slot>`. Locals use
`local:<exact-callable>/body/<structural-child-ordinals>`. The paths follow Javac's
source tree within that callable, excluding deferred lambda and local-class
bodies. They distinguish sibling declarations with the same name and survive
unrelated file line relocation. Changes to the body structure can change paths.
Field declarations retain their existing `field:<owner>#<name>:<descriptor>` IDs.

`VARIABLE_ACCESS` resolves a Javac `VariableElement` for a field, local or parameter.
Its exact owner, descriptor, enclosing callable and structural occurrence path
are retained. Plain assignment targets are `WRITE`; compound assignments and
increments/decrements are `READ_WRITE`; other occurrences are `READ`. A member
receiver and assignment RHS have separate reads. An array element assignment
reads its array reference and index; it does not write the array variable.
Declaration initializers are `INITIALIZER_DEFINITION`, without a fabricated read
of the declared name. Parameter input and absent local initializer use
`PARAMETER_INPUT` and `UNINITIALIZED`, respectively; these labels describe syntax,
not definite assignment or a runtime value.

Accesses to external fields explicitly carry `DECLARATION_SOURCE_UNAVAILABLE`.
Available targets must resolve to exactly one retained declaration in the same
compiler scope. Lambda and local-class bodies emit local boundaries and do not
produce immediate variable accesses. Field/static initializer flows, exception
parameters and resource-variable identities are outside this initial contract.

Native capture validates the closed typed facts, canonical payload digest,
registered compilation, enclosing callable, target metadata and immutable source
membership. It verifies Javac UTF-16 coordinates against exact UTF-8 byte spans, including
CR-only, CRLF and mixed source line endings. Variable context sources preserve
original intermediate terminator bytes. The Javac line contract applies only to
the new variable sources; historical source snippets keep their prior behavior.
Documentation observations expose `VARIABLE_DECLARATION` and `VARIABLE_ACCESS`;
`variableSite` pins the file, byte span, source-content digest, span digest and
retained Source record. The Source preserves the surrounding exact source lines;
the span digest identifies the individual occurrence within those bytes.
`variableSite.sourceByteStart` and `sourceByteEnd` are original-file UTF-8 bounds
for that retained context. The access byte range must fit inside them, their
length must equal Source text length, and the local source slice must match the
span digest. Portable verification also checks file, service, revision, source
ID and text/evidence digest pins whenever bounds are present. Older variable
records with neither bound remain readable; they cannot supply a column and a
later data-state consumer must report `DATA_SITE_UNAVAILABLE` instead of guessing.
A partial pair of bounds is rejected.
Existing raw documentation context queries can return these observation kinds,
including an exact `--dependency` selection once its ID is known. No new storage
or default native page/data-state projection is introduced.

Old frozen snapshots and omitted native page options retain their existing
representations. A fresh compiler capture has a new analyzer/content digest and
may contain the new facts. Existing fact and byte budgets remain unchanged.

The focused real-JDK qualification test is
`documentation::analysis::tests::javac_variables_resolve_storage_modes_spans_and_scoped_admission`.
It exercises shadows, sibling locals, field/parameter assignments, compound and
increment accesses, member receivers, array elements, Unicode, deferred bodies,
external fields, file relocation, real CR-only/CRLF/mixed compiler captures and
native admission rejection. Separate typed
contract tests reject unsupported modes, fields, paths and spans. Source-ready
regressions do not constitute a compiler qualification result until executed.
