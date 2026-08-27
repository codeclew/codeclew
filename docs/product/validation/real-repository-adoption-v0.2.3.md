# Real-repository adoption verdict for v0.2.3

## Verdict

The v0.2.3 slice is accepted as a bounded improvement to agent navigation and
release observability. It does not establish that syntax-backed Codeclew
outperforms ordinary search for every natural-language request.

The Rust self-hosting task completed through the managed conditional mutation
path. The Python task remained read-only because the target repository's local
agent contract prohibits Codeclew prepare and publish. Both profiles retained
`PARTIAL/UNSURE` evidence and required project-native verification.

## Frozen tasks

1. Rust: expose the exact compiled product version in canonical capabilities
   JSON and assert it through the managed CLI.
2. Python: trace the guard and focused test for rejecting an archived quick-add
   focus in a real backend repository.

Both tasks used exact repository revisions, explicit language/compilation
selectors, clean detached worktrees where required, and focused native tests.
No target source ref changed before an acknowledged publish.

## Observations

### Rust

- The native baseline completed in 0.24 seconds.
- Ordinary search identified two relevant files whose full size was 101,266
  bytes.
- An initial bin-only selector correctly excluded the library implementation;
  changing to explicit lib plus integration-test targets prevented an
  accidental cross-target assumption.
- Before the projection repair, a 27,125-byte bounded context identified the
  large integration-test declaration but omitted the inner capabilities
  assertion.
- After the repair, a 33,539-byte context with 10,728 source bytes contained
  both the canonical capabilities constructor and the inner managed-CLI
  assertion.
- The two-file product-version candidate passed its focused Cargo validation,
  strict publication was refused, and acknowledged conditional publication
  succeeded.

### Python

- The focused native pytest completed in 1.32 seconds on the final observed
  revision.
- Ordinary text search found the implementation and test in one command, but
  produced a broad result set; the two full relevant files totalled 272,929
  bytes.
- Before the repair, the bounded context selected the focused test but omitted
  its inner assertion and the production guard.
- After the repair, a symbol-seeded query returned both guard and assertion in
  a 21,212-byte context containing 2,554 source bytes.
- A warm query using only natural terms returned the complete test assertion
  in 6.35 seconds, but still did not select the production guard. Literal and
  local-identifier recall therefore remains incomplete.

## Accepted product change

Exact syntax declaration facts already carried start and end byte offsets, but
source projection discarded the end offset and always emitted a fixed window
around the declaration start. v0.2.3 preserves the exact range and emits the
complete declaration when it fits the existing 32 KiB source budget. Invalid,
oversized, or non-boundary ranges keep the previous bounded fallback.

The change is language-neutral, adds no endpoint, changes no publication
policy, and does not upgrade semantic certainty. A regression proves that a
matched declaration body beyond the old forty-line tail remains visible.

Canonical capabilities now also include `productVersion`, bound to the exact
compiled package version and asserted through the managed CLI.

## Deferred findings

1. Index string literals and local identifiers, or add an equally bounded
   lexical recall layer, so natural terms can select the production declaration
   without a prior ordinary search.
2. Make managed GC handle closed sessions whose target ref advanced after the
   session completed. The current implementation fails closed and manual state
   deletion remains unsupported.
3. Reduce content-addressed cold rebuild cost for lifecycle-only commands after
   a product commit without weakening runtime identity.
4. Update the Python repository's local agent contract only through a separate
   repository-owned decision before attempting conditional mutation there.

These findings do not block the v0.2.3 bounded navigation improvement, but they
block any broader claim that Codeclew already dominates default exploration on
unseeded natural-language tasks.
