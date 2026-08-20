# SThread preflight

Before every SThread `agent-context` or `task-apply`, run:

```bash
python3 scripts/sthread_preflight.py \
  --receipt /private/tmp/codeclew-sthread-preflight.json
```

The command has a 60-second default budget and emits one compact canonical JSON
receipt. `READY` proves all of the following against the current clean `HEAD`:

- the repository-local Gradle cache has been hydrated copy-on-write from the
  local dependency seed without copying `gradle.properties` or daemon state;
- the release `clew` binary builds offline from the current source;
- the complete Codeclew Gradle root configures offline against the hydrated
  cache, including the shared Kotlin 2.4.10 and Kotlin 2.1.21 projects;
- the trusted worker executes its exact `OpenProject` path on the small
  repository-owned Kotlin 2.4 fixture;
- Kotlin 2.1 Gradle and Kotlin 2.3 Maven compiler-semantic smoke indexes pass.

The script captures subprocess output internally and exposes only digests plus
a bounded error message, so cache size and project-model JSON do not consume
agent context. The normal run must not start after `FAILED`.

Each hydrated cache receives a fingerprint marker. An unchanged toolchain skips
the copy step on later runs; the probes still execute and therefore cannot be
forged by a stale marker.

The three independent worker probes run concurrently after the shared offline
cache/build checks. This keeps a cold preflight inside the same wall-time budget
without dropping any Kotlin version or turning a failure into a warning.

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
- `--budget-seconds N` changes the hard wall-time budget.
- `--gradle-cache-seed` and `--maven-repository-seed` select explicit local
  dependency seeds.

The cache hydration uses APFS clone copies on macOS and `--reflink=auto` on
Linux. Only ignored runtime state is changed; tracked source remains untouched.
