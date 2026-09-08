# Modular analysis and reader comprehension — 0.7.0

Status: complete; v0.7.0 published and verified. Base: v0.6.2 (`e6093f3`).

## Authorized outcome

Deliver built-in language/framework modules, explicit worker JVM selection,
actionable Kotlin option handling, and service documentation accepted by an
independent reader. Release the verified result. Kotlin 1.9 gets no new engine
in this change. Preserve existing language facts, source bindings, coverage,
and supported operations. Private service evidence stays outside this repository.

## Implementation plan and acceptance

1. **Runtime and module contracts.** Add explicit built-in module identities,
   compiler/toolchain requirements, fact capabilities and schema compatibility.
   Register the existing Java 17+ and Kotlin analyzers through this contract.
   Select and validate the worker JVM before spawning; keep project build JDK
   selection separate. A Java 17 project must coexist with a JDK 21 Kotlin
   worker, and an unsuitable worker runtime must produce an actionable error.
2. **Shared Spring interpretation.** Define portable annotation, value,
   definition, origin, hierarchy and coverage facts. Extract them through small
   compiler-specific bridges. Move Spring interpretation to one Rust package;
   use an explicit compatibility projection while updating consumers. Compare
   existing and new outputs before removing duplicate rules. Preserve calls,
   bodies, symbols, inherited bean identities, defaults, aliases and boundaries.
3. **Kotlin compiler options.** Preserve native project arguments. Accept
   supported options under an explicit policy; otherwise identify the rejected
   option and compiler/engine context with a concrete next action. Do not
   silently claim equivalent semantics after dropping unknown semantic flags.
   As explicitly requested, ignore `-Xannotation-default-target=param-property`
   for analysis of Kotlin 1.9 projects and report that normalization. No new
   Kotlin 1.9 compiler pack is introduced.
4. **Documentation comprehension.** Run an independent reader on the current
   generated service/scenario documents. Ask it to recover purpose, inputs,
   state changes, successful/error/retry paths, contracts and system boundaries
   using only reader-visible material. Improve both generator/authoring guidance
   and the actual sample documents. Replace opaque, excessive step references
   with useful progressive disclosure. Retain essential contracts and evidence.
   Repeat independent review after material corrections until no blocking
   comprehension, excess-detail, missing-scope or unsupported-claim finding
   remains. Store private reviews and service material outside the repository;
   publish only a sanitized evaluation description and outcome.
5. **Verification and release.** Run focused Rust, JVM worker, protocol, CLI,
   rendering and skill checks appropriate to each change. Run the complete CI
   gate after integration, repository privacy checks before publication, and
   the release workflow for all supported platforms. Verify published assets
   and main CI before reporting completion.

## Scope and execution constraints

- Use native source-development tools in the clean release worktree; preserve
  the original checkout's unrelated and historical modifications.
- Start with built-in packages and the existing worker transport. Independent
  installation, a plugin marketplace, native dynamic libraries, a universal AST
  and Kotlin 1.9 engine implementation are outside this release.
- Package identity includes implementation/schema and relevant compiler inputs;
  framework derivation retains its own provenance and coverage. Missing inputs
  never become a successful empty framework catalogue.
- Unknown options may be supported when their handling is established, or
  rejected with an explicit explanation. Version labels alone do not establish
  compiler-plugin or cross-engine compatibility.
- Independent reader acceptance is bounded to explicit services and questions;
  it is not proof of complete documentation of every service or runtime behavior.
- Record concrete progress after each implementation slice. If a line of work
  yields no artifact, confirmed test fact or resolved blocker for 30 minutes or
  100 tool calls, narrow that slice rather than adding process or scope.

## Progress

- [x] Inspect v0.6.2 and the earlier modular architecture proposal.
- [x] Create an isolated implementation branch and dispatch baseline reader review.
- [x] Runtime and built-in module contracts.
- [x] Shared facts and Spring interpretation with equivalence checks.
- [x] Actionable Kotlin option handling and 1.9 normalization.
- [x] Revised generated documents accepted by an independent reader.
- [x] Full local verification and publication privacy check.
- [x] Public release and hosted CI.

## Confirmed implementation evidence

- Same-input migration comparisons preserved Spring entries on javac 17/21 and
  Kotlin 2.1/2.3/2.4 before removing all four compiler-side Spring readers.
  Portable extraction additionally reports unavailable annotation defaults in
  older Kotlin metadata; it does not invent values to match legacy silence.
- A live Kotlin 1.9 Maven documentation fixture passed with JAVA_HOME selecting
  JDK 17 and CODECLEW_WORKER_JAVA_HOME selecting JDK 21. The fixture includes
  the explicitly ignored annotation-target option.
- Regression checks cover annotation versus declaration-type boundaries, actual
  Maven runtime parsing, independent project-JDK cache identity/invalidation,
  private option-value omission and compiler-bound framework provenance.
- Re-analysis of the private five-service sample completed without unresolved
  services and preserved 65 discovered entrypoints. The selected documentation
  remains 10 service operations plus 3 scenarios; 55 entries are explicit gaps.
- Reader review found missing duplicate/stale rules and an unsupported assumption
  that a mapper loaded a persisted quantity input. Source-checked corrections
  passed a fresh independent reader review after a follow-up correction made
  nullable time/ID precedence explicit. Private names, source and review
  artifacts remain outside the product repository.

- The complete `scripts/ci-verify.sh` gate passed locally. Seven live worker
  regressions that initially encountered changing distribution manifests passed
  again after the trusted distributions were regenerated. No failing check was
  suppressed. Staged repository privacy checks passed.

## Independent reader acceptance

A fresh reader received only the final generated bundle and its README, without
previous reviews, authoring scripts, source repositories or compiler caches.
It answered six task groups: service responsibilities; warehouse import and
partial completion; stock ordering, outcomes and quantity; outbox recovery;
interface/nested payload discovery; and excess detail versus selected scope.
The final verdict was **PASS**, with no actionable acceptance blockers.

Source auditing in the preceding correction pass supplied evidence for the
changed claims. The final reader verdict establishes comprehension of the
selected example, not independent source correctness or deployed behavior.
Browser access was unavailable: emitted links and handlers were inspected
statically, and visual layout and real browser clicks remain unverified.
Markdown still repeats some schema caveats; this was judged non-blocking.

## Publication result

Release commit `b64f1aa` passed [hosted CI](https://github.com/codeclew/codeclew/actions/runs/34279393170).
The [release workflow](https://github.com/codeclew/codeclew/actions/runs/34279395404)
passed qualification, all three platform builds and publication.
[Codeclew v0.7.0](https://github.com/codeclew/codeclew/releases/tag/v0.7.0)
contains 14 assets: the core and optional Kotlin 2.3 archives for macOS arm64,
macOS x86_64 and Linux x86_64, the installer, and their seven checksum files.
The downloaded checksum contents match the published asset SHA-256 digests.
The private portable documentation bundle passed archive-integrity and local
link checks; source repositories and local compiler caches are excluded.
