# Lifecycle Tree Step Categories (3-lite) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Annotate each step of the lifecycle pseudocode tree with a category prefix `[W]`/`[R]`/`[D]`, derived from retained flow, so readers can tell a state-changing action from a read and see decision points.

**Architecture:** Add a single pure helper `step_kind(kind, target)` in `crates/clew/src/documentation/process_flow.rs` that returns a category by event kind (`IF`/`LOOP` → `D`) or target method-name prefix (`set*`/`get*`/... → `W`/`R`). Wire it into the existing `tree()` loop to prefix lines. No data-source, renderer, or frontend changes.

**Tech Stack:** Rust (`process_flow.rs`), serde_json `Value`, existing `cargo test` harness.

**Spec:** `docs/superpowers/specs/2026-09-24-lifecycle-tree-step-categories-design.md`

---

### Task 1: Add failing tests for step categories

**Files:**
- Test: `crates/clew/src/documentation/process_flow.rs` (test module `mod tests`)

- [ ] **Step 1: Read the current tree() and tests**

Read `crates/clew/src/documentation/process_flow.rs`. Locate `pub fn tree(...)` and the test `tree_renders_indented_pseudocode_branches`. The current `tree()` renders `if`/`loop` lines without a prefix and call lines as a bare method label.

- [ ] **Step 2: Add new tests**

Append the following tests inside `mod tests` (after `tree_renders_indented_pseudocode_branches`):

```rust
#[test]
fn tree_prefixes_write_calls_with_w() {
    let events = json!([
        {"kind":"CALL","target":"method:class:ru.tins.svc.Repo#save()V"},
        {"kind":"CALL","target":"method:class:ru.tins.svc.TaskDao#setTaskStatus()V"},
        {"kind":"RETURN"}
    ]);
    let tree = super::tree(
        &events,
        "method:class:ru.tins.svc.TaskService#changeStatus()V",
    )
    .unwrap();
    assert!(tree.contains("[W] Repo#save"), "{tree}");
    assert!(tree.contains("[W] TaskDao#setTaskStatus"), "{tree}");
}

#[test]
fn tree_prefixes_read_calls_with_r() {
    let events = json!([
        {"kind":"CALL","target":"method:class:ru.tins.svc.TaskDao#getTaskStatus()V"},
        {"kind":"CALL","target":"method:class:ru.tins.svc.TaskService#findTask()V"},
        {"kind":"RETURN"}
    ]);
    let tree = super::tree(
        &events,
        "method:class:ru.tins.svc.TaskService#changeStatus()V",
    )
    .unwrap();
    assert!(tree.contains("[R] TaskDao#getTaskStatus"), "{tree}");
    assert!(tree.contains("[R] TaskService#findTask"), "{tree}");
}

#[test]
fn tree_prefixes_decisions_with_d() {
    let events = json!([
        {"kind":"IF","condition":"task != null"},
        {"kind":"CALL","target":"method:class:ru.tins.svc.Repo#save()V"},
        {"kind":"END"},
        {"kind":"LOOP","condition":"for-each"},
        {"kind":"END"}
    ]);
    let tree = super::tree(
        &events,
        "method:class:ru.tins.svc.TaskService#changeStatus()V",
    )
    .unwrap();
    assert!(tree.contains("[D] if (task != null) then"), "{tree}");
    assert!(tree.contains("[D] loop (for-each)"), "{tree}");
}

#[test]
fn tree_leaves_control_and_unclassifiable_steps_without_prefix() {
    let events = json!([
        {"kind":"CALL","target":"method:class:ru.tins.svc.Handler#perform()V"},
        {"kind":"THROW"},
        {"kind":"RETURN"}
    ]);
    let tree = super::tree(
        &events,
        "method:class:ru.tins.svc.TaskService#changeStatus()V",
    )
    .unwrap();
    // neutral verb `perform` -> no prefix; control flow -> no prefix
    assert!(tree.contains("\nHandler#perform\n"), "{tree}");
    assert!(tree.contains("\nthrow\n"), "{tree}");
    assert!(tree.contains("\nreturn\n"), "{tree}");
}
```

- [ ] **Step 3: Run the new tests to verify they fail**

Run: `cargo test --locked -p clew --lib 'process_flow::' -- --test-threads=1`
Expected: FAIL — assertions on `[W] Repo#save`, `[R] ...`, `[D] if ...` not satisfied because `tree()` emits no prefixes yet.

---

### Task 2: Implement step_kind and wire into tree()

**Files:**
- Modify: `crates/clew/src/documentation/process_flow.rs`

- [ ] **Step 1: Add the step_kind helper**

Place `step_kind` after `condition(...)` (near the other private helpers). Add:

```rust
/// Classify a flow step into a readability category for the pseudocode tree:
/// `W` (write / state change), `R` (read), `D` (decision). Returns `None` for
/// control-flow or unclassifiable steps, which are rendered without a prefix.
fn step_kind(kind: &str, target: &str) -> Option<&'static str> {
    if matches!(kind, "IF" | "LOOP") {
        return Some("D");
    }
    let method = target.rsplit('#').next().unwrap_or(target);
    let method = method.split('(').next().unwrap_or(method);
    const WRITE: &[&str] = &[
        "set", "update", "save", "add", "remove", "close", "delete", "builder", "build",
        "<init>",
    ];
    const READ: &[&str] = &["get", "find", "is", "has", "contains", "load"];
    if WRITE.iter().any(|p| method.starts_with(p)) {
        Some("W")
    } else if READ.iter().any(|p| method.starts_with(p)) {
        Some("R")
    } else {
        None
    }
}
```

- [ ] **Step 2: Wire prefixes into tree()**

In `tree()`, make these edits:

`"IF"` arm — add the `[D] ` prefix:
```rust
"IF" => {
    out.push_str(&format!(
        "{}[D] if ({}) then\n",
        indent(depth),
        condition(&row)
    ));
    depth += 1;
}
```

`"LOOP"` arm — add the `[D] ` prefix:
```rust
"LOOP" => {
    out.push_str(&format!(
        "{}[D] loop ({})\n",
        indent(depth),
        condition(&row)
    ));
    depth += 1;
}
```

`"CALL" | "CONSTRUCT"` arm — add the `[W] `/`[R] ` prefix when classified:
```rust
"CALL" | "CONSTRUCT" => {
    let target = row["target"].as_str().unwrap_or("");
    if !target.is_empty() && !is_framework_symbol(target) {
        let prefix = match step_kind(kind, target) {
            Some(c) => format!("[{c}] "),
            None => String::new(),
        };
        out.push_str(&format!(
            "{}{}{}\n",
            indent(depth),
            prefix,
            method_label(target)
        ));
    }
}
```

`"RETURN"`, `"THROW"`, `"BOUNDARY"` arms: leave unchanged (no prefix).

- [ ] **Step 3: Run the new tests to verify they pass**

Run: `cargo test --locked -p clew --lib 'process_flow::' -- --test-threads=1`
Expected: PASS for the four new tests.

---

### Task 3: Update existing test, format, full verification, commit

**Files:**
- Modify: `crates/clew/src/documentation/process_flow.rs`

- [ ] **Step 1: Update tree_renders_indented_pseudocode_branches for new prefixes**

Its events are: `IF(anyTask.isPresent())` → `CALL Handler#handle` → `ELSE` → `CALL Other#do` → `END` → `CALL Repo#save` → `RETURN`. The new output prefixes `if` with `[D] ` and `Repo#save` with `[W] `. Update the assertions:

Replace:
```rust
assert!(
    tree.contains("if (anyTask.isPresent()) then\n  Handler#handle"),
    "{tree}"
);
assert!(tree.contains("else\n  Other#do"), "{tree}");
assert!(tree.contains("Repo#save\nreturn"), "{tree}");
```
with:
```rust
assert!(
    tree.contains("[D] if (anyTask.isPresent()) then\n  Handler#handle"),
    "{tree}"
);
assert!(tree.contains("else\n  Other#do"), "{tree}");
assert!(tree.contains("[W] Repo#save\nreturn"), "{tree}");
```

- [ ] **Step 2: Format and run the full process_flow test module**

Run:
```bash
cargo fmt --all
cargo test --locked -p clew --lib 'process_flow::' -- --test-threads=1
```
Expected: all `process_flow::` tests PASS, `cargo fmt` reports no diffs.

- [ ] **Step 3: Commit**

```bash
git add crates/clew/src/documentation/process_flow.rs
git commit -m "feat(docs): categorize lifecycle tree steps as write/read/decision"
```

---

## Verification (final)

- [ ] `cargo fmt --all` — no diffs
- [ ] `cargo test --locked -p clew --lib 'process_flow::' -- --test-threads=1` — all pass
- [ ] No `app.js`/`style.css`/`render.rs` changes (frontend untouched)

## Follow-up (out of scope)

"Full 3": source-deepening to add fields and value sources (e.g. `setExecutionDate(now)`). Not part of this plan.