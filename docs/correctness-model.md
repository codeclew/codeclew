# Correctness model

Safety is based on snapshot validation, unique source-backed anchors, compiler validation, minimal byte writes and atomic Git publication.

- Preview never writes the source repository.
- PSI replacement is performed on an in-memory file copy.
- Parse diagnostics reject malformed replacements.
- A worker matching Kotlin 2.1.21 or 2.4.10 Gradle compilation rejects type, overload and unresolved-binding failures.
- Only candidate files listed by ActualWriteSet are copied into the worktree.
- The affected compilation and the snapshot's configured tests run by default before a commit exists on the target ref; a transaction may explicitly replace that test set.
- K2 facts and the complete SQLite index snapshot are built in a private same-filesystem staging database before `git update-ref <ref> <new> <expected>`.
- After the ref compare-and-swap, one atomic rename installs the staged index. A rename failure performs the inverse compare-and-swap; if that cannot succeed, the ledger retains a recoverable non-terminal state.
- Crash recovery publishes or reconstructs the index before it records `COMMITTED`; a reachable Git trailer alone is not sufficient.
- Every pre-commit status transition is appended to the ledger. After the ref/index commit point, a ledger I/O failure is reported as `ledgerRecorded: false` on an otherwise successful commit; Git trailers allow later inspection to reconstruct the terminal event.

The local data-flow model is conservative. Multiple local definitions create a PHI; PHI inputs participate in DEF_USE slicing. Branch and loop predicates add control dependencies. Calls at depth zero and unsupported constructs create incompleteness boundaries. It is never valid to report `COMPLETE_SUPPORTED_SUBSET` after a budget cutoff or external call boundary.

No-op and untouched-file fidelity follows from source-copy replacement and writing only changed candidate files. Line endings and BOM are retained because PSI candidates are only accepted for touched files; exact no-op candidates produce no diff.
