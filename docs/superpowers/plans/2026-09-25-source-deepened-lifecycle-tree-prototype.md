# Source-Deepened Lifecycle Tree Prototype (S1+S2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bind a lifecycle method's pseudocode tree to cached `TRANSFORMED_SOURCE` so step text carries data-flow (arguments, assignments) while keeping `[W]/[R]/[D]` categories.

**Architecture:** Fix `method_source()` in `render.rs` to resolve a method's retained source via `SYMBOL observation.source_ids → checked.sources()[id].text` (with a legacy fallback). Extract a shared write/read classifier `method_write_read` in `process_flow.rs`, and add `source_steps::tree()` that renders a method body as an indented tree with categories and readable arguments.

**Tech Stack:** Rust (`render.rs`, `process_flow.rs`, `source_steps.rs`), serde_json `Value`, existing `cargo test` harness.

**Spec:** `docs/superpowers/specs/2026-09-25-source-deepened-lifecycle-tree-prototype-design.md`

---

### Task 1: Fix `method_source` to resolve via SYMBOL source_ids

**Files:**
- Modify: `crates/clew/src/documentation/render.rs` (`fn method_source`, ~line 1474)
- Test: `crates/clew/src/documentation/render.rs` (test module)

- [ ] **Step 1: Read the current `method_source` and the `Check`/`Observation`/`Source` shapes**

Read `crates/clew/src/documentation/render.rs` around lines 1456-1496 (`auto_flow_puml`, `method_source`). Confirm: `Check` has `services: BTreeMap<String, ServiceEvidence>` and `pub fn sources(&self) -> BTreeMap<String, Source>`; `Observation` has `source_ids: Vec<String>`; `ServiceEvidence` has `pub observations: BTreeMap<String, Observation>` and `pub sources: BTreeMap<String, Source>` (see `model.rs`). The existing test `source_deepening_is_preferred_when_source_retained` (render.rs:3133) builds a `TRANSFORMED_SOURCE` observation with `normalized: {"documentation":{"source":...}}` — that legacy path must keep working.

- [ ] **Step 2: Write the failing test**

Append to the `render.rs` test module (near the other `auto_flow_puml`/`method_source` tests):

```rust
#[test]
fn method_source_resolves_via_symbol_source_ids() {
    let symbol = "method:class:svc.TaskService#handle";
    let source_text = "public void handle(Long id) {\n  svc.doSomething(id);\n}";
    let sym_obs = Observation {
        id: "svc:symbol:handle".into(),
        kind: "SYMBOL".into(),
        service: "svc".into(),
        symbol: symbol.into(),
        normalized: json!({}),
        digest: "d".into(),
        source_ids: vec!["svc:source:handle".into()],
    };
    let source = Source {
        id: "svc:source:handle".into(),
        service: "svc".into(),
        revision: "rev".into(),
        file: "TaskService.java".into(),
        start_line: 1,
        end_line: 3,
        text: source_text.into(),
        text_digest: "d".into(),
        evidence_digest: "d".into(),
        authority: "TRANSFORMED_SOURCE".into(),
        occurrence: None,
        url: None,
    };
    let mut evidence: ServiceEvidence = serde_json::from_value(json!({
        "schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"rev",
        "serviceDigest":"d","extractor":"test","runtimeMode":"TEST","coverage":"PARTIAL",
        "boundaries":[],"contracts":{},
        "entrypoints":[],"observations":{},"sources":{}
    }))
    .unwrap();
    evidence.observations.insert(sym_obs.id.clone(), sym_obs);
    evidence.sources.insert(source.id.clone(), source);
    let checked = Check {
        schema: "test".into(),
        input_digest: "d".into(),
        context_digest: "d".into(),
        services: BTreeMap::from([("svc".into(), evidence)]),
        unresolved: BTreeMap::new(),
        interactions: BTreeMap::new(),
        scenarios: BTreeMap::new(),
        dependencies: BTreeMap::new(),
        source_inputs: None,
        composition: None,
    };
    assert_eq!(
        method_source(&checked, symbol).as_deref(),
        Some(source_text)
    );
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --locked -p clew --lib 'documentation::render::' -- --test-threads=1`
Expected: FAIL — `method_source` returns `None` because the current code looks for a `TRANSFORMED_SOURCE` observation by `symbol`, which does not exist here.

- [ ] **Step 4: Implement `method_source`**

Replace the body of `method_source` (keep the signature `fn method_source(checked: &Check, symbol: &str) -> Option<String>`) with:

```rust
fn method_source(checked: &Check, symbol: &str) -> Option<String> {
    let sources = checked.sources();
    // Real pipeline: a SYMBOL observation (with flow events) carries
    // `source_ids` pointing to retained source blobs in `checked.sources()`.
    let via_source_ids = checked
        .services
        .values()
        .find_map(|e| {
            e.observations
                .values()
                .find(|o| o.kind == "SYMBOL" && o.symbol == symbol)
        })
        .and_then(|o| {
            o.source_ids
                .iter()
                .find_map(|id| sources.get(id).map(|s| s.text.clone()))
        });
    if via_source_ids.is_some() {
        return via_source_ids;
    }
    // Legacy fallback: a TRANSFORMED_SOURCE observation carrying the source in
    // its normalized payload.
    let obs = checked.services.values().find_map(|e| {
        e.observations
            .values()
            .find(|o| o.kind == "TRANSFORMED_SOURCE" && o.symbol == symbol)
    })?;
    let norm = &obs.normalized;
    if let Some(s) = norm
        .pointer("/documentation/source")
        .and_then(Value::as_str)
    {
        return Some(s.to_string());
    }
    if let Some(s) = norm.pointer("/source").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    if let Some(s) = norm.as_str() {
        return Some(s.to_string());
    }
    None
}
```

- [ ] **Step 5: Run the tests to verify both paths pass**

Run: `cargo test --locked -p clew --lib 'documentation::render::' -- --test-threads=1`
Expected: PASS — `method_source_resolves_via_symbol_source_ids` and the existing `source_deepening_is_preferred_when_source_retained` both pass.

- [ ] **Step 6: Commit**

```bash
git add crates/clew/src/documentation/render.rs
git commit -m "fix(docs): resolve method source via SYMBOL source_ids"
```

---

### Task 2: Extract shared `method_write_read` in process_flow.rs

**Files:**
- Modify: `crates/clew/src/documentation/process_flow.rs` (`step_kind`, ~line 210)

- [ ] **Step 1: Read the current `step_kind`**

Read `crates/clew/src/documentation/process_flow.rs` around lines 205-230. `step_kind(kind, target)` currently inlines the WRITE/READ prefix arrays and returns `Some("D")` for `IF`/`LOOP`.

- [ ] **Step 2: Add `method_write_read` and refactor `step_kind`**

Replace the body of `step_kind` and add a new `pub(crate)` helper so the final result is:

```rust
/// Classify a method name as write (`W`) or read (`R`) by verb prefix.
/// Shared by the FLOW tree (`step_kind`) and the source-deepened tree
/// (`source_steps`).
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

/// Classify a flow step into a readability category for the pseudocode tree:
/// `W` (write / state change), `R` (read), `D` (decision). Returns `None` for
/// control-flow or unclassifiable steps, which are rendered without a prefix.
fn step_kind(kind: &str, target: &str) -> Option<&'static str> {
    if matches!(kind, "IF" | "LOOP") {
        return Some("D");
    }
    let method = target.rsplit('#').next().unwrap_or(target);
    let method = method.split('(').next().unwrap_or(method);
    method_write_read(method)
}
```

Ensure the old inline arrays are removed (they now live only in `method_write_read`).

- [ ] **Step 3: Run the process_flow tests**

Run: `cargo test --locked -p clew --lib 'documentation::process_flow::' -- --test-threads=1`
Expected: PASS — existing `step_kind`-driven tree tests still pass (behavior unchanged).

- [ ] **Step 4: Commit**

```bash
git add crates/clew/src/documentation/process_flow.rs
git commit -m "refactor(docs): share method write/read classifier"
```

---

### Task 3: Add `source_steps::tree` with categories and data-flow

**Files:**
- Modify: `crates/clew/src/documentation/source_steps.rs`
- Test: `crates/clew/src/documentation/source_steps.rs` (test module)

- [ ] **Step 1: Read `source_steps.rs` helpers**

Read `crates/clew/src/documentation/source_steps.rs`. Confirm these helpers exist and are usable: `strip_comments`, `method_body`, `signature`, `keep_args`, `call_label`, `condition`, `method_name`. `shorten_statement` collapses call arguments with `call_label`, so it is not used directly for data-flow — a dedicated `tree_statement` keeps short args.

- [ ] **Step 2: Write the failing tests**

Append to the `source_steps.rs` test module (`mod tests`):

```rust
#[test]
fn tree_renders_categorized_data_flow() {
    let src = "\
private void changeStatus() {
    Date changeDate = DateTimeHolder.getCurrentTime();
    taskInstance.setUpdateDate(changeDate);
    if (priority != null) {
        taskInstance.setTaskPriority(priority);
    }
    return;
}";
    let tree = tree(
        src,
        "method:class:svc.TaskService#changeStatus()V",
    )
    .unwrap();
    assert!(
        tree.starts_with("Вход: changeStatus()\n"),
        "{tree}"
    );
    // data-flow binding preserved: time read, then written into updateDate
    assert!(
        tree.contains("[W] changeDate = DateTimeHolder.getCurrentTime(...)"),
        "{tree}"
    );
    assert!(
        tree.contains("[W] taskInstance.setUpdateDate(changeDate)"),
        "{tree}"
    );
    // decision + nested write
    assert!(
        tree.contains("[D] if (priority != null) then\n  [W] taskInstance.setTaskPriority(priority)"),
        "{tree}"
    );
    // control flow has no category
    assert!(tree.ends_with("return\n"), "{tree}");
}

#[test]
fn tree_returns_none_on_unparseable_source() {
    assert!(tree("no body here", "method:x").is_none());
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --locked -p clew --lib 'documentation::source_steps::' -- --test-threads=1`
Expected: FAIL — `tree` is not defined.

- [ ] **Step 4: Implement `tree_statement`, `statement_kind`, and `tree`**

Add these functions to `source_steps.rs` (place after `shorten_statement` / near `condition`):

```rust
/// Shorten a statement for the source tree: like `shorten_statement` but keeps
/// a call's arguments when short, so data-flow (`setUpdateDate(changeDate)`)
/// is preserved. Assignments keep their value expression's callee.
fn tree_statement(s: &str) -> String {
    let s = s.trim().trim_end_matches(';').trim();
    if let Some(rest) = s.strip_prefix("return") {
        let rest = rest.trim();
        return if rest.is_empty() {
            "return".to_string()
        } else {
            format!("return {}", keep_args(rest))
        };
    }
    if let Some(rest) = s.strip_prefix("throw") {
        let rest = rest.trim();
        return if rest.is_empty() {
            "throw".to_string()
        } else {
            format!("throw {}", keep_args(rest))
        };
    }
    if let Some((lhs, rhs)) = s.split_once('=') {
        let lhs = lhs.trim();
        let parts: Vec<&str> = lhs.split_whitespace().collect();
        let name = if parts.len() >= 2 && parts[0].chars().next().is_some_and(|c| c.is_uppercase())
        {
            parts[parts.len() - 1]
        } else {
            lhs
        };
        return format!("{} = {}", name, call_label(rhs.trim()));
    }
    keep_args(s)
}

/// Category prefix for a source-tree statement: assignments and write-verb
/// calls get `[W] `, read-verb calls get `[R] `; `return`/`throw` and
/// unclassifiable statements get no prefix.
fn statement_kind(stmt: &str) -> String {
    if stmt.starts_with("return") || stmt.starts_with("throw") {
        return String::new();
    }
    if let Some((_, rhs)) = stmt.split_once('=') {
        if !rhs.trim_start().starts_with('=') {
            return "[W] ".to_string();
        }
    }
    if let Some(open) = stmt.find('(') {
        let callee = stmt[..open].trim();
        let name = callee.rsplit('.').next().unwrap_or(callee).trim();
        if !name.is_empty() {
            if let Some(c) = super::process_flow::method_write_read(name) {
                return format!("[{c}] ");
            }
        }
    }
    String::new()
}

/// Render a method body as an indented pseudocode tree with `[W]/[R]/[D]`
/// categories and readable arguments/assignments (data-flow). Returns `None`
/// when the body cannot be isolated.
///
/// Control-flow constructs (`if`/`else`/`while`/`for`) nest by depth.
/// `try`/`catch`/`finally`/`switch`/`case`/`default`/`break`/`do` lines are
/// skipped without expanding their blocks in this prototype pass.
pub fn tree(source: &str, symbol: &str) -> Option<String> {
    let no_comments = strip_comments(source);
    let (open, close) = method_body(&no_comments)?;
    let head = no_comments[..open].trim();
    let body = &no_comments[open + 1..close];
    let mut out = String::new();
    out.push_str(&format!("Вход: {}\n", signature(head, symbol)));
    let mut depth = 0usize;
    let indent = |d: usize| "  ".repeat(d);
    for raw in body.split('\n') {
        let line = raw.trim();
        if line.is_empty() || line == "{" || line == ";" {
            continue;
        }
        if line.starts_with('}') {
            let rest = line.trim_start_matches('}').trim();
            if rest.starts_with("else if (") || rest.starts_with("elseif (") {
                depth = depth.saturating_sub(1);
                out.push_str(&format!("{}[D] else if ({}) then\n", indent(depth), condition(line)));
                depth += 1;
            } else if rest.starts_with("else") {
                depth = depth.saturating_sub(1);
                out.push_str(&format!("{}else\n", indent(depth)));
                depth += 1;
            } else {
                // standalone `}` (or `} catch`/`} while` continuation): close a
                // control block. `catch`/`finally` bodies are not expanded, so
                // their closing brace just balances the skipped open brace.
                depth = depth.saturating_sub(1);
            }
            continue;
        }
        if line.starts_with("else if (") || line.starts_with("elseif (") {
            depth = depth.saturating_sub(1);
            out.push_str(&format!("{}[D] else if ({}) then\n", indent(depth), condition(line)));
            depth += 1;
            continue;
        }
        if line.starts_with("else") {
            depth = depth.saturating_sub(1);
            out.push_str(&format!("{}else\n", indent(depth)));
            depth += 1;
            continue;
        }
        if line.starts_with("if (") {
            out.push_str(&format!("{}[D] if ({}) then\n", indent(depth), condition(line)));
            depth += 1;
            continue;
        }
        if line.starts_with("while (") || line.starts_with("for (") {
            out.push_str(&format!("{}[D] loop ({})\n", indent(depth), condition(line)));
            depth += 1;
            continue;
        }
        if line.starts_with("try")
            || line.starts_with("catch (")
            || line.starts_with("finally")
            || line.starts_with("switch (")
            || line.starts_with("case ")
            || line.starts_with("default")
            || line.starts_with("break")
            || line.starts_with("do")
        {
            continue;
        }
        let stmt = tree_statement(line);
        if !stmt.is_empty() {
            out.push_str(&format!("{}{}{}\n", indent(depth), statement_kind(&stmt), stmt));
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --locked -p clew --lib 'documentation::source_steps::' -- --test-threads=1`
Expected: PASS — both new tests pass, existing `document`/`shorten_statement` tests still pass.

- [ ] **Step 6: Format, run the full documentation module, commit**

Run:
```bash
cargo fmt --all
cargo test --locked -p clew --lib 'documentation::' -- --test-threads=1
```
Expected: all `documentation::` tests PASS, `cargo fmt` no diffs.

```bash
git add crates/clew/src/documentation/source_steps.rs
git commit -m "feat(docs): source-deepened lifecycle tree with data-flow"
```

---

## Verification (final)

- [ ] `cargo fmt --all` — no diffs
- [ ] `cargo test --locked -p clew --lib 'documentation::' -- --test-threads=1` — all pass
- [ ] No `app.js`/`style.css` changes (frontend untouched)

## Follow-up (out of scope, S3)

- Wire `source_steps::tree` into the lifecycle tree render (render.rs:2494), preferring source when available and falling back to `process_flow::tree`.
- Re-render arch-kasko and confirm `changeTaskStatus` shows `getCurrentTime → changeDate → setUpdateDate` on the page.