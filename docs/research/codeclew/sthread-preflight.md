# SThread preflight

Before every SThread `agent-context` or `task-apply`, run:

```bash
python3 scripts/sthread_preflight.py \
  --receipt /private/tmp/codeclew-sthread-preflight.json
```

Cold preparation has no default wall-time budget and emits one compact canonical
JSON receipt. `--budget-seconds` is an optional operational stop, not a performance
acceptance threshold. `READY` proves all of the following against the current clean `HEAD`:

- the repository-local Gradle cache has been hydrated copy-on-write from the
  local dependency seed without copying `gradle.properties` or daemon state;
- an isolated checkout has a copy-on-write Cargo `target/release` seed (never
  the much larger `target/debug`), which Cargo still validates against the
  selected source and toolchain before reuse;
- all three trusted Kotlin worker installDist directories are materialized only
  from a seed whose pinned manifests match the checkout byte-for-byte;
- the release `clew` binary builds offline from the current source;
- tracked Rust source passes `cargo fmt --all -- --check` before that build;
- the complete Codeclew Gradle root configures offline against the hydrated
  cache, including the shared Kotlin 2.4.10 and Kotlin 2.1.21 projects;
- the trusted worker executes its exact `OpenProject` path on the small
  repository-owned Kotlin 2.4 fixture;
- Kotlin 2.1 Gradle and Kotlin 2.3 Maven compiler-semantic smoke indexes pass.
- an additional independent Kotlin 2.1 process reports `UNCHANGED_HIT` against
  the same persistent compiler index and returns the same compiler graph digest.

The script captures subprocess output internally and exposes only digests plus
a bounded error message, so cache size and project-model JSON do not consume
agent context. The normal run must not start after `FAILED`.

Before `agent-context`, preflight the request itself: every required `--term`
must be an exact declaration name already established by bounded evidence or
repository metadata. JSON keys, field names, error strings, and broader search
needles belong in `--intent`, not in the mandatory term set. After the call,
accept only `COMPLETE_TASK` with every requirement `SATISFIED`; any partial or
unsatisfied result ends the attempt before plan construction.

Before `task-apply`, preflight CREATE_FILE dependencies as a closed set. A new
file may call a repository symbol only when bounded context exposes that symbol
as its own declaration with non-private visibility. Seeing an unqualified call
inside another declaration's `sourceText` does not prove cross-file access.
Otherwise keep the new declaration self-contained on explicitly imported
public APIs. This check is in addition to generic plan validation and prevents
candidate-only Kotlin visibility failures.

Each hydrated cache receives a fingerprint marker. Independent Cargo, Gradle,
Maven, and trusted-worker hydration runs concurrently under the same deadline.
An unchanged toolchain skips the copy step on later runs; the probes still
execute and therefore cannot be forged by a stale marker.

The Gradle configuration check and three independent worker probes run
concurrently after the shared offline cache/build checks. The separate warm
Kotlin 2.1 probe runs afterwards so cold preparation cannot masquerade as a
warm-index result.

Before those probes, `STHREAD_PROTOCOL_CAPABILITY` rejects a legacy CLI that
can only produce `LEGACY_HEURISTIC_*` task contexts. The current proof-capable
CLI must advertise the exact model-input context surface; a successful compiler
smoke alone is not sufficient authority for an edit transaction.

When a later SThread run discovers a preparation failure that this receipt did
not catch, stop that run immediately. Add a generic reproducer/probe or
authority check to this preflight, cover it in `--self-test`, and only then
retry from a fresh clean context. Do not add a repository-specific exception.

If the failed stage is inside the current SThread implementation itself, a
repair transaction may use a separate clean, known-good Codeclew bootstrap.
That bootstrap must first produce its own `READY` receipt with this script, and
the repair must be limited to the failed stage. A `FAILED` workspace is never
used for an ordinary context or task run.

Useful development switches:

- `--allow-dirty` permits testing an uncommitted preflight change but records
  `trackedClean: false`; it is not an acceptable production receipt.
- `--skip-smoke` is for focused script development only.
- `--budget-seconds N` adds an explicit hard operational timeout; it is absent by default.
- `--cargo-target-seed`, `--trusted-worker-seed`, `--gradle-cache-seed`, and
  `--maven-repository-seed` select explicit local build/dependency seeds.

The cache hydration uses APFS clone copies on macOS and `--reflink=auto` on
Linux. Only ignored runtime state is changed; tracked source remains untouched.
