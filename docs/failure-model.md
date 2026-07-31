# Threat and failure model

The platform assumes the analyzed repository and its Gradle build may be buggy or hostile. Validation runs local repository code and is therefore not an OS sandbox; users must execute it in an appropriately isolated environment. Source text is not logged by default.

Handled failures include truncated/corrupt frames, worker crashes, invalid PSI replacements, compiler/test failures, stale project models, missing/ambiguous anchors, worktree failures, concurrent ref movement and process crashes between ledger events. Before the CAS point, failure leaves the target ref unchanged. After CAS, the ledger plus commit trailers reconstruct provenance.

The MVP does not defend against a malicious Gradle build escaping the host account, compromised Git binaries, filesystem corruption after successful fsync, SHA-256 collision, or force-pushes performed outside the coordinator. These require process sandboxing, signed evidence and remote-ref coordination in a hardened release.

