# Codeclew multi-language M1 portability report

Date: 2026-08-13

## Result

`NOT_REACHED / KOTLIN_REAL_REPOSITORY_GATE_FAILED`

This is neither `FALSIFIED_BY_RUST` nor `FALSIFIED_BY_TYPESCRIPT`. The
language-neutral contract survived its Kotlin freeze, but the prerequisite
real-repository Kotlin contour failed before the Rust portability stage could
complete. Continuing to Rust or TypeScript would have violated the frozen
execution order and hidden the first decision-bearing failure.

## Kotlin K0.1 freeze

The shared contract was frozen after the bounded Kotlin fixture run.

| Item | Digest |
| --- | --- |
| protocol schema | `sha256:0a3c001b94991afedf13b1a011ecef37882378e0f240da318ac2e45e633323d9` |
| decision core | `sha256:de9e7c33b07e9ecb7c9e769a229fdca498fda83607b51de2c756320c90a27bed` |
| conformance corpus | `sha256:73235fe5d2eaf156d4e7481b3998de5d364dfbde09836e3ea6be0718c8cfba7a` |
| shared adapter contract | `sha256:3299a0d73fd5969ed29352a7eb89864e6e2cc45bfc61255d02e0e41954d3cbbe` |
| aggregate contract | `sha256:66bf7dfb018afb83868b729ae1fab7db35ba3a88cde3cde249e82ded8fcc042a` |
| lock file | `sha256:2fe26d4605f20137f4309067773c6764fffe2933696a338ec269f9d240bf4d91` |

The freeze includes the protocol, validation and policy core, conformance
vectors, and exactly four shared adapter-authority files. Language-owned
adapter implementations are deliberately outside the shared digest.

The fixture evidence is retained at
`docs/experiments/evidence/codeclew-multilang-k0-evidence.json`. It records an
honest `PARTIAL_BUDGET` result with 263 facts, 191 boundaries, 191 mandatory
unknown obligations, zero model-visible source bytes, and no completeness
claim. That fixture success is not accepted as the real-project gate.

## Real Kotlin gate

A clean clone of an existing 339-file Kotlin/Maven service at revision
`90bcd982c9184f747e4f475ca67d329163880a5f` was analyzed with the frozen
adapter binary. Offline build discovery and K2 extraction completed far
enough to retain a 67 MB provider cache containing 4,174 declaration
descriptors. Six enum-entry constructors had
`effectiveVisibility = "local"`. The legacy Kotlin descriptor validator
accepts only `public`, `internal`, `private-in-class`, `private-in-file`, and
`protected`, so it aborted with:

```text
InvalidInput: declaration descriptor has an unknown compiler enum
```

The process exited 1 after 174.84 seconds, with no projection, evidence-store
object, typed refusal, or UNKNOWN boundary. It did fail closed: no positive
receipt, `PROVEN`, `COMPLETE`, or mutation authority was issued. The exact
failure packet is
`docs/experiments/evidence/codeclew-multilang-m1-kotlin-real-failure.json`.

This is an adapter/legacy-ingestion contract gap, not evidence that a neutral
evidence protocol is impossible. It does prove that the current milestone is
not runnable on its required real Kotlin contour.

## Rust R0 status

Rust work was stopped immediately after the Kotlin result.

- Rust-owned source was aligned with the shared envelope.
- Standalone compilation, formatting, two focused pinned-rust-analyzer tests,
  one fixture collection, and strict JSON-schema validation had passed before
  the stop.
- The shared `codeclew-evidence` repetitions=2 run was interrupted and has no
  accepted receipt.
- No unprepared real Rust repository was run.
- No R0 portability manifest or post-run freeze was issued.

The Rust source therefore remains experimental, not a supported adapter.

## TypeScript T0 status

`SOURCE_READY / NOT_RUN`.

The TypeScript strict adapter and its conformance fixture exist, but the final
source was never run after the shared schema tightened. Earlier tests belonged
to a previous envelope version and are not evidence for the current source.
No T0 receipt or portability manifest exists.

## Portability conclusion

The Kotlin freeze demonstrated that the core can be made language-neutral at
the source and conformance-contract level. It did not complete the required
Kotlin -> Rust -> TypeScript falsification sequence. Consequently:

- shared-core semantic drift observed after K0.1: `0` at the stop point;
- Rust portability: `NOT_EVALUATED`;
- TypeScript portability: `NOT_EVALUATED`;
- overall portability result: `NOT_REACHED`;
- permissible decision: `PIVOT`, not `GO`.

The next portability series must start from a new Kotlin freeze after a
preregistered real-repository conformance repair. Reusing this K0.1 as though
the failed gate had passed is prohibited.
