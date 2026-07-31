# Correctness model

Safety is based on snapshot validation, unique source-backed anchors, compiler validation, minimal byte writes and atomic Git publication.

- Preview never writes the source repository.
- PSI replacement is performed on an in-memory file copy.
- Parse diagnostics reject malformed replacements.
- Kotlin 2.4.10 Gradle compilation rejects type, overload and unresolved-binding failures.
- Only candidate files listed by ActualWriteSet are copied into the worktree.
- Compilation and explicitly configured tests run before a commit exists on the target ref.
- `git update-ref <ref> <new> <expected>` is the publication boundary.
- Every status transition is appended to the ledger.

The local data-flow model is conservative. Multiple local definitions create a PHI; PHI inputs participate in DEF_USE slicing. Branch and loop predicates add control dependencies. Calls at depth zero and unsupported constructs create incompleteness boundaries. It is never valid to report `COMPLETE_SUPPORTED_SUBSET` after a budget cutoff or external call boundary.

No-op and untouched-file fidelity follows from source-copy replacement and writing only changed candidate files. Line endings and BOM are retained because PSI candidates are only accepted for touched files; exact no-op candidates produce no diff.

