# Source Tree Multi-line Aggregation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `source_steps::tree` aggregate logically complete statements (balancing across lines) so multi-line calls, builder chains, and declarations render as whole lines instead of torn fragments.

**Architecture:** Add `split_statements(body) -> Vec<String>` that walks comment-stripped body lines, accumulates a statement until it is complete (block close `}`, trailing `;`, or trailing `{`), joining multi-line expressions with spaces. Rewrite `tree()` to iterate aggregated statements instead of raw lines, keeping the existing `[W]/[R]/[D]` categories and the `Vec<bool>` depth stack.

**Tech Stack:** Rust (`source_steps.rs`), existing helpers (`strip_comments`, `method_body`, `tree_statement`, `statement_kind`, `condition`, `keep_args`).

**Spec:** `docs/superpowers/specs/2026-09-25-source-tree-multiline-aggregation-design.md`

---

### Task 1: Add `split_statements` aggregator

**Files:**
- Modify: `crates/clew/src/documentation/source_steps.rs`
- Test: `crates/clew/src/documentation/source_steps.rs` (test module)

- [ ] **Step 1: Read the current `tree()`**

Read `crates/clew/src/documentation/source_steps.rs`, the `pub fn tree(...)` function. It currently walks `body.split('\n')` line by line. Confirm `strip_comments`, `method_body`, `tree_signature`, `tree_statement`, `statement_kind`, `condition` exist.

- [ ] **Step 2: Write the failing test**

Append to the `source_steps.rs` test module (`mod tests`):

```rust
#[test]
fn split_statements_joins_multiline_call_and_builder_chain() {
    let body = "\
Date changeDate = DateTimeHolder.getCurrentTime();
log.info(
    \"count={}, type={}\",
    taskId,
    task.getType()
);
taskStatusHistoryDao = TaskStatusHistoryDao.builder()
    .taskInstance(taskInstance)
    .changeUser(user)
    .build();
if (priority != null) {
    taskInstance.setTaskPriority(priority);
}
}";
    let stmts = split_statements(body);
    assert_eq!(
        stmts,
        vec![
            "Date changeDate = DateTimeHolder.getCurrentTime();",
            "log.info( \"count={}, type={}\", taskId, task.getType() );",
            "taskStatusHistoryDao = TaskStatusHistoryDao.builder() .taskInstance(taskInstance) .changeUser(user) .build();",
            "if (priority != null) {",
            "taskInstance.setTaskPriority(priority);",
            "}",
        ]
    );
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --locked -p clew --lib 'documentation::source_steps::' -- --test-threads=1`
Expected: FAIL — `split_statements` is not defined.

- [ ] **Step 4: Implement `split_statements`**

Add this function to `source_steps.rs` (place it near `method_body`, before `tree`):

```rust
/// Split a method body (lines inside the braces) into logically complete
/// statements, joining lines that continue an expression (multi-line calls,
/// builder chains) with a single space. A statement ends at a block close
/// (`}`), a trailing `;`, or a trailing `{` (a control-flow header).
fn split_statements(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for raw in body.split('\n') {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(line);
        let trimmed = cur.trim();
        if trimmed.starts_with('}') || trimmed.ends_with(';') || trimmed.ends_with('{') {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --locked -p clew --lib 'documentation::source_steps::' -- --test-threads=1`
Expected: PASS for `split_statements_joins_multiline_call_and_builder_chain`.

- [ ] **Step 6: Commit**

```bash
git add crates/clew/src/documentation/source_steps.rs
git commit -m "feat(docs): aggregate multi-line statements in source tree"
```

---

### Task 2: Rewrite `tree()` to iterate aggregated statements

**Files:**
- Modify: `crates/clew/src/documentation/source_steps.rs`
- Test: `crates/clew/src/documentation/source_steps.rs` (test module)

- [ ] **Step 1: Write the failing tests**

Append to the `source_steps.rs` test module:

```rust
#[test]
fn tree_renders_multiline_call_as_one_statement() {
    let src = "\
void log() {
    log.info(
        \"count={}\",
        taskId,
        task.getType()
    );
    if (task != null) {
        svc.handle(task);
    }
}";
    let tree = tree(src, "method:class:svc.TaskService#log()V").unwrap();
    // The multi-line log.info collapses to a single line, not argument leaks.
    assert!(tree.contains("log.info(...)"), "{tree}");
    assert!(!tree.contains("\"count={}\""), "{tree}");
    assert!(tree.contains("[D] if (task != null) then\n  svc.handle(task)"), "{tree}");
}

#[test]
fn tree_collapses_builder_chain_into_one_assignment() {
    let src = "\
void build() {
    TaskStatusHistoryDao dao = TaskStatusHistoryDao.builder()
        .taskInstance(taskInstance)
        .changeUser(user)
        .build();
    if (dao != null) {
        repo.save(dao);
    }
}";
    let tree = tree(src, "method:class:svc.TaskService#build()V").unwrap();
    // One assignment, no stray `.field(...)` lines.
    assert!(
        tree.contains("[W] dao = TaskStatusHistoryDao.builder(...)"),
        "{tree}"
    );
    assert!(!tree.contains(".taskInstance(taskInstance)"), "{tree}");
    assert!(!tree.contains(".changeUser(user)"), "{tree}");
    assert!(tree.contains("[D] if (dao != null) then\n  repo.save(dao)"), "{tree}");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --locked -p clew --lib 'documentation::source_steps::' -- --test-threads=1`
Expected: FAIL — `log.info(...)`/assignment fragments appear and stray `.taskInstance(...)`/`.changeUser(...)` lines are present.

- [ ] **Step 3: Rewrite `tree()`**

Replace the body of `pub fn tree(...)` (from `let mut out = String::new();` after the header through the closing `}`) with:

```rust
    let statements = split_statements(body);
    let mut out = String::new();
    out.push_str(&format!("Вход: {}\n", tree_signature(head, symbol)));
    // Stack of open blocks. `true` = renderable control flow (if/loop) and
    // contributes to depth; `false` = a skipped block (try/catch/finally/
    // switch/do) whose braces balance without moving depth.
    let mut stack: Vec<bool> = Vec::new();
    let indent = |d: usize| "  ".repeat(d);
    for raw in &statements {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('}') {
            let rest = line.trim_start_matches('}').trim();
            if rest.starts_with("else if (") || rest.starts_with("elseif (") {
                stack.pop();
                let d = stack.iter().filter(|&&r| r).count();
                out.push_str(&format!(
                    "{}[D] else if ({}) then\n",
                    indent(d),
                    condition(line)
                ));
                stack.push(true);
            } else if rest.starts_with("else") {
                stack.pop();
                let d = stack.iter().filter(|&&r| r).count();
                out.push_str(&format!("{}else\n", indent(d)));
                stack.push(true);
            } else if rest.starts_with("catch (") || rest.starts_with("finally") {
                // try → catch / finally transition: both are skipped blocks.
                stack.pop();
                stack.push(false);
            } else if rest.starts_with("while (") {
                // do-while: close the `do`, nothing further rendered.
                stack.pop();
            } else {
                stack.pop();
            }
            continue;
        }
        if line.starts_with("else if (") || line.starts_with("elseif (") {
            stack.pop();
            let d = stack.iter().filter(|&&r| r).count();
            out.push_str(&format!(
                "{}[D] else if ({}) then\n",
                indent(d),
                condition(line)
            ));
            stack.push(true);
            continue;
        }
        if line.starts_with("else") {
            stack.pop();
            let d = stack.iter().filter(|&&r| r).count();
            out.push_str(&format!("{}else\n", indent(d)));
            stack.push(true);
            continue;
        }
        if line.starts_with("if (") {
            let d = stack.iter().filter(|&&r| r).count();
            out.push_str(&format!(
                "{}[D] if ({}) then\n",
                indent(d),
                condition(line)
            ));
            stack.push(true);
            continue;
        }
        if line.starts_with("while (") || line.starts_with("for (") {
            let d = stack.iter().filter(|&&r| r).count();
            out.push_str(&format!(
                "{}[D] loop ({})\n",
                indent(d),
                condition(line)
            ));
            stack.push(true);
            continue;
        }
        if line.starts_with("try") || line.starts_with("switch (") || line.starts_with("do") {
            stack.push(false);
            continue;
        }
        if line.starts_with("catch (")
            || line.starts_with("finally")
            || line.starts_with("case ")
            || line.starts_with("default")
            || line.starts_with("break")
            || line == "{"
        {
            continue;
        }
        let stmt = tree_statement(line);
        if !stmt.is_empty() {
            let d = stack.iter().filter(|&&r| r).count();
            out.push_str(&format!(
                "{}{}{}\n",
                indent(d),
                statement_kind(&stmt),
                stmt
            ));
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
```

Keep the `tree` signature and the `let no_comments = strip_comments(source);` / `method_body` / `head` / `body` setup above it unchanged.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked -p clew --lib 'documentation::source_steps::' -- --test-threads=1`
Expected: PASS — `tree_renders_multiline_call_as_one_statement`, `tree_collapses_builder_chain_into_one_assignment`, plus the existing `tree_renders_categorized_data_flow` and `tree_keeps_depth_through_skipped_try_catch`.

- [ ] **Step 5: Commit**

```bash
git add crates/clew/src/documentation/source_steps.rs
git commit -m "feat(docs): render source tree from aggregated statements"
```

---

### Task 3: Format, full verification, commit

**Files:**
- Modify: `crates/clew/src/documentation/source_steps.rs`

- [ ] **Step 1: Run fmt and the full documentation module**

Run:
```bash
cargo fmt --all
cargo test --locked -p clew --lib 'documentation::' -- --test-threads=1
```
Expected: all `documentation::` tests PASS (incl. the pre-existing `document`/`steps`/`shorten_statement`/`keep_args` tests), `cargo fmt` no diffs.

- [ ] **Step 2: Commit any fmt reflow**

```bash
git add crates/clew/src/documentation/source_steps.rs
git commit -m "style(docs): fmt source_steps after statement aggregation" || true
```

---

## Verification (final)

- [ ] `cargo fmt --all` — no diffs
- [ ] `cargo test --locked -p clew --lib 'documentation::' -- --test-threads=1` — all pass
- [ ] No `app.js`/`style.css`/`render.rs` changes (frontend and activity untouched)

## Follow-up (out of scope)

- Re-render arch-kasko and confirm `openError`/`restartPolicyChangeTask`/`changeTaskStatus` render cleanly on the page.
- Check remaining noisy cases against real snapshot sources.