# Explicit saved processes

`quantity.json` is a process definition for the source-syntax orders fixture.
It declares requested scope and outcomes; these are not asserted runtime facts.
The `docsys_t10_*` tests exercise transient inspection, save/reopen/update,
cycles and missing children, negative interaction membership, conditional
source outcomes, separate child/parent review, and transitive staleness.
Compiler-resolved cross-service calls are not implied by the syntax fixture.
