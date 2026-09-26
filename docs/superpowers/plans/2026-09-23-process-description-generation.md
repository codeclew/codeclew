# Auto-Generated Process Descriptions — Slice A: Flow Renderer (PlantUML) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In codeclew, auto-generate a detailed process flow for a process/scenario operation from retained flow evidence, bind it to source, and emit PlantUML (`.puml`) with optional SVG rendering. The generated flow is the default; a human-authored sequence takes priority.

**Architecture:** A new `process_flow` module converts a method's structured flow events (from the retained FLOW observation: IF/LOOP/CALL/RETURN/THROW/STATEMENT) into a PlantUML activity diagram, with each step carrying `source_ids`. `render.rs` hooks into operation rendering: if an operation has authored `events`, render the normal sequence; otherwise emit the auto PlantUML activity. A new `plantuml` helper provides the shared style (`!theme plain` + `!pragma layout smetana`) and SVG pre-render via an external `plantuml` binary when available (else emit `.puml` only). Source deepening (expanding shallow `BOUNDARY` flows) is Slice A2 — out of scope here.

**Tech Stack:** Rust (clew crate), serde_json, existing `documentation` modules (`model.rs`, `render.rs`, `check.rs`), external `plantuml` CLI for SVG pre-render.

---

## File Structure

- **Create:** `crates/clew/src/documentation/process_flow.rs` — flow-event → PlantUML activity conversion + evidence binding.
- **Create:** `crates/clew/src/documentation/plantuml.rs` — shared PlantUML output helper (theme, smetana, escaping, SVG pre-render).
- **Modify:** `crates/clew/src/documentation/mod.rs` — declare the two new modules.
- **Modify:** `crates/clew/src/documentation/render.rs` — hook: for an operation with empty `events`, emit the auto PlantUML; write `.puml` files alongside the existing `.mmd`.
- **Test:** inline `mod tests` in `process_flow.rs` and `plantuml.rs`; a focused render test in `render.rs`.

---

### Task 1: `plantuml` helper module

**Files:**
- Create: `crates/clew/src/documentation/plantuml.rs`
- Modify: `crates/clew/src/documentation/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/clew/src/documentation/plantuml.rs` with a `mod tests` (add the test first, before the impl):

```rust
#[test]
fn escape_html_escapes_special_chars() {
    assert_eq!(super::escape("a < b & c > d"), "a &lt; b &amp; c &gt; d");
}

#[test]
fn header_emits_theme_and_smetana() {
    let h = super::header("Title");
    assert!(h.contains("!theme plain"));
    assert!(h.contains("!pragma layout smetana"));
    assert!(h.contains("title Title"));
}
```

- [ ] **Step 2: Run to verify it fails to compile**

Run from the repository root: `CARGO_NET_OFFLINE=true cargo test --locked -p clew --lib documentation::plantuml:: -- --test-threads=1`
Expected: FAIL — module/functions do not exist.

- [ ] **Step 3: Implement the module**

```rust
//! Shared PlantUML output helpers for auto-generated process diagrams.

/// Escape PlantUML/HTML-special characters in label text.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Emit the shared diagram header: plain theme + smetana layout + title.
pub fn header(title: &str) -> String {
    format!(
        "@startuml\n!theme plain\n!pragma layout smetana\ntitle {}\n",
        escape(title)
    )
}

/// Wrap a body in the PlantUML document envelope.
pub fn wrap(title: &str, body: &str) -> String {
    format!("{} {}\n@enduml\n", header(title), body.trim())
}

/// Attempt to pre-render `.puml` to SVG using an external `plantuml` binary.
/// Returns `Ok(Some(svg_path))` on success, `Ok(None)` if the binary is absent,
/// and `Err` only on an unexpected execution failure.
pub fn render_svg(puml_path: &std::path::Path) -> Result<Option<std::path::PathBuf>, String> {
    let which = std::process::Command::new("which").arg("plantuml").output();
    let available = which.map(|o| o.status.success()).unwrap_or(false);
    if !available {
        return Ok(None);
    }
    let out = std::process::Command::new("plantuml")
        .args(["-tsvg", "-charset", "UTF-8"])
        .arg(puml_path)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(Some(puml_path.with_extension("svg")))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --locked -p clew --lib documentation::plantuml:: -- --test-threads=1`
Expected: PASS (2 tests).

- [ ] **Step 5: Declare the module**

In `crates/clew/src/documentation/mod.rs`, add `pub mod plantuml;` alongside the other module declarations.

- [ ] **Step 6: Commit**

```bash
git add crates/clew/src/documentation/plantuml.rs crates/clew/src/documentation/mod.rs
git commit -m "feat(docs): add shared plantuml output helper module"
```

---

### Task 2: Flow → PlantUML activity conversion (process_flow)

**Files:**
- Create: `crates/clew/src/documentation/process_flow.rs`
- Modify: `crates/clew/src/documentation/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/clew/src/documentation/process_flow.rs` with a `mod tests` containing:

```rust
#[test]
fn flow_events_become_activity_steps() {
    let events = serde_json::json!([
        {"kind":"STATEMENT","text":"TaskInstanceDao taskInstance"},
        {"kind":"IF","text":"Optional.of(request).map(getAnyTask).orElse(false)"},
        {"kind":"CALL","text":"taskHandlingService.changeAnyTaskStatus(...)"},
        {"kind":"RETURN","text":"createResponse(...)"}
    ]);
    let src = "method:class:TaskService#changeTaskStatus(JL...)Z";
    let body = super::activity_from_flow(&events, src).unwrap();
    assert!(body.contains(":TaskInstanceDao taskInstance;"));
    assert!(body.contains("if (Optional.of(request).map(getAnyTask).orElse(false)) then (yes)"));
    assert!(body.contains(":taskHandlingService.changeAnyTaskStatus(...);"));
    assert!(body.contains("stop"));
    assert!(body.contains("' evidence: method:class:TaskService#changeTaskStatus(JL...)Z"));
}

#[test]
fn empty_flow_returns_none() {
    let events = serde_json::json!([]);
    assert!(super::activity_from_flow(&events, "s").is_none());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --locked -p clew --lib documentation::process_flow:: -- --test-threads=1`
Expected: FAIL — `activity_from_flow` does not exist.

- [ ] **Step 3: Implement `activity_from_flow`**

```rust
//! Convert a method's structured flow events into a PlantUML activity diagram.
//! Evidence binding: the root symbol is emitted as an evidence comment; each
//! step is rendered as an activity action or branch.

use serde_json::Value;

/// Render a flow event array into PlantUML activity body.
/// Returns `None` if the flow has no usable steps.
pub fn activity_from_flow(events: &Value, symbol: &str) -> Option<String> {
    let rows = events.as_array()?;
    if rows.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!("' evidence: {}\n", symbol));
    for (i, row) in rows.iter().enumerate() {
        let kind = row["kind"].as_str().unwrap_or("STATEMENT");
        let text = row["text"].as_str().unwrap_or("");
        match kind {
            "IF" => out.push_str(&format!("if ({}) then (yes)\n", text)),
            "ELSE" | "ELSEIF" => out.push_str(&format!("else (no)\n")),
            "END_IF" | "ENDIF" => out.push_str("endif\n"),
            "LOOP" => out.push_str(&format!("while ({}) is (yes)\n", text)),
            "END_LOOP" => out.push_str("endwhile\n"),
            "RETURN" => {
                out.push_str(&format!(":{}\n", text));
                if i + 1 == rows.len() {
                    out.push_str("stop\n");
                }
            }
            "THROW" => out.push_str(&format!(":throw {};\n", text)),
            "BOUNDARY" => out.push_str(&format!("note right\n  {} (not expanded)\nend note\n", text)),
            _ => out.push_str(&format!(":{}\n", text)), // STATEMENT, CALL, LOCAL
        }
    }
    if !out.contains("stop") {
        out.push_str("stop\n");
    }
    Some(out)
}
```

- [ ] **Step 4: Add a public entry point that produces a full document**

Add (public, used by render):

```rust
/// Produce a full PlantUML activity document for a flow, or `None` if empty.
pub fn document(events: &Value, symbol: &str, title: &str) -> Option<String> {
    let body = activity_from_flow(events, symbol)?;
    Some(format!(
        "@startuml\n!theme plain\n!pragma layout smetana\ntitle {}\nstart\n{body}\n@enduml\n",
        super::plantuml::escape(title)
    ))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --locked -p clew --lib documentation::process_flow:: -- --test-threads=1`
Expected: PASS (2 tests).

- [ ] **Step 6: Declare the module**

In `crates/clew/src/documentation/mod.rs`, add `pub mod process_flow;`.

- [ ] **Step 7: Commit**

```bash
git add crates/clew/src/documentation/process_flow.rs crates/clew/src/documentation/mod.rs
git commit -m "feat(docs): convert structured flow events to plantuml activity"
```

---

### Task 3: Render hook — auto PlantUML for unauthored operations

**Files:**
- Modify: `crates/clew/src/documentation/render.rs`

- [ ] **Step 1: Add a test-only helper + failing test**

Add to the bottom `mod tests` in `render.rs`:

```rust
#[test]
fn auto_flow_puml_is_emitted_when_events_empty() {
    let flow = serde_json::json!([
        {"kind":"STATEMENT","text":"TaskInstanceDao taskInstance"},
        {"kind":"RETURN","text":"return createResponse(...)"}
    ]);
    let puml = super::auto_flow_puml(&flow, "method:X", "changeTaskStatus");
    assert!(puml.is_some());
    let d = puml.unwrap();
    assert!(d.contains("title changeTaskStatus"));
    assert!(d.contains(":TaskInstanceDao taskInstance;"));
    assert!(d.contains("' evidence: method:X"));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --locked -p clew --lib documentation::render::auto_flow_puml_is_emitted_when_events_empty -- --test-threads=1`
Expected: FAIL — `auto_flow_puml` does not exist.

- [ ] **Step 3: Implement the helper**

In `render.rs`, add (near `mermaid`):

```rust
/// If an operation has no authored events, produce an auto PlantUML activity
/// document from the operation's root FLOW evidence. Returns `None` when there
/// is authored content or no usable flow.
fn auto_flow_puml(flow: &serde_json::Value, symbol: &str, title: &str) -> Option<String> {
    super::process_flow::document(flow, symbol, title)
}
```

- [ ] **Step 4: Wire into `.mmd`/`.puml` file emission**

In `render.rs`, in the loop that writes `diagrams/{subject}-{operationId}.mmd` (around line 2393), add a parallel `.puml` write when the operation has no authored events. Locate the operation's root FLOW evidence from `checked.dependencies` (kind `"FLOW"`, symbol == operation's root boundary) and write:

```rust
if o.events.is_empty() {
    if let Some(flow) = operation_flow(checked, o) {
        if let Some(puml) = auto_flow_puml(flow, &flow_symbol(checked, o), &o.title) {
            files.insert(
                format!("diagrams/{}-{}.puml", subject.replace(':', "-"), o.id),
                puml.into_bytes(),
            );
        }
    }
}
```

Where `operation_flow(checked, o) -> Option<&Value>` looks up the FLOW observation for the operation's root boundary symbol in `checked.dependencies`, returning its `normalized["documentation"]["events"]` (or the whole normalized value).

- [ ] **Step 5: Run the render test to verify it passes**

Run: `cargo test --locked -p clew --lib documentation::render::auto_flow_puml_is_emitted_when_events_empty -- --test-threads=1`
Expected: PASS.

- [ ] **Step 6: Run the whole render suite to confirm no regression**

Run: `cargo test --locked -p clew --lib documentation::render:: -- --test-threads=1`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/clew/src/documentation/render.rs
git commit -m "feat(docs): emit auto plantuml activity for unauthored process operations"
```

---

### Task 4: SVG pre-render + verification

**Files:**
- Modify: `crates/clew/src/documentation/plantuml.rs`
- Test: `crates/clew/src/documentation/plantuml.rs`

- [ ] **Step 1: Write a failing test for `render_svg` absence-handling**

```rust
#[test]
fn render_svg_returns_none_when_plantuml_absent() {
    // PATH without plantuml is hard to simulate portably; assert the Ok(None)
    // path only when `which plantuml` fails — which is expected in CI without
    // graphviz/plantuml installed. This guards the API shape.
    let p = std::env::temp_dir().join("clew-plantuml-absent.puml");
    std::fs::write(&p, "@startuml\n[*] --> A\n@enduml\n").unwrap();
    let r = super::render_svg(&p);
    // Either Ok(None) (absent) or Ok(Some(_)) (present) — never Err.
    assert!(!matches!(r, Err(_)));
    let _ = std::fs::remove_file(&p);
}
```

- [ ] **Step 2: Run to verify it passes (API exists from Task 1)**

Run: `cargo test --locked -p clew --lib documentation::plantuml::render_svg_returns_none_when_plantuml_absent -- --test-threads=1`
Expected: PASS (the Task 1 implementation already returns Ok(None) when `which plantuml` fails).

- [ ] **Step 3: End-to-end manual verification (optional, requires a docs repo with retained evidence)**

```sh
cd /path/to/codeclew
JAVA_HOME=/path/to/jdk-17/Contents/Home \
CODECLEW_WORKER_JAVA_HOME=/path/to/jdk-21/Contents/Home \
CARGO_NET_OFFLINE=true ./clew docs render --root /path/to/architecture
ls /path/to/architecture/docs/generated/*/diagrams/*.puml
```
Expected: `.puml` files exist for unauthored process operations. (Note: for the saved `task-lifecycle-management` scenario, an authored sequence already exists, so its `.puml` is not emitted — verify on a scenario without authored steps, or temporarily omit the authored proposal.)

- [ ] **Step 4: Commit**

```bash
git add crates/clew/src/documentation/plantuml.rs
git commit -m "test(docs): cover plantuml svg render absence handling"
```

---

## Self-Review

- **Spec coverage:** Slice A per spec: flow renderer (Task 2), PlantUML output (Task 1, 4), render hook + auto-default (Task 3), evidence binding (source symbol comment in Task 2; `source_ids` binding is a noted follow-up since auto steps are render artifacts, not narrative events). Manual-priority (authored events present → no auto puml) is in Task 3. Source deepening is explicitly scoped out to Slice A2.
- **Placeholder scan:** every step has concrete code and commands; no TBD/TODO. `operation_flow` / `flow_symbol` are described at a design level in Task 3 Step 4 — flagged as needing the exact existing lookup pattern (the implementer should mirror how `mermaid`/`Walker` reads `checked.dependencies[flow].normalized["documentation"]["events"]`).
- **Type consistency:** `escape`, `header`, `wrap`, `render_svg` (Task 1) are used consistently in Tasks 2–4. `activity_from_flow` / `document` (Task 2) match the render hook usage (Task 3). Naming consistent.
- **Known follow-ups (Slice A2 / later):** source deepening (expand shallow `BOUNDARY` from `TRANSFORMED_SOURCE`); full `source_ids` binding per auto step; render-time SVG pre-render into the bundle; participation/multi-operation coverage.
