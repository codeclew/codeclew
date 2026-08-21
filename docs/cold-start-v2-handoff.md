# Work to resume after cold-start v2

This file is the authoritative handoff for work intentionally paused on
2026-08-21. Cold-start v2 is now the blocking priority.

## Completed foundation

- `9bebde6`: managed runtime, sessions, context/plan CAS, detached task-run
  supervisor, bounded context output, direct task-apply removal, preflight and
  cache-runner deletion.
- `19d0ceb`: proven K24 BTA backend/store source compatibility imported from a
  prior atomic SThread transaction.
- `388693f`, `102cdd7`: trusted K24 distribution update and verification of all
  worker distributions.
- RELEASE capsule/session smoke tests passed.
- warm launcher audit measured approximately 0.13 seconds without Cargo,
  rustc, Gradle, or Maven.

## Deliberately cancelled SThread attempt

Session: `session:e18b5251-2c5e-4378-8f64-6fe187b8900c`.

Intent: add authoritative K24 per-file receipts, preserve incremental
generations, and move large worker transport out of the repository.

The K21 context attempt was cancelled after more than 60 minutes when profiling
proved repeated repository walks and compiler analyses. It produced no context,
plan, candidate, or Kotlin source change. It must not be resumed or reused.
After cold-start v2, create a fresh session and one fresh bounded context.

## Recovery foundation

The foundation change includes a tested explicit publication recovery
implementation in:

- `crates/clew/src/main.rs`;
- `crates/clew/src/transaction.rs`.

It adds `session recover --session ... --run ...`, distinguishes cancellation
before ref movement from recovery after the ref reached the candidate, rebuilds
only the repository index in the latter case, and never rolls Git back. `cargo
check`, its targeted CLI test, and strict clippy passed.

An isolated clone contains commit `4719932` based on `102cdd7` which removes all
legacy public command families. It exposes only `session`, `context`, `plan`, and
`task-run`, changes help text to language-neutral wording, and has a strict CLI
contract test. Integrate or reproduce it after the shared recovery diff is
committed. The temporary clone is not an authority for worker/runtime bytes.

## BTA24 work after optimization

1. Open a fresh RELEASE session on a clean base using the new generation API.
2. Create one bounded context for the shared K21/K24 generation/receipt
   surfaces. Do not reuse the cancelled context.
3. Implement generic per-file and cross-boundary completeness receipts for K24.
4. Implement K24 delta generations with full fallback on unknown invalidation.
5. Replace repository `.semantic-thread` large-response transport with private
   attempt/CAS transport.
6. Verify cold, incremental, recovery, unchanged-hit, corruption, and tamper
   cases for K21/K23/K24.
7. Require `UNCHANGED_HIT` internal p95 <= 300 ms and end-to-end p95 <= 2 s.

## Remaining radical cutover work

- Integrate explicit `session recover` and legacy CLI removal.
- Remove any production fallback to checkout-relative
  `CARGO_MANIFEST_DIR`; commands must require `./clew` runtime authority.
- Make context expansion read missing immutable shards only.
- Complete candidate worktree relocation/GC and derived-output force rules.
- Enforce PROJECT_NATIVE model-cache policy and EXTERNAL release-only policy.
- Remove remaining v1 production schemas/routes/scanners after v2 acceptance.
- Add syscall-equivalence tests proving `.semantic-thread` is never accessed.
- Run demo and correct benchmark corpus setup.
- Run full workspace format, clippy, tests, worker manifests, privacy and history
  checks.

## Final validation and delivery

After cold-start v2 and BTA24:

1. run cold, warm, incremental, cancellation, recovery, concurrency,
   determinism, privacy, and resource-limit suites;
2. run the complete verify script;
3. repeat the paired Default-vs-Codeclew experiment on the same hidden oracle;
4. record time, starts, raw/noncached tokens, files changed, recovery events,
   and final oracle result;
5. remove temporary Codeclew clones/worktrees/runs after explicit target checks;
6. commit and push;
7. watch GitHub Actions to completion and fix repository-owned failures.
