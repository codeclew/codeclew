# E04 readiness DAG: terminal product-coverage NO-GO

Date: 2026-08-11

Status: complete; independently accepted terminal `NO_GO`

## Outcome

E04 did not start the 42-task zero-model preflight, an R1 materialization, a
model canary, or the 126-run matrix. The new readiness graph stopped earlier at
the first decision-bearing prerequisite: the frozen product can support at
most 2 of the preregistered 14 positive family-by-build cells, while the E04
contract requires at least 9.

This is an intentional successful outcome of the corrective pass. It prevents
an expensive run whose success criterion is unreachable without changing the
product or lowering the threshold after seeing the evidence. Neither action is
permitted inside this series.

The historical evidence in
`codeclew-e04-corrective-action-2026-08-10.md` remains unchanged. This report
supersedes only its proposed next action: the explicit readiness DAG now proves
that full preflight is downstream of a failed semantic-applicability gate.

## Readiness graph delivered

The readiness layer now provides:

- one byte-pinned canonical DAG instead of caller-selected success flags;
- content-addressed immutable receipts and atomic current pointers;
- exact `READY`, `FAILED`, `STALE`, and `BLOCKED` semantics;
- selector-scoped live inputs, dependency receipt binding, and transitive
  invalidation;
- separate `PREPARE`, read-only `VERIFY`, direct-authority, and import actions;
- live rehashing of binaries, catalogs, source, public populations, dependency
  seeds, reports, packet sets, and stage-specific artifacts;
- explicit start roots checked before any tool, controller, R1, or model work;
- retained structured failure evidence and a separate audited terminal branch;
- rejection of alternate but structurally valid graphs before store creation;
- technical gates on preflight, canary, materialization, hidden verification,
  final runs, judging, and summarization.

The full successful-series graph remains present, but none of its downstream
roots can be recognized when product coverage is `FAILED`.

## Live terminal execution

Fresh readiness store:

```text
/private/tmp/codeclew-INFRA_DIAGNOSTIC-e04-v2.xt6osY/readiness-terminal-coverage-v2
```

The superseded v1 store was not reused after an alternate-graph bypass was
found and closed.

Canonical live identities:

| Item | SHA-256 |
| --- | --- |
| Readiness graph identity | `5925214112f6ea4a2914bfc4bb2d9702f8c76c77298bb7656d6392cf70ae3342` |
| Readiness graph file | `46d0a4356cd96ce05b6a09633abc4d2ba7e6a9dab854a21825a94280b4b768f6` |
| Readiness checker | `3103d8fc8e7bb13ab51b27d1f322b6f2811db848da1776fd49bc6c9cd373ef07` |
| Harness runner | `6b02abba5104a1e4cf9a7a2d363bb2ac4b3855cf3ce3c744134fc8b3b8f64dc1` |
| Codeclew binary | `35ab42fa039e70d2061913bcda9dbc8930d956c757043d045cc80a7a31669a3a` |
| semantic-corpus binary | `5f4b2c8838de9429423fb17252a3387eae6e30969b47019131a192cd1b92c517` |
| Product-coverage contract | `2b5092965614ead650f2a892e703daf4f6ef024d2b34d3c6f4321a2af81883f1` |
| Sealed dependency-seed manifest | `fb5b9e2789c70c1873ef413eba582d6207f1dddc8ef12c3866a95825c99e1339` |

The live path reached these material states:

| Node | Status | Receipt |
| --- | --- | --- |
| `ARTIFACT_PROVENANCE` | `READY` | `8948d5a32ecb2e1cb94e4668af720cb9f0ac72919dbf6b5a36f1595118946ba6` |
| `DIAGNOSTIC_PUBLIC_CORPUS_42` | `READY` | `131c71aedea3f86607ddc9d2ac9f3e76ca43f77d27f2dcfbaebbd39b125759dc` |
| `DEPENDENCY_SEED_VERIFY` | `READY` | `6ddb9dbfc2be39e07bbe4d728053e445e7ec797734dc626ca1c41a258a13f2c7` |
| `HARNESS_SELF_TEST` | `READY` | `84fef4252efd6a5c277dec98a5ad86ac1594732eaef6ac92a3cb4ef5e37bcb06` |
| `DIAGNOSTIC_FREEZE_PREPARE` | `READY` | `9a9ca20849d5e2812d56958341c12c2c9b25a9f4b2a38e7624eef7fb8f7834b6` |
| `DIAGNOSTIC_FREEZE_VERIFY` | `READY` | `31ff734617fce94db23cf9619985c82ad0e9b4f51cfec321a2f4b6953aeb8f1e` |
| `PRODUCT_COVERAGE_START_READY` | `READY` | `bdfcc9de272bbb18589dcc37bb2eadfe377dcc2ae8dd0de272ddccc1bf3adef4` |
| `PRODUCT_COVERAGE_GUARD` | `FAILED` | `8e63a7c18c6308164506c7ee4f514f1f02628e181ad15b1d2c95ce6b50e9c52b` |
| `PRODUCT_COVERAGE_FAILURE_AUDIT_IMPORT` | `READY` | `6f18f9d2c02dea08dd15eb505d027d70b6a97637c5a6438fecc1ec85a6589d3a` |
| `E04_COVERAGE_NO_GO_COMPLETE` | `READY` | `0468e7ca53cee1b08329452cc4148ed1afca9f319dfa669f38359e119697341a` |

Production root recognition independently returned that terminal receipt as
current `READY` under store ID
`e5cd617b86658289f50caa41a587fb6112065353b8e488fada5e1862a482adbe`.

## Why coverage is terminal

The preregistered gate requires:

- at least 5 composition identities;
- at least 9 of 14 positive family-by-build cells;
- exact ambiguity support;
- all 14 must-refuse cases correct;
- zero false `BOUND` decisions;
- both Gradle and Maven.

The compiler/catalog/source-derived upper-bound analysis found:

- 2 supported cells: producer-transform-consumer on Gradle and Maven through
  `MAP_EDGE`;
- 2 incomplete cells: type-signature propagation on Gradle and Maven binds
  only 2 of the 3 required roles (`CALL_SITE` is absent);
- 10 unsupported cells across DTO/event evolution, persistence nullability,
  configuration lifecycle, retry/resource lifetime, and test strengthening.

Therefore the maximum possible positive-cell count is 2/14, below the fixed
9/14 threshold. This is an applicability upper bound, not a model score.
Running models cannot repair a missing executable root.

The canonical coverage report has SHA-256
`d500a55daa45987349f41aba9a3acc73f2de300853f3ef4eac17cf84f52c222f`.
The independent audit is
`codeclew-e04-product-coverage-audit-2026-08-11.json`, SHA-256
`577b61448f79b4a0ff228da07dde2f242accff73e642a8e058b2bd49ad352c50`.
Rust authority derivation and an independent Python derivation both returned
the same 14 cells and upper bound.

## Safety and non-execution evidence

The final current-pointer set contains only the ten terminal-branch nodes
listed above. The readiness artifact set contains exactly the diagnostic
freeze, coverage report, and coverage audit.

There is no pointer or artifact for:

- diagnostic full preflight or model canary;
- R1 decision, materialization, annotation, hidden verification, or R1
  preflight;
- final 126-run matrix;
- judging, summary, results verification, or GE1.

The audit records:

```text
modelCalls=0
controllersOpened=false
r1Materialized=false
```

An independent verifier re-recognized the complete live terminal closure and
returned `ACCEPT` after the audit import and root publication.

## Decision and next permissible action

Final decision:

```text
NO_GO / preregistered 9-of-14 product coverage is unreachable
```

GE1 is not evaluated. Thresholds are not lowered, unsupported cells are not
relabelled, and no product code is repaired after observing this result.

A future attempt requires a separately preregistered experiment with either:

1. a narrower, justified applicability denominator fixed before materializing
   any corpus; or
2. a new product series that first adds and independently accepts the missing
   executable semantic roots, then regenerates every freeze and readiness
   receipt from the beginning.

The current E04 series is complete at the terminal `NO_GO` root.
