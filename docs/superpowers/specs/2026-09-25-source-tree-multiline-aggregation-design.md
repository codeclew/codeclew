# Design: Multi-line-aware statement aggregation for the source tree

Date: 2026-09-25
Status: Approved (design)
Scope: Codeclew docs — fix `source_steps::tree` rendering of multi-line constructs.

## Problem

`source_steps::tree` (from the S1+S2/S3 prototype) parses the method body line by
line. On real lifecycle methods this produces noisy output for constructs that
span multiple lines:

- **Multi-line calls** — `log.info(\n  "msg",\n  arg\n)` is split into a bare
  `log.info(` line plus its argument lines rendered as garbage.
- **Builder chains** — `TaskStatusHistoryDao.builder()\n    .taskInstance(t)\n    ...\n    .build();`
  renders `.taskInstance(...)`, `.changeUser(...)`, `.build()` as separate
  stray lines.
- **Ternary expressions** — `... ? "final" : "not final"` splits across lines.
- **Declarations** — `TaskInstanceDao taskInstanceDao;` renders as a bare line.

The data-flow binding works (verified on the page), but readability degrades on
`openError`, `restartPolicyChangeTask`, and the builder chain in
`changeTaskStatus`.

## Goal

Make `tree()` aggregate logically complete statements before rendering, so a
statement that starts on one line and continues on later lines is rendered once,
as a whole, instead of being torn apart. Keep the existing `[W]/[R]/[D]`
categories and the depth stack.

## Non-goals

- No re-capture or new data sources.
- No changes to activity/SVG (`document`, `steps`) output.
- No changes to `app.js`/`style.css`.

## Approach: statement aggregation with brace/paren balance

Replace the per-line body walk in `tree()` with a two-phase pass:

1. **Split the body into logical statements**, balancing `(`, `[`, `{` and
   their closers across lines. A logical statement ends when:
   - it is a control-flow header (`if (...)`, `else if (...)`, `while (...)`,
     `for (...)`, `else`) — these end at the opening `{` (the `{` may be on the
     same line);
   - or a statement ends at a `;`;
   - or a closing `}` that terminates a block.
   Inside an expression, newlines are treated as spaces; a line that begins
   with `.` is treated as a continuation of the current expression (builder
   chain).
2. **Render each aggregated statement** through the existing classification
   (`statement_kind` / `tree_statement`) and control-flow handling, keeping the
   `Vec<bool>` depth stack (true = renderable if/loop, false = skipped block).

### Example effects (expected)

- `log.info("Task errors count: ...", taskId, taskInstanceDao.getTaskType(), errorsCount > maxRetryCount ? "final" : "not final");`
  → one statement, collapsed by `keep_args` to `log.info(...)` when long.
- `taskStatusHistoryDao = TaskStatusHistoryDao.builder().taskInstance(taskInstance).changeUser(user).taskStatus(status).statusDate(changeDate).build();`
  → one assignment statement, rendered as
  `[W] taskStatusHistoryDao = TaskStatusHistoryDao.builder()...` (collapsed).
- `TaskInstanceDao taskInstanceDao;` → rendered once as `TaskInstanceDao taskInstanceDao` (no prefix).
- `errorsCount > maxRetryCount ? "final" : "not final"` no longer escapes as a
  standalone `[W] "..."` line.

## Implementation notes

- Add a helper (in `source_steps.rs`) that walks the raw (comment-stripped)
  body lines and produces `Vec<String>` of aggregated statements, preserving
  the control-flow/`}` markers so the depth logic can be driven from it.
- Reuse `strip_comments`, `method_body`, `tree_statement`, `statement_kind`,
  `condition`, and `super::process_flow::method_write_read`.
- Keep `tree_statement`/`keep_args` behavior; multi-line expressions are joined
  with a space before `tree_statement` sees them (so `keep_args` balance logic
  operates on the whole expression).

## Tests

Add to `source_steps.rs`:

- `tree_renders_multiline_call_as_one_statement` — a `log.info("m", a, b)` split
  across lines renders as a single line (e.g. `log.info(...)`, not argument
  lines leaking).
- `tree_collapses_builder_chain_into_one_assignment` — a builder chain starting
  with `x = Builder.builder()\n .field(v)\n .build();` renders as one `[W] x = ...`
  assignment, with no stray `.field(...)` lines.
- `tree_keeps_depth_through_skipped_try_catch` — existing test must still pass.
- `tree_renders_categorized_data_flow` — existing test must still pass.
- Regression: `tree_returns_none_on_unparseable_source` and
  `keep_args_collapses_multiline_call_without_closing_paren` still pass.

## Verification

- `cargo fmt --all`
- `cargo test --locked -p clew --lib 'documentation::' -- --test-threads=1`

## Follow-up

- Re-render arch-kasko and confirm `openError`/`restartPolicyChangeTask`/
  `changeTaskStatus` render cleanly on the page.
- Re-run the source tree renderer against real snapshot sources to check no
  remaining noisy cases.