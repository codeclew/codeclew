# E02: goal-wide transaction safety and completeness falsifier

Date: 2026-08-09

Outcome: `SUCCESS + STOP_FALSE_COMPLETENESS`

Independent verdict on `COMPLETE_FOR`: `REJECT` after the single permitted
bounded repair and delta re-verification.

## Product delta retained

E02 produced two independently executable safety improvements which do not
depend on the rejected completeness prototype.

1. The old heuristic status is no longer called `COMPLETE_TASK`. Contexts now
   report `LEGACY_HEURISTIC_READY` or an explicit partial state. `task-apply`
   refuses that path by default and requires the deliberate
   `--allow-legacy-heuristic` compatibility opt-in.
2. A transaction can retain every required Thread IR instead of only the first
   root. Before commit, all roots must share the transaction snapshot and have
   unique identities. If HEAD moves, Codeclew rebuilds every required root,
   compares every semantic ReadSet and their union, and reports which thread
   became stale.

This removes two concrete unsafe behaviours: heuristic retrieval can no longer
silently advertise theorem-like completeness, and a concurrent change to a
second required root can no longer be ignored by first-thread-only replay.

## Demonstration

| Scenario | Executable result |
| --- | --- |
| A legacy context is passed to `task-apply` without an opt-in | Refused with `INCOMPLETE_SEMANTIC_ANALYSIS`; the error names `--allow-legacy-heuristic`. |
| Two required semantic roots are stored and only the second root changes before commit | Refused with `STALE_REQUIRES_RESLICE`; evidence identifies the second thread. |
| Required thread IDs are duplicated, snapshots disagree, or the primary thread does not match | Transaction validation refuses before commit. |
| A synthetic `COMPLETE_FOR` packet invents internally consistent source anchors, family edges and an oracle | The prototype incorrectly accepted it; independent verification rejected E02 completeness. |

The first three rows are retained product behaviour. The fourth row is the
falsifier; the prototype which exposed it was removed from the product module
tree and was not committed as an authorization mechanism.

## Why `COMPLETE_FOR` was rejected

The repaired prototype checked exact family obligations, source-shaped
anchors, graph relations, canonical root commitments, boundaries, budgets and
oracle linkage. Those checks establish internal consistency only.

The caller could still construct all inputs itself: fictitious source origins,
source hashes, graph edges, derived obligations and a validation record whose
hash was publicly computable. Replay used the same caller-provided packet and
therefore confirmed the fiction. No real source or independently issued
validation artifact had to exist.

The missing invariant is an authority boundary:

> Root, edge and oracle receipts used by `COMPLETE_FOR` must be issued by the
> actual Kotlin/index/validation workers and must not be synthesizable by the
> caller asking for completeness.

Without that invariant, more cross-checks only make a forged packet more
elaborate; they do not turn it into evidence.

## Verification evidence

- `cargo test -p sthread`: all unit and integration suites passed, including
  112 library tests, Kotlin 2.1, Maven, projection, transaction and concurrency
  tests.
- `cargo test -p sthread --test agent_context -- --nocapture`: passed; includes
  the default legacy-apply refusal.
- `cargo test -p sthread --test concurrency_matrix change_in_second_required_thread_forces_reslice -- --nocapture`:
  passed; the second-root replay regression completes in about 24 seconds.
- Independent first pass: `REJECT`, because role-labelled, disconnected and
  self-attested inputs could pass.
- Single bounded repair: exact D02 obligations, exact family relation chains,
  canonical commitments, strict boundary refusal and derived evidence linkage.
- Independent delta pass: `REJECT`; the remaining executable counterexample
  forges the whole internally consistent evidence packet without reading a
  source or consuming an independently issued oracle.

## Gate efficiency

The first independent pass found a real false-completeness class. One bounded
repair eliminated shallow relabelling and omission attacks. The required single
delta pass then showed that the residual problem is architectural, not another
missing predicate. Work stopped there; no second repair loop, controller or
materializer was added.

## Graph consequence

Per the approved E02 measured branch, this is
`SUCCESS + STOP_FALSE_COMPLETENESS`. Universal editing materialization must not
open from this result. The retained goal-wide transaction changes are useful
independently, but they do not prove applicability, correctness, token savings,
time savings or a grep-free advantage.
