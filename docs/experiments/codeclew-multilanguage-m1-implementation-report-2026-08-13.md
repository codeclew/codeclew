# Codeclew multi-language M1 implementation report

Date: 2026-08-13

## DECISION

`PIVOT / REAL_REPOSITORY_CONFORMANCE_GAP`

## SELECTED_TRACK

`B` — Repository Understanding + Change Impact.

## RESEARCH_REPORT

`/workspace/user/Downloads/deep-research-report (1).md`

SHA-256:
`6b9d9c73a809e896506dfd2645d09b77e8251940138eb813c85aeb573a270791`.

Execution-contract SHA-256:
`a115a0690a7fe9ffc79d6cfbe2f31f2a58bc3412f9af44d22dd6e336765c35ee`.

## IMPLEMENTED

- A standalone `evidence-core` crate with versioned protobuf schemas for
  exact workspace snapshots, adapter-owned entities and occurrences,
  namespaced relations, capabilities, coverage/boundaries, obligations,
  receipts, and cost telemetry.
- Canonical content digests, strict ordering/duplicate validation,
  snapshot/toolchain/build/adapter binding, exact mandatory closure, explicit
  grade acceptance, and fail-closed receipt verification.
- A language-neutral, read-only adapter envelope and runtime that invokes an
  adapter by absolute digest, validates it into the typed core, writes a
  content-addressed object, and returns a bounded source-free projection.
- Kotlin K2/FIR translation into the new envelope and an honest bounded impact
  result.
- Frozen K0.1 contract and Kotlin fixture evidence.
- Experimental Rust and TypeScript adapter sources, stopped before capability
  qualification when the real Kotlin gate failed.

No automatic edit, patch application, reusable apply authority, model call,
JBMC, ByteBack, universal AST, bytecode IR, or benchmark-family dispatch was
added.

## REUSED

- trusted version-pinned Kotlin worker launch and K2/FIR extraction;
- canonical hashing and exact repository identities;
- bounded projection concepts and evidence paths;
- snapshot-validity, provenance, UNKNOWN propagation, and closure mechanics;
- content-addressed storage and atomic publication concepts.

The legacy MAP/PTC edit vertical was not renamed into a generic operator.

## REPLACED_OR_DELETED

Nothing was deleted. The new Track-B path is additive and read-only. Legacy
editing, heuristic context, and E04 infrastructure are isolated from the new
decision core. The detailed disposition is in
`codeclew-multilanguage-m1-migration-report-2026-08-13.md`.

## CORE_FREEZE

Kotlin K0.1:

- protocol: `sha256:0a3c001b94991afedf13b1a011ecef37882378e0f240da318ac2e45e633323d9`;
- decision core: `sha256:de9e7c33b07e9ecb7c9e769a229fdca498fda83607b51de2c756320c90a27bed`;
- conformance: `sha256:73235fe5d2eaf156d4e7481b3998de5d364dfbde09836e3ea6be0718c8cfba7a`;
- shared adapter contract:
  `sha256:3299a0d73fd5969ed29352a7eb89864e6e2cc45bfc61255d02e0e41954d3cbbe`;
- aggregate: `sha256:66bf7dfb018afb83868b729ae1fab7db35ba3a88cde3cde249e82ded8fcc042a`;
- lock file:
  `sha256:2fe26d4605f20137f4309067773c6764fffe2933696a338ec269f9d240bf4d91`.

Rust freeze: `NOT_ISSUED`.

TypeScript freeze: `NOT_ISSUED`.

## PORTABILITY_RESULT

`NOT_REACHED / KOTLIN_REAL_REPOSITORY_GATE_FAILED`.

This is not a Rust or TypeScript falsification. The prerequisite Kotlin
real-repository adapter failed before those stages could be decision-bearing.
The complete status is in
`codeclew-multilanguage-m1-portability-report-2026-08-13.md`.

## PROOF_SAFETY

- false `PROVEN` in the executed evidence-core conformance suite: `0`;
- false completeness in that suite and retained Kotlin evidence: `0`;
- real Kotlin failure issued a positive receipt: `false`;
- real Kotlin failure issued `COMPLETE`: `false`;
- fixture projection status: `PARTIAL_BUDGET`;
- fixture mandatory UNKNOWN obligations: `191`.

The real failure did not become a false proof, but it also did not become the
required typed refusal/UNKNOWN packet. That totality gap is one reason the
decision is PIVOT.

## TESTS

Passed before the decision-bearing failure:

```text
cargo fmt --package evidence-core -- --check
cargo check -p evidence-core --all-targets
cargo test -p evidence-core --all-targets -- --test-threads=1
  20 conformance + 1 unit PASS
cargo clippy -p evidence-core --all-targets -- -D warnings
cargo run -q -p evidence-core --bin core-contract -- verify
python3 scripts/multilang_portability_stage.py verify \
  contracts/core/kotlin-k0.portability.json
cargo test -p evidence-adapters --all-targets -- --test-threads=1
  3 adapter-library + 7 shared-runtime tests PASS
```

The preregistered strict scan found no language/task-family branch in the
frozen core or four shared adapter-authority files.

Known baseline failures remain separate: workspace verification already
failed on 12 pre-existing `clew` Clippy diagnostics and four
`semantic-corpus` diagnostics. A full workspace green baseline was never
established and is not claimed.

The final `cargo clippy -p evidence-adapters --all-targets -- -D warnings`
also stopped on those same 12 `clew::evidence_authority` diagnostics before
lint qualification of the dependent package could complete. This is retained
as the pre-existing baseline failure, not reported as a green adapter Clippy
gate and not repaired after the decision.

Decision-bearing real Kotlin command:

```text
codeclew-kotlin-evidence --repo "$CLEAN_REPO" \
  --max-depth 2 --max-entities 128
```

Result: exit 1 after 174.84 seconds with
`InvalidInput: declaration descriptor has an unknown compiler enum`.

## COST

Bounded fixture run (two exact-snapshot adapter invocations plus core/store
and projection):

| Metric | Value |
| --- | ---: |
| end-to-end wall | 29.02 s |
| first adapter invocation | 11.200230 s |
| second adapter invocation | 10.733142 s |
| typed evidence-core validation | 3.757934 s |
| evidence-store write/read | 0.111295 s / 0.022988 s |
| peak RSS | 612,007,936 bytes |
| source bytes read | 66,651 |
| stored fact bytes | 239,236 |
| projection bytes | 32,121 |
| model-visible source bytes | 0 |

The second invocation repeated cold indexing (`warmIndexMicros = 0`); it is
not reported as an effective warm cache.

Real-repository decision run:

| Metric | Value |
| --- | ---: |
| final analysis wall | 174.84 s |
| maximum resident set | 4,820,566,016 bytes |
| Kotlin main source files | 339 |
| provider descriptors before rejection | 4,174 |
| output bytes | 0 |
| model calls | 0 |

All discovery/preparation/retry attempts leading to the decision consumed at
least 285.58 seconds of observed command wall time. This includes failed
Gradle plugin/dependency closure attempts, clean local clones, isolated cache
seeding, and the final Maven/K2 run; it excludes later read-only diagnostics
and the earlier core implementation/test time. Dependency-cold cost, a
successful real warm run, and Rust/TypeScript end-to-end costs remain
unmeasured. Therefore the Definition-of-Done full-cost criterion is `FAIL`,
not inferred from the fixture.

At the decision checkpoint the goal had consumed at least 1,230,809 agent
tokens. Model benchmark calls and benchmark model tokens remained zero.

## EXPERIMENT_STATUS

`NOT_STARTED_WITH_REASON`.

The prospective four-arm model experiment is prohibited because the
zero-model real-repository applicability gate failed, Rust/TypeScript
portability did not complete, complete cost was not measured, and the product
GO thresholds cannot yet be evaluated.

## GO_STOP_CRITERIA

| # | Criterion | Result |
| ---: | --- | --- |
| 1 | Track matches research | PASS |
| 2 | Runnable end-to-end milestone | FAIL on real Kotlin repository |
| 3 | Versioned schemas/APIs | PASS |
| 4 | Exact capability tuple | PASS in core/fixture contour |
| 5 | Snapshot/fact/obligation/receipt provenance | PASS in core/fixture contour |
| 6 | Explicit UNKNOWN/incompleteness | PARTIAL; fixture yes, real abort no typed packet |
| 7 | No false PROVEN in executed suite | PASS, 0 |
| 8 | No benchmark-family dispatch in new core | PASS |
| 9 | No language decision branch in shared core | PASS |
| 10 | Kotlin -> Rust -> TypeScript portability | FAIL / not reached |
| 11 | Tests/baseline reported | PASS with pre-existing failures separated |
| 12 | Complete cost measured | FAIL |
| 13 | Research product GO thresholds | NOT EVALUATED |
| 14 | Reproducible result | PASS for K0.1 and the Kotlin failure |
| 15 | No unsupported model-benefit claim | PASS |

The required conjunction for `GO` is false.

## DEVIATIONS_FROM_RESEARCH

- The research target expected all three adapters on real projects. Only the
  Kotlin fixture completed; the real Kotlin project failed, Rust stopped
  mid-R0, and TypeScript remained unrun.
- Complete cold/warm/dependency cost was not obtained.
- No prospective benchmark harness was activated because its prerequisites
  failed.
- No claim is made that the shared architecture is impossible. The observed
  failure is localized to Kotlin provider/legacy descriptor conformance and
  process-level refusal totality.

## REPRODUCTION

Verify the frozen core and retained Kotlin fixture evidence:

```bash
cd /workspace/user/repo/research/codeclew
cargo run -q -p evidence-core --bin core-contract -- verify
python3 scripts/multilang_portability_stage.py verify \
  contracts/core/kotlin-k0.portability.json
```

Reproduce the real Kotlin failure without modifying the source repository:

```bash
cd /workspace/user/repo/research/codeclew
work=$(mktemp -d /private/tmp/codeclew-kotlin-real.XXXXXX)
git clone --local /workspace/user/repo/private-product/product-repo "$work/repo"
mkdir -p "$work/repo/.semantic-thread/maven-repository"
cp -al /workspace/user/.m2/repository/. \
  "$work/repo/.semantic-thread/maven-repository/"
/usr/bin/time -l target/debug/codeclew-kotlin-evidence \
  --repo "$work/repo" --max-depth 2 --max-entities 128 \
  >"$work/stdout.json" 2>"$work/stderr.txt"
```

Expected semantic result: nonzero exit with the unknown compiler enum error;
stdout remains empty. Wall time and diagnostic paths are observational and
need not byte-match the retained run.

## NEXT_DECISION

Preregister M1.1 around a total Kotlin-owned enum/boundary translation, run it
on two unprepared real Kotlin repositories including Maven and Gradle, and
issue a new Kotlin freeze only if both produce either a valid bounded result
or a canonical typed refusal. Rust and TypeScript portability must restart
after that new freeze; this failed K0.1 series must not be resumed as GO.
