# Design: lifecycle tree step categories (3-lite)

Date: 2026-09-24
Status: Approved (design)
Scope: Codeclew docs — pseudocode tree of lifecycle sub-processes

## Problem

The pseudocode tree rendered for lifecycle operations (`process_flow::tree` →
"Detailed process flow" cards) lists every non-framework call as a bare
line (e.g. `TaskInstanceDao#setTaskStatus`, `DateTimeHolder#getCurrentTime`).
It does not make clear *what happens* at each step: a reader cannot tell a
state-changing action from a read, and decision points blend into the list.
Full fidelity (fields + value sources) is deferred to a later "full 3" slice
that requires TRANSFORMED_SOURCE retention / source-deepening, which is not
active in the current snapshot. This slice ("3-lite") adds step categories from
retained flow alone.

## Goal

Annotate each step of the lifecycle pseudocode tree with a category prefix
`[W]` (write), `[R]` (read), or `[D]` (decision), derived from the retained flow
event (event kind and/or call target method name). No new data sources, no
frontend changes.

## Non-goals

- No fields or value sources (e.g. `setExecutionDate(now)`). That is the
  "full 3" slice, gated on source-deepening.
- No changes to activity/SVG diagrams.
- No hiding or collapsing of "service" steps (e.g. `getCurrentTime`) — every
  call is kept and annotated.

## Scope

Only the pseudocode tree of lifecycle operations:
`process_flow::tree(events, symbol)`.

## Output format

```text
▸ [D] if (!ignoreFinished || !FINISHED.equals(task.getTaskStatus()))
  [W] TaskInstanceDao#setTaskStatus
  ▸ [D] if (priority != null)
    [W] TaskInstanceDao#setTaskPriority
  [W] TaskInstanceDao#setExecutionDate
  [R] TaskInstanceDao#getTaskId
  [W] TaskErrorRepository#closeErrorsForTask
return
```

Rules:
- `IF` / `LOOP` rows get `[D]` (decided by event kind, not method name).
- `CALL` / `CONSTRUCT` rows get `[W]` or `[R]` by the target method name.
- `RETURN`, `THROW`, `BOUNDARY` rows get no prefix.
- Unknown/ambiguous method names get no prefix (no fallback noise).

## Classification rules (approved)

Classify by the method name in the call target (after `#`):

- **write** `[W]`: `set*`, `update*`, `save*`, `add*`, `remove*`, `close*`,
  `delete*`, and construction/accumulation: `builder`, `build`, `#<init>`.
- **read** `[R]`: `get*`, `find*`, `is*`, `has*`, `contains*`, `load*`.
- **decision** `[D]`: `IF` / `LOOP` event kinds.

## Implementation

In `crates/clew/src/documentation/process_flow.rs`:

1. Add a helper
   `fn step_kind(kind: &str, target: &str) -> Option<&'static str>`
   returning `Some("W")`, `Some("R")`, `Some("D")`, or `None`:
   - `kind == "IF" || kind == "LOOP"` → `Some("D")`
   - otherwise inspect the method name after the final `#` in `target`:
     `write`/`read` prefixes above → `Some("W")`/`Some("R")`
   - else `None`.
2. In `tree()`:
   - `IF` / `LOOP`: prefix the line with `[D] `.
   - `CALL` / `CONSTRUCT` (after the existing framework filter): prefix with
     `[W] ` / `[R] ` when `step_kind` returns a category.
   - `RETURN` / `THROW` / `BOUNDARY`: unchanged (no prefix).

No changes to `render.rs`, `app.js`, or `style.css` — the tree remains plain
text inside `<pre class="tree">`.

## Tests

Add to `process_flow.rs` test module:

- a `write`-named method renders `[W]`;
- a `read`-named method renders `[R]`;
- an `IF` row renders `[D]`;
- `RETURN` / `THROW` render without a prefix;
- a method without a known verb (e.g. `closeErrorsForTask` — covered by `close*`,
  so use a neutral name like `perform`) renders without a prefix;
- update `tree_renders_indented_pseudocode_branches` for the new prefixes.

## Verification

- `cargo fmt --all`
- `cargo test --locked -p clew --lib 'process_flow::' -- --test-threads=1`
- No frontend verification required (no `app.js`/asset changes).

## Follow-up (not in this slice)

"Full 3": source-deepening (retain TRANSFORMED_SOURCE, parse readable step text
with fields and value sources) to show `setExecutionDate(now)` and the
`getCurrentTime → executionDate` binding. To be scoped as a separate slice.
