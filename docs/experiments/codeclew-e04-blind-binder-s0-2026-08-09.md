# E04-S0: blind binder experiment — infrastructure-invalid series

Date: 2026-08-09

Verdict: `INFRA_ERROR` (`infrastructure-invalid`) / `NO_DECISION`

Independent verdict: `REJECT`
Graph consequence: `E04` is not accepted; `GE1` and `E05` remain closed.

## Executive conclusion

The first complete 42-task, three-arm E04 series was executed and retained, but
it cannot decide whether Codeclew beats default filesystem navigation or
ast-index. The stored arithmetic is reproducible; the capability comparison is
not valid because the runner misclassified real tool calls, removed the AST
repository before auditing reads, and prevented Codeclew from using the
Gradle/Maven caches required by its project model.

This series is preserved as `E04-S0`. It is useful evidence about the missing
product/agent integration, but it is not a `GE1` receipt and its opened tasks
must not be reused for a decision-bearing rerun.

The significant product result is narrower and actionable: S0 does not
establish whether the lower project/index/context layer works in this contour,
while source and CLI inspection show that a self-describing, one-command
task-to-typed-goal binder does not yet exist. The current dedicated PTC proof
requires the agent to prelocalize both a workflow and a matching test; the
corpus supplies no such relevant test, and the other six families have no
certified binder.

## Frozen execution and retention

- Product base: `a6ae1e48359eccef15060c1bb249a648857f30c9`.
- Binder tree: `fc349a728c92750e7eb36c39368ef693d708c98badccf4eb9c0a246279474ba4`.
- Population: `a209f115b0a175bb74859b0539f75932cd664a495332ccf10b634b3cf1c2b9f2`.
- Model: `gpt-5.6-terra`, reasoning `low`.
- Matrix: 42 withheld tasks, seven families, three outcomes, Gradle and Maven,
  three arms; 126/126 unique runs retained and judged.
- Source mutations: 0/126 by before/after canonical digest.
- Native token telemetry: 126/126.
- Controller/public commitments and all seed/arm permutations independently
  reproduced.
- Arm-blind annotators agreed on 42/42 family, outcome and obligation labels;
  role/refusal nomenclature was adjudicated before the runs.

Artifact digests:

| Artifact | SHA-256 |
|---|---|
| retained run packets | `34047d0d22161c450272300a4f62820dce8a7cb559527faf22b570ad3fbfc757` |
| hidden judgments | `3f4214ab3c23f6216d5d1de629d527719fc9cdfd9e30750404ef080032fd3da6` |
| stored summary | `ac8a4b2d67b2c3260147a06141f2bb760d31b82cd06f843ea7a114938ac9b092` |
| fresh blind annotation | `3f3914de9f9339dba76ce46fd86583bb57da0da610dd28c6403587aa90dbdc2c` |
| verifier annotation | `eaf68b0e623bf46e92f19ab9c9872e2664a6c852575efc1f26466e62b0f04fc3` |
| pre-arm adjudication | `2b2b2f295f655febccce6336e39fd8fd02f416564656583020f820339c47aebe` |

## Recorded scores — not gate evidence

These are the exact stored judge results. They are reported for traceability,
not interpreted as a product ranking.

| Arm | accepted | audited/failed | positive | ambiguity | must-refuse | false complete |
|---|---:|---:|---:|---:|---:|---:|
| default | 15/42 | 7/42 | 0/14 | 9/14 | 6/14 | 2 |
| ast-index | 0/42 | 42/42 | 0/14 | 0/14 | 0/14 | 2 |
| Codeclew | 0/42 | 42/42 | 0/14 | 0/14 | 0/14 | 0 |

The recorded cost of this invalid series was still measured correctly:

- 6,946,958 input tokens;
- 81,068 output tokens;
- 882,746 noncached tokens (`input - cached + output`);
- 257 action calls;
- 2,783,001 ms aggregate worker wall time.

This cost is itself a gate-efficiency finding: a no-model preflight plus one
canary per specialized arm would have detected the invalid environment before
the full 126-run spend.

## Why the comparison is invalid

### 1. Real tool calls were rejected by the audit

Codex records shell calls as `/bin/zsh -lc '<payload>'`. The audit expected the
first executable to be `ast-index` or the frozen absolute `clew` binary and did
not unwrap the shell envelope.

- AST was actually called in 12/42 runs, but all 42 were labelled
  `AST_INDEX_NOT_USED`.
- Codeclew was actually called in 40/42 runs, with 124 total commands, but all
  42 were labelled `CODECLEW_PROOF_NOT_USED`.

An independent executable counterexample showed that the exact allowed
`clew projection --help` passes the audit directly and fails when represented
in the real `/bin/zsh -lc` event shape.

### 2. AST provenance was checked after deleting the repository

The temporary repository was destroyed before `audit()` checked whether a
bounded `sed` read referred to a preceding AST result. Therefore even a valid
AST-localized read could not satisfy `resolved.is_file()`.

### 3. Codeclew was denied its project-model dependencies

The workspace sandbox allowed the isolated repository but not writes to the
host Gradle and Maven caches. Raw events contain 16 cache permission failures.
Other calls used a misleading `--repo repository` path even though the process
already ran at the repository root. Seven calls reached a real product
boundary: multi-module Maven is unsupported. Four calls also reached the
authority layer with an isolated repository that had no committed Git `HEAD`.

No Codeclew semantic request completed successfully in the measured series, so
the series did not exercise the supported Gradle binder contour.

### 4. The judge required undefined oracle semantics

The common prompt listed four oracle classes without defining them. The hidden
controller required `EXTERNAL_SPEC` for every positive task. Nine default
positives had exact bindings, obligations and evidence but selected another
reasonable oracle class and were rejected. The blind annotations did not
preregister an oracle class.

Seven default runs were also falsely labelled `SOURCE_EDIT_ATTEMPT` because the
audit treated `2>/dev/null` as a source write; canonical source digests prove
that no source changed.

## What the series did teach us about the product

The following observations are diagnostic, not `GE1` claims:

- Agents attempted to use Codeclew in 40/42 runs, so discoverability at the
  binary-name level is not the primary problem.
- Agents spent many calls guessing subcommands and required flags. Raw events
  include invalid positional syntax, nonexistent subcommands and wrong repo
  paths. The product surface is not self-routing from a task.
- `prove map-edge-with-context` requires a workflow symbol and a matching test
  symbol. That is evidence-authority safety, but it leaves localization and the
  missing-oracle decision to the model.
- The remaining six corpus families have no certified task binder.

Thus S0 has localized the next product question at the boundary between the
previously accepted narrow E03 foundation and the missing product layer that
turns task intent into a typed, provable change goal. S0 itself supplies no new
claim that the foundation executes successfully in its generated repositories.

## Graph position

```text
E03 accepted narrow binder contour
  |
  v
E04-S0 executed and retained
  |
  +-- independent REJECT: infrastructure-invalid
  |
  X  E04 not accepted
     GE1 not evaluated
     E05 remains closed
```

Mechanically applying the corrupt scores would produce
`STOP_UNIVERSAL_EDITING`, but that branch is prohibited because the `E04`
receipt is invalid. The evidence graph therefore remains at accepted `E03`,
with a failed E04 attempt attached as research evidence.

## Product-first next line

Before another full experiment, implement one self-describing operation such
as:

```text
clew bind-task --repo . --intent "..."
```

For the first supported PTC family it must, in one invocation:

1. normalize FQNs and select semantic roots itself;
2. return the common `BOUND | AMBIGUOUS | REFUSED` schema;
3. emit exact bindings, obligations, provenance and preservation evidence;
4. separate binder-only external specification from the later materialization
   test oracle, without claiming an unsupported behavioral proof;
5. return explicit `UNSUPPORTED_FAMILY` for every other family.

Only after a no-model environment preflight and one fresh canary for each
specialized arm demonstrate the repaired operation should a new immutable
`E04-R1` series be created. The repair is bounded to shell-envelope parsing,
audit-before-teardown, non-mutating redirection handling, writable isolated
build caches, clean committed isolated repositories, exact CLI examples and
defined oracle semantics.

The decision-bearing rerun must use a new seed domain and new commitments.
Reusing the opened S0 tasks is allowed only for diagnostics, never for `GE1`.
S0 is a non-node diagnostic attempt: its `INFRA_ERROR` is not recorded as an
exhausted `E04 -> GE2` transition while the fresh R1 repair remains planned.
