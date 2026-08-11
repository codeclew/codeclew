# E04 corrective action: one-hour infrastructure verdict

Date: 2026-08-10

Status: time-boxed corrective pass; no model experiment in scope

## Purpose

This pass exists because the previous E04 preparation consumed roughly fourteen
hours without producing a decision-bearing run. Its purpose is not to extend
Codeclew or recover the original 126-run objective. It fixes only reproduced
infrastructure defects and produces a zero-model `GO` or `NO-GO` verdict.

The time box explicitly excludes:

- new semantic operators, providers, language backends, or proof contours;
- opening hidden controller packages or deriving R1 seeds;
- model canaries or any of the 126 decision-bearing runs;
- speculative hardening without an executable false-pass counterexample.

## What the long preparation established

`E04-S0` remains `INFRA_ERROR / NO_DECISION`. Its 126 stored rows cannot rank
default, ast-index, and Codeclew because the runner misclassified real shell
commands, audited AST reads after repository teardown, treated redirection as
source mutation, and denied build-model caches. It is retained as diagnostic
evidence only.

The independently accepted product catalog is narrower than the D02
population:

- executable roots: `MAP_EDGE`, `TYPE_ASSIGNABLE`, and
  `PROPAGATE_DECLARED_TYPE`;
- auxiliary-only operators: `BIND_UNIQUE` and `VALUE_FLOWS_TO`;
- `NULL_HANDLES` and `PROJECTS_VALUE` are visible but non-executable and must
  return `UNSUPPORTED_CONSTRAINT_DOMAIN` before repository or receipt work.

This is the applicability that a future E04 run must measure. The benchmark
must not silently reinterpret unsupported families as infrastructure errors or
add product code after seeing their tasks.

## Root causes of the fourteen-hour overrun

1. **The experiment started without a complete zero-model preflight.** A
   three-arm canary would have exposed the S0 shell, cache, and CLI failures
   before 126 model calls.
2. **Product-first became product expansion.** Work crossed from harness repair
   into new goal language, compiler facts, providers, validation authority, and
   three Kotlin worker versions. Those changes may be useful, but they were not
   preparation for a fixed experiment.
3. **The target moved after every repair.** Product binaries, worker manifests,
   corpus packages, public commitments, cache snapshots, and runner hashes were
   repeatedly invalidated together.
4. **Fail-fast was correct but purely sequential.** Each cold Gradle/Maven task
   exposed one later defect, so missing full-population dependency closure was
   discovered one package at a time.
5. **Reproducibility drifted toward a security project.** Independent review
   found real false passes in AST readiness, but descriptor-relative inode
   attestation and sealed seed publication consumed time well beyond the
   minimum benchmark boundary.
6. **There was no wall-clock stop rule.** A token-unbounded persistent goal
   rewarded continued local repairs instead of an early `NO-GO` report.

## Corrective rules

The following rules are release criteria for any future E04 attempt:

1. Freeze product scope before corpus materialization. A product change ends
   the series and requires a new diagnostic cycle; it is never repaired inside
   a decision-bearing run.
2. Run gates in this order: harness self-test, public-corpus verification,
   full zero-model preflight, independent audit, one triplet canary, then R1
   freeze and model matrix.
3. Any full-population external dependency closure is built and verified
   offline before the first task preflight. Do not patch caches per failing
   task.
4. Every failure packet is written atomically before raising and includes
   `status`, `stoppedAt`, completed rows, provenance, and the exact failed
   invariant.
5. Unsupported product outcomes are valid, scorable outcomes. Infrastructure
   validity is limited to execution, provenance, isolation, and telemetry.
6. No new hardening is accepted without a minimal executable false-pass that
   changes the experiment verdict.
7. Each corrective pass has a one-hour wall-clock limit. At expiry, publish the
   strongest retained evidence and return `NO-GO`; do not continue repair.
8. A fresh decision series may be frozen only after the full preflight and the
   independent audit both pass unchanged artifacts.

## Corrective-pass evidence

The pass repaired two already reproduced defects:

- the sealed dependency-seed self-test now distinguishes the earliest
  read-only violation from a same-mode content digest mismatch;
- a new physical Maven dependency seed was resolved from pinned Maven Central
  and then verified offline against every authoritative leaf in all 21 public
  Maven packages.

Sealed dependency seed v4:

- output: `/private/tmp/codeclew-e04-dependency-seed-v4`;
- manifest SHA-256:
  `fb5b9e2789c70c1873ef413eba582d6207f1dddc8ef12c3866a95825c99e1339`;
- seal SHA-256:
  `9d9e3998a09a744808d640148c0955cc735c7fe3f09b8e08ae8c83470f1a2119`;
- Maven tree SHA-256:
  `7b00d9f60cf84de324eff1279d21d2e66cf221ed54d8d6286414acd9eb064cf7`;
- Maven tree: 19,431 files, 3,845,190,412 bytes;
- offline Maven leaf verification: 21/21.

## Verdict

`NO-GO` for a full zero-model preflight under the currently checked freeze.

The final freeze-checked attempt in this corrective pass stopped before row
1/42 with:

```text
preflight Codeclew binary/catalog does not match freeze provenance
```

No task subprocess, model call, controller read, R1 seed derivation, or canary
followed that refusal. There is no row packet for this final attempt because
the mismatch occurs at the pre-row freeze gate. An earlier pre-v4 diagnostic
attempt is retained separately with two rows (one Gradle pass and one Maven
offline-cache failure); it is not evidence for the current sealed-v4 state.

The refusal is correct. The checked freeze still commits the old S0-era
artifacts:

- frozen Codeclew binary:
  `a619ca0140cf61d62cb0e0fe09c196e0308e9b83d542923ed3d05386b493906a`;
- current Codeclew binary:
  `35ab42fa039e70d2061913bcda9dbc8930d956c757043d045cc80a7a31669a3a`;
- frozen runner:
  `2d5e9b63242b89912c6ce81d4eb83215db9892ae61376a5ff42af8bcb03791e7`;
- current runner:
  `6a4b43e2545db28c186b95108f1fea535213960a5941a5a1cc6eb6dc06d23f85`.

The old freeze also says `finalSeedsMaterializedAtFreeze=true`; it therefore
cannot be repurposed as a fresh diagnostic or R1 commitment.

This is a successful corrective-action outcome even though infrastructure is
not yet `GO`: the one-hour rule prevented another repair loop, retained a
complete offline Maven seed, and localized the next action to one provenance
operation rather than another product change.

Exact next step, in a separate explicitly authorized pass: create a new
diagnostic/preflight freeze bound to the current binary, catalog, runner,
public corpus, AST executable, and sealed v4 seed, with no hidden controllers
or final R1 seeds; then run the full 42-task zero-model preflight unchanged. A
decision-bearing R1 freeze is permitted only after that preflight and its
independent audit pass.
