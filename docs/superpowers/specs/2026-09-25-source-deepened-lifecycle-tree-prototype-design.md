# Design: Source-deepened lifecycle tree — prototype (S1+S2)

Date: 2026-09-25
Status: Approved (design)
Scope: Codeclew docs — bind lifecycle pseudocode tree to cached `TRANSFORMED_SOURCE` so step text carries data-flow (arguments, assignments), keeping the `[W]/[R]/[D]` categories.

## Problem

The lifecycle pseudocode tree (`process_flow::tree`) is built from retained FLOW events, which carry no arguments or assignments. As a result the data-flow binding (`getCurrentTime → changeDate → setUpdateDate`) is invisible. The cached source (`TRANSFORMED_SOURCE`) does contain it, and `source_steps.rs` can already parse a method body into readable steps with arguments — but:

- `method_source()` in `render.rs` looks up `TRANSFORMED_SOURCE` observations **by `symbol`**, which those observations do not have; the real link is `SYMBOL observation.source_ids → Source.text` in `checked.sources()`.
- `source_steps.rs` renders only PlantUML activity, not a pseudocode tree with `[W]/[R]/[D]`.

This slice (S1+S2) fixes the lookup and adds a tree renderer. S3 (wiring into the lifecycle render + re-render) is a follow-up.

## Goal

1. **S1 — fix `method_source`**: resolve a method's retained source via `SYMBOL observation.source_ids → checked.sources()[id].text`, falling back to the legacy `TRANSFORMED_SOURCE`-by-symbol path (kept for the existing test).
2. **S2 — add `source_steps::tree`**: render a method body as a pseudocode tree with `[W]/[R]/[D]` categories and readable arguments/assignments.

## Non-goals

- No wiring into the lifecycle renderer (S3) in this slice.
- No changes to `app.js`, `style.css`, or activity/SVG output.
- No re-capture of snapshots; works on already-retained evidence.

## S1: `method_source` (render.rs)

Replace the body of `fn method_source(checked: &Check, symbol: &str) -> Option<String>`:

1. Build `let sources = checked.sources();`.
2. Find the `SYMBOL` observation with `o.kind == "SYMBOL" && o.symbol == symbol`; for its `source_ids`, return the first `Source.text` present in `sources`.
3. If none, fall back to the existing legacy path (`TRANSFORMED_SOURCE` observation with `o.symbol == symbol`, reading `normalized["documentation"]["source"]`, then `normalized["source"]`, then `normalized` as string).

This preserves `source_deepening_is_preferred_when_source_retained` (render.rs:3133, legacy path) and adds the real pipeline path.

## S2: `source_steps::tree` (source_steps.rs)

Add a shared write/read classifier in `process_flow.rs` and a tree renderer in `source_steps.rs`.

### Shared classifier (`process_flow.rs`)

Extract the WRITE/READ prefix matching from `step_kind` into:

```rust
/// Classify a method name as write (`W`) or read (`R`) by verb prefix.
pub(crate) fn method_write_read(name: &str) -> Option<&'static str> {
    const WRITE: &[&str] = &[
        "set", "update", "save", "add", "remove", "close", "delete", "builder", "build",
        "<init>",
    ];
    const READ: &[&str] = &["get", "find", "is", "has", "contains", "load"];
    if WRITE.iter().any(|p| name.starts_with(p)) {
        Some("W")
    } else if READ.iter().any(|p| name.starts_with(p)) {
        Some("R")
    } else {
        None
    }
}
```

Refactor `step_kind` to call it for the method portion (behavior unchanged; existing `step_kind` tests must still pass).

### Tree renderer (`source_steps.rs`)

Add:

```rust
/// Render a method body as an indented pseudocode tree with `[W]/[R]/[D]`
/// categories and readable arguments/assignments (data-flow).
pub fn tree(source: &str, symbol: &str) -> Option<String>
```

Rules (per cleaned line, reusing `strip_comments`, `method_body`, `condition`, `shorten_statement`):

- First line: `Вход: <signature>` (via existing `signature(head, symbol)`).
- `if (...)`, `else if (...)`, `while (...)`, `for (...)` → line prefixed `[D] ` and increase depth (else/else-if continue at current depth after closing one level).
- `else` → close one level, emit `else`, re-open.
- `catch`/`finally` → close/open a level (no label).
- `switch`/`case`/`default`/`break`/`{`/`;` → skipped.
- Closing `}` (including `} else`, `} catch`, `} while`) → close one level (handle continuations as in `render_body`).
- Real statement (via `shorten_statement`) → prefix by category:
  - `return` / `throw` → no prefix.
  - assignment (contains `=`, not `==`) → `[W] `.
  - call → method name after the last `.` and before `(`; classify via
    `super::process_flow::method_write_read(name)` → `[W] `/`[R] `/`""`.
  - otherwise no prefix.
- Return `None` when the body is empty or unparseable (`method_body` fails).

## Tests

### S1 (`render.rs` tests)
- New: `method_source_resolves_via_symbol_source_ids` — build a `Check` whose `SYMBOL` observation has `source_ids` pointing to a `Source` in `services[].sources`; assert `method_source` returns that `Source.text`.
- Keep `source_deepening_is_preferred_when_source_retained` passing (legacy fallback).

### S2 (`source_steps.rs` tests)
- `tree_renders_categorized_data_flow`: using the existing `CHANGE_TASK_STATUS` fixture, assert the tree contains:
  - `[D] if (anyTask.isPresent()) then`
  - `taskInstance = anyTask.get(...)` with a `[W]` prefix (assignment)
  - a `set` call rendered with `[W]` and its argument kept, e.g. `changeAnyTaskStatus(...)` collapsed but a short-arg call like `setUpdateDate(changeDate)` kept
  - `return` lines with no category prefix
- `tree_none_on_unparseable_source` — `tree("no body here", "method:x")` is `None`.
- `process_flow` step_kind tests remain green after the `method_write_read` refactor.

## Verification

- `cargo fmt --all`
- `cargo test --locked -p clew --lib 'documentation::' -- --test-threads=1`

## Follow-up (not in this slice)

- S3: wire `source_steps::tree` into the lifecycle tree render (render.rs:2494), preferring source when available, falling back to `process_flow::tree`.
- Re-render arch-kasko and confirm `changeTaskStatus` shows `getCurrentTime → changeDate → setUpdateDate` on the page.