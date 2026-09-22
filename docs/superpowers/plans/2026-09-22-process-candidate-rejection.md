# Process Candidate Rejection (noise filter + suppress list) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop generated/DTO noise (Lombok `equals`, accessors) and user-selected candidates from appearing on the "Internal processes" page of rendered documentation.

**Architecture:** Two orthogonal controls applied inside `process_candidates::catalog()` (single source of truth for candidate admission), consumed by both `docs process candidates` and `docs render`:
- **A — noise filter by default:** a deterministic `accessor_or_synthetic(symbol)` predicate blocks the structural orchestration reasons (`INTERNAL_ORCHESTRATION_CANDIDATE`, `LEXICAL_ORCHESTRATION_CANDIDATE`) for accessor/synthetic symbols. `EXPLICIT_ROOT` and `DISCOVERED_TRIGGER` are kept unconditional, so a real boundary trigger or explicitly declared root is never dropped.
- **B — persistent suppress list:** `catalog/process-candidates-suppress.json` holds `scope@symbol` entries the user rejects. `catalog()` drops any record whose `scope@symbol` is in the set. Managed via `docs process candidates suppress add/list`; read at render time in `publish_internal_phases` and threaded into `page_data`.

**Tech Stack:** Rust (clew crate), serde_json, clap subcommands, existing `documentation::store::Repository` (atomic writes, write lock), existing `documentation::process_candidates` test harness.

---

## File Structure

- **Modify:** `crates/clew/src/documentation/process_candidates.rs` — add `suppress` param + noise predicate + suppress filter + summary counters; add `load_suppress()` helper; update all existing test call sites; add new tests.
- **Modify:** `crates/clew/src/documentation/processes.rs` — add `--suppress` ad-hoc flag to `Candidates`; add `Suppress` subcommand (`add`/`list`); wire `load_suppress`; add `validate_suppress_entry`.
- **Modify:** `crates/clew/src/documentation/render.rs` — add `suppress` param to `page_data`, load the list in `publish_internal_phases`, pass through; update the existing `process_preview` test call site and add a suppress assertion.
- **Create:** `catalog/process-candidates-suppress.json` (produced by the new command; schema `codeclew-documentation-process-candidates-suppress/1.0`).

No `app.js` change: the client renders whatever `processCandidates.internal` the server emits, so server-side filtering is sufficient.

---

### Task 1: Noise filter for accessor/synthetic symbols (Option A)

**Files:**
- Modify: `crates/clew/src/documentation/process_candidates.rs`
- Test: `crates/clew/src/documentation/process_candidates.rs` (inline `mod tests`)

- [ ] **Step 1: Write the failing tests (append to `mod tests`)**

Add these two tests before the closing brace of `mod tests`:

```rust
#[test]
fn accessor_symbols_are_not_nominated_as_orchestration_candidates() {
    let mut e = evidence();
    add_method(&mut e, "obj-equals", ":obj", "equals", Some(orchestration()));
    add_method(&mut e, "obj-getname", ":obj", "getName", Some(orchestration()));
    add_method(&mut e, "worker-run", ":worker", "run", Some(orchestration()));
    let result = catalog(&e, &BTreeSet::new(), &BTreeSet::new()).unwrap();
    let ids: Vec<_> = result
        .records
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["worker-run"]);
    assert_eq!(result.summary["accessorFilteredCount"], 2);
}

#[test]
fn accessor_is_preserved_when_declared_as_explicit_root() {
    let mut e = evidence();
    add_method(&mut e, "obj-equals", ":obj", "equals", Some(orchestration()));
    let declared =
        catalog(&e, &BTreeSet::from(["obj-equals".into()]), &BTreeSet::new()).unwrap();
    assert_eq!(declared.records[0]["reasons"], json!(["EXPLICIT_ROOT"]));
    assert_eq!(declared.summary["accessorFilteredCount"], 0);
}
```

- [ ] **Step 2: Run tests to verify they fail to compile**

Run: `cargo test --locked -p clew --lib documentation::process_candidates:: -- --test-threads=1`
Expected: FAIL — `catalog` has the wrong arity (three args passed but signature takes two) and `accessorFilteredCount` does not exist yet.

- [ ] **Step 3: Add the predicate and wire the filter**

> Do NOT change the import block in this task. `bytes` and `store` are added only when actually used (Task 2 adds `store`, Task 3 uses `bytes`).

Add `accessor_or_synthetic` right after the `public_boundary` function:

```rust
fn accessor_or_synthetic(symbol: &str) -> bool {
    let s = symbol;
    let accessor_prefix = |prefix: &str| {
        s.len() > prefix.len()
            && s.starts_with(prefix)
            && s[prefix.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase())
    };
    matches!(
        s,
        "equals"
            | "hashCode"
            | "toString"
            | "clone"
            | "getClass"
            | "compareTo"
            | "compare"
            | "builder"
            | "build"
            | "toBuilder"
    ) || accessor_prefix("get")
        || accessor_prefix("set")
        || accessor_prefix("is")
        || accessor_prefix("has")
        || accessor_prefix("with")
}
```

Change the `catalog` signature:

```rust
pub fn catalog(
    evidence: &ServiceEvidence,
    explicit: &BTreeSet<String>,
    suppress: &BTreeSet<String>,
) -> Result<Catalog, ClewError> {
```

Inside `catalog`, next to the other counters (`let mut unavailable = 0;`), add:

```rust
let mut suppressed = 0;
let mut accessor_filtered = 0;
```

Right after `let mut reasons = Vec::new();` add:

```rust
let accessor = accessor_or_synthetic(declaration["symbol"].as_str().unwrap_or(""));
```

Guard the two orchestration reasons so accessors never get them:

```rust
if !accessor
    && targets.len() >= 2
    && controls > 0
    && declaration["status"] != "AMBIGUOUS"
{
    reasons.push("INTERNAL_ORCHESTRATION_CANDIDATE");
}
if !accessor
    && lexical_calls >= 2
    && controls > 0
    && declaration["status"] != "AMBIGUOUS"
{
    reasons.push("LEXICAL_ORCHESTRATION_CANDIDATE");
    gaps.insert("LEXICAL_CALL_TARGETS_UNRESOLVED".into());
}
```

Change the admission gate to count filtered accessors:

```rust
if reasons.is_empty() {
    if accessor {
        accessor_filtered += 1;
    }
    continue;
}
```

Add the counters to the summary (right after `"internalCandidateCount":internal,`):

```rust
"suppressedCount":suppressed,
"accessorFilteredCount":accessor_filtered,
```

- [ ] **Step 4: Update every existing `catalog(...)` call site in the file to the new arity**

All test call sites take `&BTreeSet::new()` (or `&selected`/`&BTreeSet::from([...])`) as the second argument; append a third `&BTreeSet::new()` to each:
- `catalog(&e, &BTreeSet::from([kind.into()])).is_err()` → `catalog(&e, &BTreeSet::from([kind.into()]), &BTreeSet::new()).is_err()`
- `catalog(&e, &BTreeSet::from(["method_declaration".into()]))` → add `, &BTreeSet::new()`
- `catalog(&e, &BTreeSet::new())` (multiple) → `catalog(&e, &BTreeSet::new(), &BTreeSet::new())`
- `catalog(&e, &selected)` (two places) → `catalog(&e, &selected, &BTreeSet::new())`

- [ ] **Step 5: Run the focused tests to verify they pass**

Run: `cargo test --locked -p clew --lib documentation::process_candidates:: -- --test-threads=1`
Expected: PASS (all existing + two new).

- [ ] **Step 6: Commit**

```bash
git add crates/clew/src/documentation/process_candidates.rs
git commit -m "feat(docs): filter accessor/synthetic symbols from process candidate admission"
```

---

### Task 2: Suppress list load + catalog filter (Option B, core)

**Files:**
- Modify: `crates/clew/src/documentation/process_candidates.rs`
- Test: `crates/clew/src/documentation/process_candidates.rs` (inline `mod tests`)

- [ ] **Step 1: Write the failing test (append to `mod tests`)**

```rust
#[test]
fn suppressed_scope_symbol_is_excluded_and_counted() {
    let mut e = evidence();
    add_method(&mut e, "worker-find", ":worker", "find", Some(vec![]));
    add_method(&mut e, "worker-dispatch", ":worker", "dispatch", Some(vec![]));
    add_method(&mut e, "worker-run", ":worker", "run", Some(orchestration()));
    add_method(&mut e, "worker-helper", ":worker", "helper", Some(orchestration()));
    let suppress = BTreeSet::from([":worker@run".to_owned()]);
    let result = catalog(&e, &BTreeSet::new(), &suppress).unwrap();
    let ids: Vec<_> = result
        .records
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["worker-helper"]);
    assert_eq!(result.summary["suppressedCount"], 1);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --locked -p clew --lib documentation::process_candidates::suppressed_scope_symbol_is_excluded_and_counted -- --test-threads=1`
Expected: FAIL — `suppressedCount` is missing and `worker-run` is still present.

- [ ] **Step 3: Implement the suppress drop inside `catalog`**

Immediately after the `if reasons.is_empty() { ... }` gate you added in Task 1, insert:

```rust
if suppress.contains(&format!("{}@{}", scope, observation.symbol)) {
    suppressed += 1;
    continue;
}
```

> Note: in `catalog`, the symbol is `observation.symbol` (there is no bare `symbol` binding in scope).

- [ ] **Step 4: Add the `load_suppress` helper (public, used by CLI and render)**

This helper uses `Repository`, `store::read`, and `store::MAX_RECORD`, so extend the import block first:

```rust
use super::{
    invalid,
    model::{Entrypoint, Observation, ServiceEvidence},
    store::{self, Repository},
};
```

Append this function after the `catalog` function (before `mod tests`):

```rust
/// Reads the persisted `scope@symbol` suppress list. Absence of the file is
/// an empty list, not an error. The file lives outside `RepositoryInputs` so
/// suppressing a candidate does not invalidate the retained check/digest.
pub fn load_suppress(repo: &Repository) -> Result<BTreeSet<String>, ClewError> {
    let path = repo.path("catalog/process-candidates-suppress.json")?;
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let raw: Value = store::read(&path, store::MAX_RECORD)?;
    Ok(raw["suppressed"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default())
}
```

- [ ] **Step 5: Run the focused tests to verify they pass**

Run: `cargo test --locked -p clew --lib documentation::process_candidates:: -- --test-threads=1`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/clew/src/documentation/process_candidates.rs
git commit -m "feat(docs): drop user-suppressed scope@symbol candidates from the process catalogue"
```

---

### Task 3: CLI — `--suppress` flag and `Suppress add/list` subcommand (Option B, surface)

**Files:**
- Modify: `crates/clew/src/documentation/processes.rs`

- [ ] **Step 1: Add the ad-hoc `--suppress` flag to the `Candidates` variant**

In the `Command` enum, extend the `Candidates` variant:

```rust
        #[arg(long, default_value = "all", value_parser = ["all", "internal", "trigger"])]
        lane: String,
        /// Ad-hoc scope@symbol entries to exclude for this query (not persisted).
        #[arg(long)]
        suppress: Vec<String>,
```

- [ ] **Step 2: Add the `Suppress` subcommand and its inner commands**

After the `Candidates` variant in the `Command` enum, add:

```rust
    /// Manage the persisted list of rejected candidates (scope@symbol).
    Suppress {
        #[arg(long)]
        root: PathBuf,
        #[command(subcommand)]
        command: SuppressCommand,
    },
```

After the `Command` enum, add:

```rust
#[derive(Debug, Subcommand)]
pub enum SuppressCommand {
    /// Add scope@symbol entries to the reject list (idempotent).
    Add {
        #[arg(long)]
        symbol: Vec<String>,
    },
    /// Print the current reject list.
    List {},
}
```

- [ ] **Step 3: Add `root` to the top-of-`run` match**

In `run`, extend the `root` match so the new variant resolves its root:

```rust
        Command::Show { root, .. }
        | Command::Put { root, .. }
        | Command::Inspect { root, .. }
        | Command::Prepare { root, .. }
        | Command::Suppress { root, .. } => root,
```

- [ ] **Step 4: Update the `Candidates` arm to union persisted + ad-hoc suppress**

Replace the body of the `Candidates` arm with:

```rust
        Command::Candidates {
            page,
            service,
            snapshot,
            declaration,
            lane,
            suppress,
        } => {
            let (checked, handle) = Check::retained(
                &repo,
                snapshot.as_deref(),
                &BTreeSet::from([service.clone()]),
            )?;
            let mut selected = super::process_candidates::load_suppress(&repo)?;
            for entry in suppress {
                validate_suppress_entry(&entry)?;
                selected.insert(entry);
            }
            let catalogue = super::process_candidates::catalog(
                &checked.services[&service],
                &declaration.into_iter().collect(),
                &selected,
            )?;
            let records: Vec<_> = catalogue
                .records
                .into_iter()
                .filter(|row| lane == "all" || row["lane"] == lane)
                .collect();
            let selection = digest(
                &json!({"snapshot":handle,"summary":catalogue.summary,"lane":lane,"records":records}),
            )?;
            super::cli::page(
                &selection,
                records,
                page.cursor.as_deref(),
                page.limit as usize,
                json!({"snapshot":handle,"authority":"PINNED_SNAPSHOT_NOT_REVERIFIED","lane":lane,"suppressed":selected,"catalogue":catalogue.summary}),
            )
        }
```

- [ ] **Step 5: Add the `Suppress` arm and the validation helper**

Add this match arm to `run` (after the `Candidates` arm):

```rust
        Command::Suppress { root, command } => {
            let repo = Repository::open(root)?;
            match command {
                SuppressCommand::Add { symbol } => {
                    let _lock = repo.lock()?;
                    let mut current = super::process_candidates::load_suppress(&repo)?;
                    for entry in symbol {
                        validate_suppress_entry(&entry)?;
                        current.insert(entry);
                    }
                    let data = super::bytes(&json!({
                        "schema": "codeclew-documentation-process-candidates-suppress/1.0",
                        "suppressed": current
                    }))?;
                    repo.atomic("catalog/process-candidates-suppress.json", &data)?;
                    Ok(json!({"status":"SAVED","suppressedCount":current.len()}))
                }
                SuppressCommand::List {} => {
                    let current = super::process_candidates::load_suppress(&repo)?;
                    Ok(json!({"schema":"codeclew-documentation-process-candidates-suppress/1.0","suppressed":current}))
                }
            }
        }
```

Add the validation helper (place it near `load_definition` in the same file):

```rust
fn validate_suppress_entry(entry: &str) -> Result<(), ClewError> {
    let (scope, symbol) = entry.split_once('@').ok_or_else(|| {
        invalid("suppress entry must be <scope>@<symbol>")
    })?;
    if scope.is_empty() || symbol.is_empty() {
        return Err(invalid("suppress entry must be <scope>@<symbol>"));
    }
    Ok(())
}
```

- [ ] **Step 6: Verify the crate compiles and the existing documentation tests still pass**

Run: `cargo test --locked -p clew --lib documentation::processes:: -- --test-threads=1`
Expected: PASS (compile succeeds; `invalid` is already imported in this module).

- [ ] **Step 7: Commit**

```bash
git add crates/clew/src/documentation/processes.rs
git commit -m "feat(docs): expose docs process candidates --suppress and suppress add/list subcommands"
```

---

### Task 4: Render respects the suppress list (Option B, published page)

**Files:**
- Modify: `crates/clew/src/documentation/render.rs`

- [ ] **Step 1: Add `suppress` to `page_data` and thread it into `catalog`**

Change the signature:

```rust
fn page_data(
    subject: &str,
    title: &str,
    subtitle: &str,
    n: &Narrative,
    checked: &Check,
    suppress: &BTreeSet<String>,
) -> Value {
```

Change the `catalog` call inside `page_data` (currently `super::process_candidates::catalog(evidence, &BTreeSet::new())`):

```rust
let catalog = super::process_candidates::catalog(evidence, &BTreeSet::new(), suppress)
    .expect("an empty explicit selection cannot name an invalid declaration");
```

Extend the `processCandidates` JSON object to surface the active list:

```rust
json!({"summary":catalog.summary,"internal":internal,"omittedInternal":omitted,"suppressed":suppress})
```

- [ ] **Step 2: Load the suppress list once in `publish_internal_phases` and pass it**

In `publish_internal_phases` (which already receives `repo`), right after the
`if let Some((id, binding)) = &previous { ... }` block, add:

```rust
let suppress = super::process_candidates::load_suppress(repo)?;
```

At the `page_data(...)` call site (currently `n, &checked,`), pass the list:

```rust
        let mut data = page_data(
            subject,
            title,
            super::reader::text(
                ui_language,
                "Service behavior and explicit evidence boundaries",
                "Поведение сервиса и границы подтверждённых сведений",
            ),
            n,
            &checked,
            &suppress,
        );
```

- [ ] **Step 3: Update the existing test call site**

At the bottom test, change:

```rust
let data = page_data("service:svc", "Service", "", &narrative, &checked);
```
to:
```rust
let data = page_data("service:svc", "Service", "", &narrative, &checked, &BTreeSet::new());
```

- [ ] **Step 4: Add a render-level suppress assertion**

In the same bottom test, after the existing `assert_eq!(data["processCandidates"]["omittedInternal"], 2);` line, add a suppression variant by re-rendering with a suppress set that drops the top candidate and asserting the first internal candidate changed:

```rust
        let suppress = BTreeSet::from([format!(":main@method-00")]);
        let filtered = page_data("service:svc", "Service", "", &narrative, &checked, &suppress);
        assert_eq!(filtered["processCandidates"]["suppressed"], json!([":main@method-00"]));
        assert!(
            filtered["processCandidates"]["internal"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["symbol"].as_str() != Some("method-00"))
        );
```

> Note: the bottom test builds ten methods `method-00..method-09`, each with `localCallTargetCount` 2 targeting `method-00`/`method-01`; symbol `method-00` is the lexicographically-first internal candidate, so suppressing `:main@method-00` must remove it from `internal`.

- [ ] **Step 5: Verify the render tests pass**

Run: `cargo test --locked -p clew --lib documentation::render:: -- --test-threads=1`
Expected: PASS (existing assertions unchanged, new suppress assertions hold).

- [ ] **Step 6: Commit**

```bash
git add crates/clew/src/documentation/render.rs
git commit -m "feat(docs): apply process candidate suppress list at render time"
```

---

### Task 5: Full verification (fmt + focused suites)

**Files:** none (verification only).

- [ ] **Step 1: Format check**

Run: `cargo fmt --all --check`
Expected: PASS (no diff).

- [ ] **Step 2: Run the affected test suites together**

Run:
```sh
cargo test --locked -p clew --lib documentation::process_candidates:: -- --test-threads=1
cargo test --locked -p clew --lib documentation::processes:: -- --test-threads=1
cargo test --locked -p clew --lib documentation::render:: -- --test-threads=1
```
Expected: PASS for all three.

- [ ] **Step 3: Optional end-to-end smoke (if a docs repo with retained evidence is handy)**

```sh
clew docs process candidates --root <docs-root> --service <svc> --lane internal
clew docs process candidates suppress add --root <docs-root> --symbol ":main@equals"
clew docs process candidates suppress list --root <docs-root>
```
Expected: `equals` absent from the first output; the `add` prints `"status":"SAVED"`; `list` shows `:main@equals`.

---

## Self-Review

- **Spec coverage:** A (Task 1) removes default noise; B core (Task 2) drops persisted rejects in `catalog`; B surface (Task 3) adds CLI to manage the list and ad-hoc flag; render (Task 4) makes the published page respect both. No `app.js` change needed because filtering is server-side in the single `catalog()` source of truth.
- **Placeholder scan:** every Rust edit shows exact before/after text; every command shows expected output; no TBD/TODO.
- **Type consistency:** `catalog` is updated to three args in all callers (processes.rs Task 3, render.rs Task 4, and all tests in Tasks 1–2). `load_suppress` returns `BTreeSet<String>`; `suppressed`/`accessorFilteredCount` fields match between `catalog` summary and the tests. `page_data`'s new `suppress: &BTreeSet<String>` param is threaded in both production and test call sites.
- **Notable constraint honored:** the suppress file is deliberately **not** added to `RepositoryInputs` (which is `deny_unknown_fields` and feeds `input_digest()`), so editing the suppress list does not invalidate the retained check or force a ~20-minute `docs check`. This is the intended behavior: rejection is applied at render time from the file.