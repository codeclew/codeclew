# D01: neutral corpus generator and hidden manifest

Date: 2026-08-09

Outcome: `SUCCESS`

Independent verdict: `ACCEPT`

## Evidence delta

`semantic-corpus/0.2` extends the existing Rust corpus crate rather than adding
a controller service. A seed now deterministically selects neutral vocabulary,
Gradle or Maven layout, flat or module topology, a decoy declaration, and one
of `positive`, `ambiguous`, or `must-refuse`. The agent package contains task
and repository inputs; obligations, refusal reasons and oracle artifacts remain
in the controller package.

`verify-hidden` checks:

- exact public/hidden task, variant, layout and generator identity;
- canonical public manifest digest;
- canonical recursive `(normalized relative path, bytes)` repository digest;
- controller-manifest commitment;
- exact controller tree equal to `manifest.json + hiddenArtifacts`;
- every hidden artifact's committed SHA-256;
- refusal/positive oracle shape;
- no symlinks, non-regular entries, absolute paths or parent traversal.

Focused tests cover deterministic unseen-seed replay for Gradle and Maven,
layout/vocabulary/decoy variation, all three variants, public-oracle isolation,
manifest/refusal tampering, repository-source mutation and hidden-test
mutation.

## Independent verification

The first verifier pass returned `REJECT` despite all six initial tests being
green. Two executable mutations still passed `verify-hidden`:

1. changing generated Kotlin source after manifest publication;
2. changing the controller-owned hidden test after commitment.

The repair bound both byte sets. Delta-only replay confirmed both mutations now
exit non-zero. The final crate suite passes `8/8`; `git diff --check` is clean.

## Boundary

D01 establishes deterministic package generation, separation and integrity.
It does not establish the six editing families, ecological weights, sealed
evaluation entropy, worker correctness, applicability, token savings or
wall-time savings. Those remain D02/E04 responsibilities.

## Next edge

`D01 -> D02` is open. `E01` remains closed until D02 is independently
accepted.

## Gate efficiency

The verifier initially began a 320-seed sweep; orchestration stopped it because
the marginal evidence was low. The useful gate consisted of two adversarial
mutations, each exposing a real false acceptance, followed by the same two
delta-only checks. No full workspace suite, network build, new dependency or
separate harness was needed.
