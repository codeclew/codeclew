# Portable evidence fixture contract

The `docsys_t12_*` CLI cases produce real packages through local supported capture.
They copy each artifact into a fresh documentation fixture with no application
checkout, Git command or compiler available, then inspect, import, check, render
and separately review its evidence.

Mutation cases cover missing/corrupt parts, traversal, symlinks, compression,
unknown schemas, wrong service/revision, excessive byte declarations, invented
source authority and a rehashed untrusted manifest. Valid selection bytes remain
unchanged after rejection. Coordinator expectation advancement, missing new
results, idempotent imports and older replay are exercised separately.

The source-free report case omits source/index parts and refuses evidence
admission. A Kotlin compiler capture failure at a known revision is preserved as
an offline gap with its original stable reason and artifact provenance. This
fixture does not establish successful Kotlin 1.9 compiler admission; that is a
separate runtime qualification.
