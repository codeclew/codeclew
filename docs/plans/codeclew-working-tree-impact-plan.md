# Working-tree analysis and change consequences

Status: proposed implementation plan; implementation has not started.
Prepared: 2026-09-06.
Source baseline: `b666129b750b0db3225a090d3570fd528897c983`.

## Product outcome

An agent can answer: "What do my current uncommitted changes affect, why, and
what should I verify?" The result contains a bounded before/after explanation,
a selectable graph, exact source evidence, and explicit gaps in coverage.
The developer can keep editing after capture; the result continues to describe
its immutable input and reports when that input differs from the live checkout.

The first independently useful delivery is narrower: public read-only context
and navigation can inspect current saved working-tree files without a commit.
The complete first release adds change comparison and bounded consequences for
one Kotlin/JVM Gradle repository. Start with one main compilation; include test
compilations only when explicitly selected. Rust is the following dogfood
contour, with syntax authority preserved. Other languages and cross-repository
impact follow qualification of this slice.

## Existing building blocks and gaps

These findings come from maintainer source inspection, not a new runtime test.
All references below describe the pinned baseline above.

| Building block | Existing behavior | Work required |
| --- | --- | --- |
| `repository_snapshot.rs`, `capture_with_hook`, `materialize` | Captures Git index blobs and working-tree entries into CAS; checks repeated file reads and Git views; materializes a private sealed tree. | Scope and budget capture for public analysis; bind base revision; integrate the captured content with public session authority. |
| `session.rs`, `SessionAuthority::open` | Binds HEAD and a local target ref; Python captures selected committed blobs, other languages create a detached source worktree at the base revision. | Introduce explicit source selection and analysis-only authority; all downstream reads must use the selected snapshot. |
| `operations.rs`, `main.rs` | `--committed` allows analysis in a dirty checkout while excluding local edits. | Add working-tree source admission without changing existing defaults or mutation requirements. |
| `thread_impact.rs`, `thread_change_set.rs` | Bounded Kotlin impact and before/after observations for declared provider/consumer pairs and callable families. | Reuse suitable fact/comparison primitives; add single-repository change subjects without fabricating a pair. |
| `explanation_freshness.rs`, `explanation_render.rs` | Retained explanation rendering and claim freshness with unresolved outcomes. | Connect new change evidence; distinguish live-checkout freshness from explanation validity and coverage. |

Source files: [snapshot](https://github.com/codeclew/codeclew/blob/b666129b750b0db3225a090d3570fd528897c983/crates/clew/src/repository_snapshot.rs),
[session](https://github.com/codeclew/codeclew/blob/b666129b750b0db3225a090d3570fd528897c983/crates/clew/src/session.rs),
[impact](https://github.com/codeclew/codeclew/blob/b666129b750b0db3225a090d3570fd528897c983/crates/clew/src/thread_impact.rs),
[comparison](https://github.com/codeclew/codeclew/blob/b666129b750b0db3225a090d3570fd528897c983/crates/clew/src/thread_change_set.rs),
[explanation freshness](https://github.com/codeclew/codeclew/blob/b666129b750b0db3225a090d3570fd528897c983/crates/clew/src/explanation_freshness.rs).

## Proposed user flow

```mermaid
flowchart LR
    A[Saved working-tree files] --> B[Capture immutable snapshot]
    H[Pinned HEAD] --> C[Compare before and after]
    B --> C
    C --> D[Changed declarations and direct consequences]
    D --> E[Explanation and selectable evidence graph]
    D --> U[Unresolved scope and verification suggestions]
    B --> F[Compare with current checkout on request]
```

This diagram is the proposed product architecture, not an observed execution
trace. The graph returned by the product will distinguish compiler relations,
syntax observations and agent interpretations on individual edges.

Proposed CLI shape, not currently implemented:

```sh
clew nav query --repo <repo> --target-ref <branch> \
  --language kotlin --profile <admitted-profile> --compilation <main> \
  --source working-tree --term <identifier>

clew change inspect --repo <repo> --target-ref <branch> \
  --language kotlin --profile <admitted-profile> --compilation <main> \
  --source working-tree --base HEAD
```

`--source working-tree` also belongs on `context open`. Keep `--committed` as
the existing committed-source spelling and reject conflicting selections.
`change inspect` is a thin read-only facade over retained snapshot/comparison
services. The agent writes the narrative from its result; Codeclew does not
need an embedded LLM or a new edit/publication workflow.

## Source contract

- The comparison base is HEAD, resolved once to an exact commit. Arbitrary
  comparison refs and index-only analysis are later additions.
- Working-tree mode uses the current saved file content: staged and unstaged
  changes are not applied as two patches. Preserve the captured index as
  provenance, including when its bytes differ from the working-tree file.
- Include tracked inputs and non-ignored untracked source/build inputs inside
  the admitted scope. Declare the included roots and excluded input categories
  in the result. Changes outside that scope remain visible as out of scope;
  they must not disappear into an apparent whole-repository verdict.
- Deleted files remain explicit entries. Treat uncertain rename correspondence
  as deletion/addition or unresolved matching, with its basis recorded.
- Unsaved editor buffers are outside this filesystem-based source mode.
- Apply limits to file count, individual bytes, total bytes and traversal.
  Reject unsupported conflicts, path/link cases and input-budget overflow
  with a typed result; do not silently substitute committed HEAD.
- Check HEAD, index, path inventory and captured content for observed drift.
  Abort an unstable capture with a retryable diagnostic. Existing repeated
  reads are useful but do not establish an atomic filesystem transaction;
  specify the bounded consistency guarantee and test edits after an earlier
  file has been read. Never advertise stronger atomicity than implemented.
- Bind source mode, base commit, content snapshot, scope, runtime/profile and
  analysis-only operation into session/evidence authority. Version affected
  schemas and preserve verification of old committed-session digests.
- Original source files, HEAD, Git refs and index bytes remain unchanged.
  No stash/reset or user-repository commit is part of capture. Existing
  private materialization can retain its internal synthetic Git metadata.
- A working-tree analysis session cannot enter mutation prepare or publish,
  including through lower-level public commands.

## Delivery sequence

| Delivery | Implementation slice | Accepted user result |
| --- | --- | --- |
| 1. Analyze current edits | Source contract, scoped capture, session/admission binding, Kotlin source materialization, `nav query` and `context open` source selection, lifecycle/GC integration. | A dirty Kotlin fixture returns the edited declaration and a new untracked declaration from the sealed snapshot. Further edits do not alter that retained answer. |
| 2. Explain what changed | Retain base and working-tree analyses; file/hunk changes mapped to before/after declarations; explicit correspondence and build-model comparability. | `change inspect` lists additions, deletions, body changes and supported declaration-shape changes with exact before/after evidence. |
| 3. Find bounded consequences | Single-repository direct relation queries rooted in changed declarations; before/after relation union; selected tests and entrypoint evidence. | The report identifies direct consumers or affected steps, gives the reason for each finding and names what remains unproven. |
| 4. Render an explanation | Agent skill, machine-readable claims, before/after flow, source inspector, retained render and freshness integration. | One readable report explains the change, shows affected and unaffected steps, and opens the code supporting each claim. |
| 5. Qualify daily use | End-to-end fixtures, real Kotlin dogfood, Rust syntax dogfood, CI and published example. | A repeatable daily workflow with measured latency, coverage and cleanup behavior; release claims match tested contours. |

Deliver each row as a bounded implementation slice. Delivery 1 is the next
implementation task and must finish before adding the consequence graph.

### Comparison and consequence rules

- A body change is a change even if the callable signature is unchanged.
  Distinguish textual change from compiler-projected semantic change; comments
  and formatting must not automatically become behavioral-change claims.
- Match declarations using supported stable identity and explicit
  correspondence. Name similarity alone cannot prove rename continuity.
- Analyze both sides with compatible scope/profile/runtime authority. If build
  files or dependencies change, rebuild the relevant model and record whether
  the two results remain comparable. Never reuse an index for different inputs.
- A failure to analyze the after snapshot still permits an exact textual diff
  and usable before evidence, marked with the failed/unavailable after scope.
  It cannot produce "no impact" from an empty after graph.
- Begin with direct relations in the admitted compilation. Include edges from
  both snapshots so a removed call is still explainable. Bounded traversal can
  be added later; limits and omitted scope must stay visible.
- Resolved direct callers can be reported as affected candidates, not as proven
  runtime failures. Preserve overload, dispatch, reflection and framework
  boundaries. Known entrypoints are affected only through supported paths.
- Test references are evidence of a relationship, not proof of sufficient
  coverage. When test compilation was not analyzed, say so. A suggested test
  is separate from an executed test result; inspect does not run tests.
- Findings distinguish exact source delta, compiler relation, agent inference
  and unresolved obligation. An unchanged projected shape is not a universal
  proof of unchanged behavior.

### Explanation, retention and freshness

Every claim and graph edge retains before/after anchors as applicable, snapshot
identities, evidence references and authority. New uncommitted text opens from
retained local evidence; do not fabricate a GitHub commit link for those bytes.
Existing source-bound rendering should be reused where its contracts fit.

The initial public UI is a local report with an overview, selected-step text,
before/after code and verification suggestions. Publication is a separate user
action. A repeat render reads retained evidence without rebuilding the project.

Keep three questions separate: whether retained evidence is valid; whether it
still describes the live checkout; and whether the selected scope is complete.
Continued editing makes the live view out of date, not the old evidence corrupt.
Refreshing creates new immutable inputs and a new result. There is no background
watcher in the first release. Retain the new snapshot roots in normal reachability
and GC rules, and remove derived materialization when its session is collected.

## Acceptance cases

1. One file has staged version A and unstaged version B: working-tree analysis
   shows B, preserves index A, and compares against pinned HEAD.
2. An added source file, a deletion and a rename all appear; ignored generated
   files are excluded by policy and unmatched changes remain explicit.
3. An edit during capture produces a typed unstable-input outcome when detected;
   an edit after capture cannot change retained code or comparison evidence.
4. Changing a function body while preserving its signature is reported. A
   comment-only edit preserves the supported behavior claims or records the
   limits of proving that preservation.
5. Removing a call preserves its before edge and invalidates the corresponding
   explanation claim. An unchanged neighboring step retains its evidence.
6. Changing a signature identifies a supported direct consumer; ambiguity,
   unresolved dispatch and truncated coverage never become a green verdict.
7. Broken Kotlin or changed build inputs produce an explicit incomplete result
   while preserving available before/after source evidence.
8. Test scope omitted, test scope analyzed and tests actually executed remain
   distinct states. The report never invents a test relationship or run.
9. Original file bytes/modes, index bytes and refs are unchanged after success,
   failure and cleanup. Working-tree sessions reject prepare/publish through
   every supported command surface; committed-source behavior remains intact.
10. Repeat rendering starts no compiler/build process; repeat identical capture
    preserves content identity. Cleanup respects retained evidence roots.

Use focused tests for changed snapshot/admission/comparison behavior, then the
repository's normal CI and a small Linux/macOS end-to-end qualification. Do not
repeat broad independent verification without a material change or the final
release gate.

Record time to first useful answer, cold capture/model/analysis time, unchanged
repeat time, bytes retained, changed declarations found, missed direct
consequences and unsupported claims. Set numeric performance gates from the
first representative measurements, not guessed budgets for an unknown project.

## First implementation task

Implement Delivery 1 for one Kotlin/JVM Gradle main compilation, using the
existing snapshot store and private workspace machinery. Include a fixture
with staged-plus-unstaged content, an untracked source file and a subsequent
edit after capture. Add the source-mode contract to CLI help and the agent
skill. The acceptance artifact is one CLI transcript proving that navigation
returns the captured edited bytes while the user's checkout and index retain
their original state. No impact facade or new graph engine is needed for this
first result.
