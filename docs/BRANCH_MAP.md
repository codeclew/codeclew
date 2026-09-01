# Codeclew Branch Map

This document is the durable inventory of product work, frozen research, and
Git worktree state. Its purpose is to prevent useful changes from being lost
in experimental branches and to prevent research histories from being merged
wholesale into the product.

## Authority and snapshot

- Snapshot date: 2026-09-01.
- Comparison authority: `origin/main` at
  `7f75a3994910fbdf09ce9438ab93f97ed403c96d`.
- Inventory before creating this document: 51 local branch refs.
- Six branch tips were exact ancestors of `origin/main`; 45 were not.
- `feat/agent-skill-install` was not an ancestor, but its one patch was already
  present in `origin/main` with a different commit identity.
- Git recorded 134 valid and 63 prunable worktrees. The valid count includes
  Codeclew-managed sessions and benchmark worktrees; it is not a count of
  active feature branches.
- The branch `codex/branch-map-20260901`, created for this document, is excluded
  from the 51-branch snapshot.

Counts in this document must not be added across branches. Many research tips
share the same unmerged commit stack.

### Cleanup applied after the snapshot

The safe cleanup requested on 2026-09-01 produced this current local state:

- 63 missing-directory worktree registrations were pruned; no prunable
  registrations remain.
- Two clean worktrees were removed: the Kotlin marketing evidence worktree
  (only ignored build outputs remained) and the preliminary evidence site
  worktree (no local files remained).
- Five redundant local branch refs were removed:
  `codex/cas-recovery-20260901`,
  `codex/kotlin-marketing-evidence-20260901`,
  `codex/preliminary-evidence-site-20260831`, `codex/roadmap-c1`, and
  `feat/agent-skill-install`.
- 47 local branch refs and 132 valid worktrees remain, including this map
  branch and Codeclew-managed session worktrees.
- `research/identity-docs-pilot` and `research/semantic-support-pilot` remain
  checked out because their worktrees contain untracked research protocols,
  source, and results. Their integrated branch tips are not sufficient reason
  to discard those files.
- Release tags and remote branch refs were preserved. Neither is disposable
  worktree metadata, and deleting them would affect release history or other
  checkouts.

## Status vocabulary

| Status | Meaning | Default action |
|---|---|---|
| `INTEGRATED` | The recorded tip is an ancestor of `origin/main`. | Branch ref may be removed after worktree and evidence checks. |
| `PATCH_EQUIVALENT` | The commit identity differs, but Git patch comparison finds the change in `origin/main`. | Confirm behavior, then remove the redundant ref. |
| `PRODUCT_CANDIDATE` | The branch contains product changes not present in `origin/main`. | Review against current main, run its focused gate, then cherry-pick or rebase the minimal coherent slice. |
| `SUPERSEDED` | A later branch contains the intended fix or a hardened replacement. | Do not merge; retain only until the successor is integrated and evidence is located. |
| `RESEARCH_FROZEN` | The branch preserves an experiment, oracle, protocol, or negative result. | Do not merge wholesale. Extract a named product commit only after an explicit product decision. |
| `WIP_BLOCKED` | The branch explicitly preserves an incomplete or unsafe result. | Keep frozen; do not use as product authority. |
| `LOCAL_DIVERGED` | A local control branch differs from `origin/main`. | Preserve unrelated changes; do not use it as integration authority. |

`VALID` and `PRUNABLE` below describe only Git worktree registration. A
`PRUNABLE` registration points to a missing worktree directory; it does not
mean that the branch is obsolete.

## Integrated and equivalent work

| Branch | Tip | Status | Notes |
|---|---:|---|---|
| `codex/cas-recovery-20260901` | `7f75a39` | `INTEGRATED` | Bounded CAS storage recovery. |
| `codex/kotlin-marketing-evidence-20260901` | `f0b874f` | `INTEGRATED` | Kotlin evidence study and site material. |
| `codex/preliminary-evidence-site-20260831` | `1035a2a` | `INTEGRATED` | Qualified preliminary evidence site. |
| `codex/roadmap-c1` | `0618f5b` | `INTEGRATED` | Exact identity evidence ranking. |
| `research/identity-docs-pilot` | `8553be4` | `INTEGRATED` | Common semantic fact envelope. |
| `research/semantic-support-pilot` | `8553be4` | `INTEGRATED` | Same integrated tip as the identity docs pilot. |
| `feat/agent-skill-install` | `976f604` | `PATCH_EQUIVALENT` | Portable agent skill installer; no unique patch remains. |

The local `main` is `LOCAL_DIVERGED`: at this snapshot it was one commit ahead
of and 59 commits behind `origin/main`. It is not an integration authority and
may also contain unrelated working-copy changes.

## Integrated 2026-09-01 batch

`codex/branch-map-20260901` integrated the following work into `main`:

- `9611a1c`: this branch inventory and the safe-cleanup record;
- `af857a9`, `c19c18f`, `f85e13e`: explicit navigation decision
  authority, fail-closed refinement, and the matching embedded skill digest;
- `e8045aa`, `df505c7`: sealed TypeScript/Vitest test-scope location and
  source-candidate evidence binding.

The navigation slice passed 57 library navigation tests, two CLI navigation
tests, and the embedded skill digest test. The TypeScript slice passed 18
`source_locate` unit tests and two managed CLI tests. These slices are
`INTEGRATED`; this statement does not make their source branches product
authorities.

Two apparent candidates required no product code transfer:

- The behavior of `codex/kotlin-relation-coordinate-domain-20260830` is already
  present in `origin/main` in a newer form, including UTF-16 to UTF-8
  normalization, fail-closed invalid-coordinate handling, and focused Unicode
  and surrogate tests.
- The three Gradle behaviors from `research/q1-gradle-model-integration` are
  already present in `origin/main`: selected classpath build dependencies are
  materialized, the target compile task is excluded, and the regression fixture
  runs offline. Replaying `69ad2c0` while preserving current hardening produced
  an empty diff. The focused multi-module Gradle test passed on this integration
  branch.

## Product integration queue

These branches contain the clearest unintegrated product value. The commit
counts are relative to the snapshot authority and overlap between branches.

| Priority | Branch | Tip | Unique patches | Behind | Worktree | Intended product value | Integration rule |
|---:|---|---:|---:|---:|---|---|---|
| 1 | `codex/q1-roadmap` | `cc1341b` | 31 | 16 | `PRUNABLE` | Bounded navigation, exact selection, source windows, reference following, and evidence coverage. | Treat as the main navigation stack. Rebase and gate as milestones; do not reconstruct it from descendant research branches. |
| 2 | `research/launchpad-literal-locate` | `4cef051` | 30 | 16 | `PRUNABLE` | Snapshot-bound direct Git literal navigation. | Resolve the Q1 stack first, then review only the literal-navigation delta (`2ab5347`, `4cef051`). |
| 3 | `research/rank2-k2-exact-target` | `1389dc7` | 16 | 16 | `PRUNABLE` | Exact FIR targets, stable owners, argument mapping, nested JVM descriptors, and worker receipts. | Extract independently justified Kotlin product commits. Never merge the whole research history as one unit. |

The ordering expresses dependency and review safety, not a promise to merge
every candidate. A candidate is removed from this queue only after one of
these outcomes is recorded: integrated, patch-equivalent, superseded, rejected
with rationale, or retained as research-only.

## Superseded implementation branches

| Branch | Tip | Status | Successor or reason |
|---|---:|---|---|
| `codex/kotlin-relation-coordinate-domain-20260830` | `7b8b6cd` | `SUPERSEDED` | The coordinate normalization and adversarial tests already exist in newer `origin/main` implementations. |
| `research/q1-gradle-model-integration` | `457d9b7` | `SUPERSEDED` | Its Gradle product behavior already exists in `origin/main`; its remaining navigation parent belongs to the separate Q1 roadmap. |
| `research/accidental-gradle-fix-preserve` | `f76deb3` | `SUPERSEDED` | Early producer-materialization patch; current `origin/main` contains the hardened behavior. |
| `research/gradle-model-project-deps` | `0e0914a` | `SUPERSEDED` | Intermediate Gradle variant; current `origin/main` contains the hardened behavior. |
| `research/rank2-k2-integration` | `0f1aeeb` | `SUPERSEDED` | Intermediate K2/Gradle integration tip; later exact-target work contains the useful product slices. |
| `research/rank2-k2-release-preflight` | `0f1aeeb` | `SUPERSEDED` | Same intermediate tip as `research/rank2-k2-integration`. |

## Frozen research inventory

The following refs preserve useful experiments or evidence, but none is a
product merge unit.

### Experiment harness and isolation

- `codex/three-arm-triplet-harness-20260830` (`5f4b631`): closed three-arm
  experiment harness.
- `codex/triplet-boundary-hardening-20260830` (`07eb42d`): lease and boundary
  hardening for that harness.
- `research/agent-isolation-boundary` (`cb8750f`): pinned JDK isolation image
  and fail-closed experiment boundary.

### Kotlin K2 exploration

- `research/k2-frontier-stage-b` (`0823c6e`): shallow K2 influence projection
  falsification.
- `research/k2-owner-carrier` (`4a40fc6`): FIR owner/carrier and packaged-worker
  experiment.
- `research/k2-stage-c` (`20fd3ba`): bounded Stage C reuse.
- `research/k2-stage-c-real` (`f020ba0`): large real-project Stage C history
  and retrospective.
- `research/k2-stage-c-real-batch` (`0afee69`): real-project batch replay.

### Kotlin Multiplatform oracle work

- `research/kmp-task-pool` (`ac9b140`)
- `research/kmp-provenance-oracle` (`afe136c`)
- `research/kmp-std-oracle` (`fdc6051`)
- `research/kmp-assert-oracle` (`3e972c5`)
- `research/kmp-lexer-mutation-evidence` (`873e1a4`)
- `research/kmp-lexer-external-admission` (`08350b5`)

### Launchpad task and evidence experiments

- `research/launchpad-baseline-diagnosis` (`5251827`)
- `research/launchpad-fallback-pool` (`20b43ea`)
- `research/launchpad-repaired-pool` (`eb56579`)
- `research/launchpad-rank1-spec-oracle` (`1cad671`)
- `research/launchpad-source-packet-v2` (`43a09e0`)
- `research/launchpad-prospective-oracle-v2` (`175dc4e`)
- `research/launchpad-prospective-native-v3` (`4af45f7`)
- `research/launchpad-local-pair-receipt-analysis` (`5c9c1b7`)
- `research/lp-trace-snapshot-prospective-2-receipt` (`49c2578`)

### Rank-2 protocol and admission experiments

- `research/rank2-action-ledger-design` (`6024f67`)
- `research/rank2-contour-admission` (`8e9d8dc`)
- `research/rank2-native-admission-v2` (`3eba74f`)
- `research/rank2-packet-protocol` (`47d43b6`)
- `research/rank2-read-isolation-gate` (`346877c`)
- `research/rank2-kotlin-call-surface` (`3337759`) — `WIP_BLOCKED`:
  explicitly preserves an incomplete Kotlin call surface.

### Stable-shell and successor-boundary experiments

- `research/stable-shell-profile` (`ef10d66`)
- `research/stable-shell-image-capture` (`135f178`)
- `research/stable-shell-image-capture-clean` (`1c4e913`)
- `research/successor-lima-boundary` (`7271cff`)

## Worktree handling

Worktree state is operational metadata, not integration evidence.

1. Never delete a branch because its worktree registration is `PRUNABLE`.
2. Run `git worktree prune --dry-run` to distinguish missing directories from
   live worktrees before removing stale registrations.
3. For a valid worktree, inspect tracked, staged, and untracked changes before
   removal. A clean worktree still may contain the only convenient checkout of
   an unmerged branch.
4. Treat Codeclew-managed session and candidate worktrees through Codeclew's
   session lifecycle. Do not classify them as hand-maintained feature worktrees.
5. Preserve benchmark evidence separately from reproducible build outputs.
6. Do not record absolute personal paths in this document, Git commits,
   evidence, or command output intended for publication.

## Updating this map

Update this file at every integration or intentional freeze. Use
`origin/main`, not the local `main`, as the comparison authority.

```sh
git fetch origin --prune
git merge-base --is-ancestor BRANCH origin/main
git cherry origin/main BRANCH
git log --cherry-pick --right-only --no-merges origin/main...BRANCH
git worktree list --porcelain
```

For each changed branch, record:

- current tip and base authority;
- integrated, equivalent, superseded, rejected, or frozen status;
- product value and the smallest coherent integration slice;
- required focused tests or evidence;
- successor/dependency branch;
- worktree status without an absolute personal path.

This map is an integration aid, not proof that a branch is safe. Current code,
tests, schemas, and runtime manifests remain authoritative at integration time.
