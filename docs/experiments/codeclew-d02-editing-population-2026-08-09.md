# D02: editing families and ecological population

Date: 2026-08-09

Outcome: `SUCCESS + NARROW_POPULATION`

Independent verdict: `ACCEPT` after two contract corrections

## Evidence delta

The executable `codeclew-editing-population/0.1` contract preregisters 42
future withheld slots: seven structural families, each crossing positive,
ambiguous and must-refuse with Gradle and Maven. Every family declares required
semantic obligations, must-refuse boundaries and decoy dimensions.

The 42 slots are enumerated as typed `(family, variant, buildSystem, ordinal)`
records rather than inferred from counts. A versioned SHA-256 protocol derives
one reproducible seed from the frozen binder digest, population digest and slot;
materialization must prove that the resulting manifest identity equals its
slot. A wrong variant cannot silently satisfy the matrix.

This is `semantic-editing-corpus/0.1`, a frozen E04 materializer protocol, not
the D01 smoke generator `semantic-corpus/0.2`. The latter deliberately has no
six-family source templates and is not an executable path for these slots.
Under the editing protocol the variant is an explicit `FROZEN_SLOT` input; it
is never inferred from seed bits. Implementing or invoking the family
materializer before the binder freeze would violate D02 rather than complete it.

Exact tasks and seeds do not exist yet. E04 derives them only after the binder
source tree and this population specification are frozen. The product worker
does not depend on the corpus crate, public packages cannot contain oracle
data, and demonstrative samples are excluded from evaluation.

## Why the outcome is narrow

The repository has no independently sampled, double-annotated ecological
population of public Kotlin/JVM tasks with verified provenance. Existing
literature notes and historical repositories are not promoted into weights.
The frozen weighting is therefore `BALANCED_STRUCTURAL_SAFETY`, not an estimate
of typical-task prevalence. Results on this population may test binding,
refusal and generalization across the seven declared structures, but may not
support a claim about “most real tasks”.

The annotation protocol requires two arm-blind labels and third-party
adjudication before execution. It is a construction rule, not a fabricated
completed annotation dataset.

## Executable checks

The Rust validator rejects fewer than six families, fewer than 36 slots,
missing variant/build combinations, empty obligations/refusal/decoy sets,
duplicate or missing slots, wrong materialized identity, premature seed
materialization, unsafe oracle sharing, single-annotator labels
and any attempt to convert unavailable ecology into a typical-task claim.

## Boundary and next edge

D02 does not implement the E04 family source templates, generate final tasks,
run a model, prove applicability, measure
tokens or validate a binder. It opens E01 only on `NARROW_POPULATION`; universal
editing remains impossible unless a later independently sampled ecological
population is added before the full-corpus claim.

## Gate efficiency

An initial delegated attempt was stopped because it produced no artifact
within the stop-loss window. The replacement is one data contract, one small
validator module and focused mutation tests; it adds no service, controller or
source template implementation.
