# S4K Private Kotlin Pilot Runbook

This runbook is the only supported execution order for the closed S4K pilot.
It is a macOS-only qualification because the broker and warm audit are bound to
the checked Seatbelt policy. Linux may run the fast self-tests, but it may not
publish a qualified adoption result until an equivalent sealed audit adapter
exists.

## One-shot experiment authority

Every attempt uses one brand-new `mktemp` root and one shell owner. There is no
resume, import, or automatic recovery. Do not use repository-local private
outputs. Every regular private file is mode `0600`; every private directory is
mode `0700`.

```sh
export CODECLEW_REPO=/absolute/path/to/codeclew
export S4K_EXPERIMENT_PARENT=/absolute/private/parent
export CODEX_BIN=/absolute/path/to/codex
export GIT_BIN="$(/usr/bin/xcrun --find git)"

# The frozen R2 services intentionally have no Maven wrapper. The runner
# resolves `mvn` from this ambient PATH once, pins its resolved executable and
# byte digest, then admits only that parent directory to semantic preparation.
command -v mvn >/dev/null

umask 077
install -d -m 0700 "$S4K_EXPERIMENT_PARENT"
export S4K_EXPERIMENT_ROOT="$(mktemp -d "$S4K_EXPERIMENT_PARENT/codeclew-s4k.XXXXXXXX")"
chmod 0700 "$S4K_EXPERIMENT_ROOT"
export S4K_PRIVATE_DIR="$S4K_EXPERIMENT_ROOT"
export S4K_STATE_DIR="$S4K_EXPERIMENT_ROOT/codeclew-state"
export S4K_TMP_DIR="$S4K_EXPERIMENT_ROOT/tmp"
install -d -m 0700 "$S4K_STATE_DIR"
install -d -m 0700 "$S4K_TMP_DIR"
export CODECLEW_HOME="$S4K_STATE_DIR"
export TMPDIR="$S4K_TMP_DIR"
export S4K_SHAPE_REVIEW_MANIFEST="$S4K_PRIVATE_DIR/shape-review.json"
export S4K_IMPLEMENTATION_REVIEW_MANIFEST="$S4K_PRIVATE_DIR/implementation-review.json"
export S4K_VALUE_REVIEW_MANIFEST="$S4K_PRIVATE_DIR/value-review.json"
cd "$CODECLEW_REPO"

export S4K_RUNNER="$CODECLEW_REPO/tools/run_thread_kotlin_pilot.py"
export S4K_BUILDER="$CODECLEW_REPO/tools/build_thread_kotlin_shape_oracle.py"
export S4K_WARM="$CODECLEW_REPO/tools/run_thread_kotlin_warm_audit.py"
export S4K_G1K="$CODECLEW_REPO/docs/plans/evidence/thread-kotlin-descriptor-gate.json"
```

Private files must be mode `0600`; private directories must be mode `0700`.
The frozen private corpus must be
`$S4K_PRIVATE_DIR/corpus.json`; the frozen private benchmark must be
`$S4K_PRIVATE_DIR/benchmark.json`. They are copied from the approved private
authority, never regenerated or edited for an arm.

## Pre-paid compiler oracle review and build

First emit the review ingredients. An independent reviewer compares these
ingredients and the builder implementation, then creates
`$S4K_SHAPE_REVIEW_MANIFEST`. The builder never creates its own PASS review.
The closed review binds the emitted `localModuleManifest`, pinned `gitDigest`
and `mavenDigest`, closed `gitEnvironmentDigest`, builder/runner/G1K/test
digests, and public fixture authority. Any changed byte requires a new
independent review.

```sh
python3 -I -S "$S4K_BUILDER" review-inputs \
  --g1k-evidence "$S4K_G1K" \
  --clew "$CODECLEW_REPO/clew" \
  --git "$GIT_BIN" \
  --pilot-runner "$S4K_RUNNER" \
  --experiment-root "$S4K_EXPERIMENT_ROOT" \
  > "$S4K_PRIVATE_DIR/shape-review-inputs.json"
chmod 0600 "$S4K_PRIVATE_DIR/shape-review-inputs.json"

python3 -I -S "$S4K_BUILDER" build \
  --private-corpus "$S4K_PRIVATE_DIR/corpus.json" \
  --private-benchmark "$S4K_PRIVATE_DIR/benchmark.json" \
  --g1k-evidence "$S4K_G1K" \
  --clew "$CODECLEW_REPO/clew" \
  --git "$GIT_BIN" \
  --shape-oracle "$S4K_PRIVATE_DIR/shape-oracle.json" \
  --attestation "$S4K_PRIVATE_DIR/shape-attestation.json" \
  --review-manifest "$S4K_SHAPE_REVIEW_MANIFEST" \
  --pilot-runner "$S4K_RUNNER" \
  --experiment-root "$S4K_EXPERIMENT_ROOT"
```

## Phase 1: prepare

`prepare` seals every input and executable digest, opens the ten immutable
threads, proves the private broker policy, and creates authority/oracle once.
Neither final path nor any pending ledger may exist before this command. A
partial publication makes the whole experiment terminal and quarantined.

```sh
python3 -I -S "$S4K_RUNNER" prepare \
  --experiment-root "$S4K_EXPERIMENT_ROOT" \
  --private-corpus "$S4K_PRIVATE_DIR/corpus.json" \
  --private-benchmark "$S4K_PRIVATE_DIR/benchmark.json" \
  --g1k-evidence "$S4K_G1K" \
  --clew "$CODECLEW_REPO/clew" \
  --codex "$CODEX_BIN" \
  --model gpt-5.6-sol \
  --reasoning-effort high \
  --private-shape-oracle "$S4K_PRIVATE_DIR/shape-oracle.json" \
  --private-shape-attestation "$S4K_PRIVATE_DIR/shape-attestation.json" \
  --shape-oracle-review-manifest "$S4K_SHAPE_REVIEW_MANIFEST" \
  --shape-oracle-builder "$S4K_BUILDER" \
  --warm-audit-runner "$S4K_WARM" \
  --private-authority "$S4K_PRIVATE_DIR/pilot-authority.json" \
  --private-oracle "$S4K_PRIVATE_DIR/pilot-oracle.json"
```

Before any paid arm, an independent implementation reviewer must create
`$S4K_IMPLEMENTATION_REVIEW_MANIFEST`. Its closed body is:

```text
schema, protocolDigest, runnerDigest, brokerDigest, publicVerifierDigest,
localModuleManifest, localModuleManifestDigest, answerSchemaDigest,
warmAuditAdapterDigest, shapeOracleBuilderDigest,
verdict="PASS", findings=[], authorityDigest
```

Every digest is copied exactly from the prepared authority. `authorityDigest`
is SHA-256 of canonical JSON for the other fields. The reviewer must refuse
PASS if any P0/P1 finding remains. Any harness edit after this review requires
a new manifest and a new preparation.

### No-paid dry run boundary

For a no-paid dry run, run all fast self-tests, the oracle review/build, Phase
1, and the independent implementation review, then stop. Do not invoke
`execute`. This proves CLI closure, compiler authority, create-once publication,
fail-stop resource-ledger behavior, and the Seatbelt canaries without starting a Codex
arm.

```sh
python3 -I -S "$S4K_RUNNER" --help >/dev/null
python3 -I -S "$CODECLEW_REPO/tools/test_run_thread_kotlin_pilot.py" -q
python3 -I -S "$CODECLEW_REPO/tools/verify_thread_kotlin_pilot.py" --self-test
python3 -I -S "$S4K_BUILDER" --self-test
python3 -I -S "$S4K_WARM" --self-test
```

## Phase 2: execute (paid boundary)

This is the only paid step. It runs exactly twenty arms in the sealed
alternating order. There is no retry or resume. Cancellation or ambiguous
transport/audit state makes the experiment terminally invalid.
Before the first arm, the runner atomically creates the root-scoped
`.codeclew-s4k-execute-admission.json` marker. It is never removed. A second
`execute` call is refused even if it names a different output, and both draft
and publish require the run path bound by that marker.

```sh
python3 -I -S "$S4K_RUNNER" execute \
  --experiment-root "$S4K_EXPERIMENT_ROOT" \
  --private-authority "$S4K_PRIVATE_DIR/pilot-authority.json" \
  --private-oracle "$S4K_PRIVATE_DIR/pilot-oracle.json" \
  --implementation-review-manifest "$S4K_IMPLEMENTATION_REVIEW_MANIFEST" \
  --private-output "$S4K_PRIVATE_DIR/pilot-run.json"
```

## Phase 3: warm

The checked adapter creates the private measured attestation. The runner then
verifies that exact attestation and creates the pilot warm result.

```sh
python3 -I -S "$S4K_WARM" \
  --private-authority "$S4K_PRIVATE_DIR/pilot-authority.json" \
  --private-oracle "$S4K_PRIVATE_DIR/pilot-oracle.json" \
  --source-repo "$CODECLEW_REPO" \
  --private-output "$S4K_PRIVATE_DIR/warm-attestation.json"

python3 -I -S "$S4K_RUNNER" warm \
  --experiment-root "$S4K_EXPERIMENT_ROOT" \
  --private-authority "$S4K_PRIVATE_DIR/pilot-authority.json" \
  --private-attestation "$S4K_PRIVATE_DIR/warm-attestation.json" \
  --private-output "$S4K_PRIVATE_DIR/pilot-warm.json"
```

## Phase 4a: project draft

Draft recomputes all private scores from the hidden oracle and raw arm
provenance, performs terminal strict resource cleanup, and creates a closed
private review input. It does not publish checked evidence.

```sh
python3 -I -S "$S4K_RUNNER" project draft \
  --experiment-root "$S4K_EXPERIMENT_ROOT" \
  --private-authority "$S4K_PRIVATE_DIR/pilot-authority.json" \
  --private-oracle "$S4K_PRIVATE_DIR/pilot-oracle.json" \
  --private-run "$S4K_PRIVATE_DIR/pilot-run.json" \
  --private-warm "$S4K_PRIVATE_DIR/pilot-warm.json" \
  --implementation-review-manifest "$S4K_IMPLEMENTATION_REVIEW_MANIFEST" \
  --private-draft-output "$S4K_PRIVATE_DIR/pilot-draft.json"
```

An independent value reviewer reads that draft and creates
`$S4K_VALUE_REVIEW_MANIFEST` with exactly:

```text
schema, pilotAuthorityDigest, runDigest, warmAttestationDigest,
draftMetricsDigest, benchmarkDigest, verdict="PASS", findings=[],
authorityDigest
```

The values must equal the private draft/prepared authority. The value reviewer
must not receive the hidden compiler oracle or source bodies.

## Phase 4b: project publish

Publish reloads all private inputs, recomputes the draft byte-for-byte, verifies
both independent review manifests, and only then creates aggregate checked
evidence. No input or output path may alias another path, and the checked output
must not already exist.

```sh
python3 -I -S "$S4K_RUNNER" project publish \
  --experiment-root "$S4K_EXPERIMENT_ROOT" \
  --private-authority "$S4K_PRIVATE_DIR/pilot-authority.json" \
  --private-oracle "$S4K_PRIVATE_DIR/pilot-oracle.json" \
  --private-run "$S4K_PRIVATE_DIR/pilot-run.json" \
  --private-warm "$S4K_PRIVATE_DIR/pilot-warm.json" \
  --private-draft "$S4K_PRIVATE_DIR/pilot-draft.json" \
  --implementation-review-manifest "$S4K_IMPLEMENTATION_REVIEW_MANIFEST" \
  --value-review-manifest "$S4K_VALUE_REVIEW_MANIFEST" \
  --checked-output "$CODECLEW_REPO/docs/plans/evidence/thread-kotlin-pilot.json"
```

Any crash, cancellation, residual process group, partial output, pre-existing
ledger, or `openInFlight` marker is terminal. Quarantine and preserve the whole
`$S4K_EXPERIMENT_ROOT` for operator diagnosis. Do not delete, clean, import,
resume, or reopen anything from it, and never restart a paid arm. Start the
runbook from the first `mktemp` command with a new root and newly reviewed
authority bytes.
