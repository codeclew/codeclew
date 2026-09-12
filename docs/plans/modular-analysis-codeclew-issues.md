# Codeclew issues encountered during modular architecture analysis

Date: 2026-09-08. Status: analysis complete; no product fixes attempted.

This journal records observed limitations separately from defects and hypotheses.
The installed release launcher was used throughout. The preceding task reported
release 0.6.1; observed runtime identity is
`sha256:d922fbf00d7610e5f33c3677066d543ed11800e8373bb2d858b3a0e647cf63a1`.
Raw JSON and stderr are retained in the caller-owned private directory identified
as `modular-20260908` outside this repository, with file permissions 0600.
Artifact filenames below are relative to that directory, not public links.
Commands use `$REPO` for the checkout and `$CLEW` for the resolved installed
launcher. No diagnostic data has been sent or published.

## MCA-001 — Java saved-working-tree analysis is unsupported

- Observed: 2026-09-08, initial discovery; confirmed support limitation.
- Command: `$CLEW doctor repository --repo "$REPO" --working-tree`.
- Source selection: `WORKING_TREE`; discovery target
  `refs/heads/codex/kotlin-service-documentation`; no revision in discovery.
- Profiles: `java-17plus-gradle-read-only`, `java-21-gradle-read-only`.
- Expected scope: inspect the final saved source across relevant engines.
- Actual: Java contours are `UNSUPPORTED`, with
  `SELECT_SUPPORTED_WORKING_TREE_LANGUAGE`: working-tree source currently
  supports Kotlin/Gradle and Rust only. No Java admission was attempted.
- Impact: no compiler-backed Java-source evidence for saved edits in this run.
  Rust adapter source may still be inspected as Rust syntax; that does not
  establish javac behavior. No silent substitution with committed Java source.
- Artifact: `01-discovery.json`, `01-discovery.stderr`.
- Status: limitation retained in architectural evidence coverage.

## MCA-002 — Broad Rust context is partial and repeats source representations

- Observed: 2026-09-08; confirmed bounded-output limitation and usability cost.
- Command: `$CLEW context open --repo "$REPO" --target-ref
  refs/heads/codex/kotlin-service-documentation --language rust --profile
  rust-syntax --compilation 'cargo:crates/clew/Cargo.toml#clew#lib#clew'
  --operation analysis --intent '<architecture analysis>' --term Spring
  --term annotation --working-tree`.
- Admission: `PASS`, installed release, analysis only. Source: `WORKING_TREE`,
  base revision `a15a104268f4155c10165bd4cc758a9322ecf325`.
- Actual: `PARTIAL`, `UNSURE`, `CONDITIONAL_TASK`, `truncated=true`, with
  `VERIFY_CFG_AND_MACRO_EXPANSION` and `VERIFY_RUST_NAME_RESOLUTION` obligations.
  Exact source is available, but syntax references do not prove resolved calls.
  Source text also appears in full source projections and individual windows;
  the raw response is large enough to exceed a routine tool-output budget.
- Expected: bounded architecture context that exposes its limits; limits were
  exposed correctly. No completeness or semantic-resolution defect is claimed.
- Impact/workaround: retain raw output, show compact metadata and exact windows
  from that same output, narrow remaining questions, avoid re-reading live source.
  Initial request reports 23,076 ms; this single sample is not a benchmark.
- Artifacts: `02-rust-open.json`, `02-rust-open.stderr`.
- Status: limitation; redundant representation is a product-improvement candidate.

## MCA-003 — Kotlin analysis is admitted but not exhaustive

- Observed: 2026-09-08; confirmed evidence limitation, not admission failure.
- Command: the MCA-002 `context open` shape with `--language kotlin --profile
  kotlin-2.4.10-gradle-single`, compilations `:workers:kotlin/main`,
  `:workers:kotlin21/main`, `:workers:kotlin23/main`, and terms `Spring`,
  `annotation`.
- Admission: `PASS`; `WORKING_TREE`, same base revision and input snapshot as
  MCA-002. Actual result: `PARTIAL`, `UNSURE`, `CONDITIONAL_TASK`, with
  `VERIFY_PARTIAL_KOTLIN_BOUNDARIES`; a returned generated `copy` declaration
  has `GENERATED_OR_NO_SOURCE` rather than proven source identity.
- Project/compiler pairs: 2.4.10/2.4.10, 2.1.21/2.4.10, 2.3.0/2.3.0.
  Selecting a profile named 2.4.10 does not mean every selected compilation uses
  that analyzer, and it does not mean analysis of 2.1 uses an exact 2.1 compiler.
- Impact/workaround: use exact returned declarations and source windows only;
  retain compatibility and generated-source limits. Do not claim a full worker
  inventory or rerun the preceding task's tests.
- Performance: request reports 95,477 ms for three compilations. This is one
  observation, with no matched native control; it establishes no speedup.
- Artifacts: `03-kotlin-open.json`, `03-kotlin-open.stderr`,
  `05-kotlin-reader.json`, `05-kotlin-reader.stderr`.
- Status: limitation recorded; useful compiler-backed declarations were obtained.

## MCA-004 — Speculative identifier and accumulated terms yielded no source

- Observed: 2026-09-08; agent query mistake plus CLI usability observation.
- Command: `$CLEW context expand --session <rust-session> --from <context>
  --term KotlinCompilerFact --term JavaCompilerFact`.
- Profile/source/revision: as MCA-002.
- Expected: locate language fact contracts. `KotlinCompilerFact` was an
  agent-selected guess, not an established declaration.
- Actual: `unmatchedTerms=[kotlincompilerfact]`, `QUERY_COMPLETE` with respect
  to the bounded query, and no source windows; the Java enum identity was
  nevertheless returned. No source-availability defect is established.
- Workaround: stop that unmatched term; select the established Java enum with
  `nav expand --term JavaCompilerFact --file crates/clew/src/java_adapter_v2.rs
  --source`. Exact source was returned without reopening the session.
- Artifacts: `04-contracts.json`, `04-contracts.stderr`,
  `06-java-contract.json`, `06-java-contract.stderr`.
- Status: resolved for Java; the guessed Kotlin identity was abandoned.

## MCA-005 — Framework interpretation is coupled to compiler-version readers

- Observed: 2026-09-08; confirmed architectural coupling, not a runtime defect.
- Evidence: Kotlin source windows in `03-kotlin-open.json` show the same
  hierarchy-selection/controller/Feign rules in `SpringAnnotationReader21`,
  `SpringAnnotationReader23`, and `SpringAnnotationReader24`.
  `05-kotlin-reader.json` contains the 24 reader at lines 51–349, including
  Spring alias interpretation, extraction through FIR APIs, and runtime markers.
- Profile/source/revision: as MCA-003. Exact windows establish local source
  behavior; cross-version runtime equivalence is not claimed.
- Impact: changing framework rules requires maintaining version-specific source;
  sharing FIR-dependent code alone does not create a stable framework boundary.
- Workaround: architectural proposal in the companion analysis; no source edits.
- Error/nextAction: none; this is a design finding.
- Status: confirmed design debt.

## MCA-006 — Multiple mappings and legacy alias rules need semantic qualification

- Observed: 2026-09-08; hypothesis of version-dependent semantic mismatch.
- Evidence: `05-kotlin-reader.json` lines 81–92 records
  `MULTIPLE_REQUEST_MAPPINGS_ON_ELEMENT` but emits each mapping; lines 261–263
  apply a same-name legacy attribute override. The selected build configuration
  uses Spring Web 6.1.2 test artifacts.
- External reference: [Spring mapping documentation](https://docs.spring.io/spring-framework/reference/web/webmvc/mvc-controller/ann-requestmapping.html)
  describes using only the first mapping when multiple mappings occur on one
  element. [Spring annotation model](https://github.com/spring-projects/spring-framework/wiki/Spring-Annotation-Programming-Model)
  and [AliasFor API](https://docs.spring.io/spring/docs/6.2.4/javadoc-api/org/springframework/core/annotation/AliasFor.html)
  distinguish explicit, implicit and transitive aliases.
- Impact: preserving current output alone is insufficient to certify framework
  semantics for every library version. Existing boundaries may already make
  the extra entries conservative candidates; downstream treatment was not
  established here, so this is not a confirmed false-positive defect.
- Reproduction: same Kotlin context as MCA-005; a runtime fixture against each
  claimed Spring policy version remains necessary to resolve the hypothesis.
- Workaround/proposal: retain current boundaries in migration, qualify framework
  policies separately from compiler versions, and test expected Spring behavior.
- Status: open qualification obligation; no runtime test executed in this analysis.

## MCA-007 — Whole-snapshot freshness changes while writing the report

- Observed: 2026-09-08; expected lifecycle behavior with a usability limitation.
- Commands: `$CLEW change check-freshness --session <rust-session>` and the
  same command for the Kotlin session. Profiles/source/base are as MCA-002/003.
- Actual: both return `LIVE_CHANGED`, `headMatchesExpected=true`,
  `retainedEvidenceValid=true`, and
  `CAPTURE_NEW_SNAPSHOT_IF_CURRENT_EDITS_ARE_NEEDED`.
- Context: the task had added its journal to the captured repository. The
  freshness result does not enumerate which files changed, so it alone cannot
  establish that only documentation changed.
- Workaround: retain the captured snapshot as the analysis authority. A later
  native comparison of all 36 returned source windows found them identical at
  their recorded live line ranges. This checks cited windows, not whole-snapshot
  equality, dependency freshness or complete semantic coverage.
- Artifacts: `13-rust-freshness.json`, `14-kotlin-freshness.json`,
  `17-cited-window-freshness.json` and corresponding stderr files where present.
- Status: retained evidence valid; whole live snapshot intentionally not certified.

## MCA-008 — Invalid cross-domain digest comparison during verification

- Observed: 2026-09-08; agent verification mistake, not a product defect.
- Operation: a local script compared Codeclew source `contentRef.digest` values
  with direct SHA256 hashes of live files without establishing identical digest
  domains. All 11 comparisons differed; this did not prove source modification.
- Expected/actual: attempted freshness shortcut was invalid. No product error
  or remediation code was returned.
- Workaround: discard that comparison as evidence of change; perform the exact
  retained-window comparison described in MCA-007 instead. All 36 windows match.
- Artifacts: `16-cited-source-digests.json` (explicitly invalid comparison),
  `17-cited-window-freshness.json` (bounded corrected check).
- Status: resolved; the evidence index labels the discarded comparison.

## MCA-009 — Inherited registration identity may collide in documentation

- Observed: 2026-09-08; source-backed risk hypothesis, not a reproduced defect.
- Evidence: `05-kotlin-reader.json` preserves `beanClass` and `targetSymbol`;
  `11-docs-project.json` lines 546–568 derives documentation IDs from target and
  ordinal, and lines 634–635 sorts/deduplicates by ID. Concrete bean owner is
  not visible in that ID construction.
- Profile/source/base: Rust syntax and Kotlin evidence as MCA-002/003.
- Impact: two concrete beans inheriting one target with equal ordinals might
  collapse in documentation. Upstream facts and a full end-to-end reproduction
  are needed to establish whether that case reaches this projection.
- Reproduction still required: fixture with two concrete bean owners sharing
  one inherited handler; verify distinct registrations survive projection.
- Workaround/proposal: treat owner, implementation and annotation occurrence
  as separate identities in the package contract and migration checks.
- Status: open hypothesis; no correction or runtime test in this analysis.

## Artifact capture times

These UTC timestamps are local artifact modification times, not a claim that
the diagnostic payload itself supplied an event timestamp. Reused artifacts
can support findings made later in the analysis.

| Issue | Evidence captured (UTC) | Artifact |
|---|---|---|
| MCA-001 | 2026-09-08T08:23:15+00:00 | `01-discovery.json` |
| MCA-002 | 2026-09-08T08:23:51+00:00 | `02-rust-open.json` |
| MCA-003 | 2026-09-08T08:25:36+00:00 | `03-kotlin-open.json` |
| MCA-004 | 2026-09-08T08:25:00+00:00 | `04-contracts.json` |
| MCA-005 | 2026-09-08T08:26:14+00:00 | `05-kotlin-reader.json` |
| MCA-006 | 2026-09-08T08:26:14+00:00 | `05-kotlin-reader.json` |
| MCA-007 | 2026-09-08T08:30:13+00:00 | `13-rust-freshness.json` |
| MCA-008 | 2026-09-08T08:35:20+00:00 | `16-cited-source-digests.json` |
| MCA-009 | 2026-09-08T08:29:20+00:00 | `11-docs-project.json` |
