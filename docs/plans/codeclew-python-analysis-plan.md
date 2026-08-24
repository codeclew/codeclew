# Codeclew Python analysis implementation plan

## Outcome

Codeclew opens an explicit `python:<import-root>#<source-root>` read-only
session for an arbitrary Git
repository, parses every selected tracked `.py` source from the immutable
session base revision with a pinned in-process grammar, without importing or
executing target code, and returns a
  bounded context backed by file, declaration, import, decorator-name and
  syntactic call-name facts. Dynamic import/name/call resolution remains explicit `UNSURE`
authority. PlanItForMe uses this contour as the first navigation path for
Python work; its generated AST overlay remains the JavaScript/JSX path and an
independent comparison oracle during cutover.

Python mutation and publication are not part of this slice. They remain
fail-closed until a Python validation contour is separately qualified.

## Contracts fixed before implementation

- Language URI: `language:python`.
- Capability: `analysis:python-syntax-facts`.
- Compilation selector: `python:<import-root>#<source-root>`, with two exact,
  normalized repository-relative directories (`.` is allowed). Source-root
  must equal import-root or be its descendant; `python:backend#tools` is
  rejected. It means every eligible tracked `.py` blob in the exact base
  commit below
  source-root, with module names interpreted relative to import-root.
  PlanItForMe uses `python:.#backend`.
- Parser authority: pinned `tree-sitter` and `tree-sitter-python` grammar bytes
  compiled into the Rust core and therefore into the content-addressed runtime.
  CPython, a virtualenv and target modules are never started or imported.
- Input authority: selected source blobs are read directly from the exact base
  commit into Codeclew CAS. Python session open creates no checkout, runs no
  worktree status, and does not consult live target paths. Symlinks are
  excluded and represented by an analysis boundary.
- Coverage: `PARTIAL` and `UNSURE`, with publication-blocking obligations for
  runtime import/name resolution and dynamic Python behavior.
- Limits: at most 4096 files, 4 MiB per file, 128 MiB total source, one million
  syntax nodes per file/four million per generation, 131072
  declarations/imports/calls, 64 nesting levels and 4096 boundary records.
  Exact-commit tree discovery is streamed and capped at 262144 rows, 64 MiB of
  metadata and 4096 bytes per path before any blob or CAS write. A
  project or file over a source/fact limit fails atomically with
  `RESOURCE_LIMIT`; it never silently analyzes a prefix. Syntax-error nodes
  remain analyzable and create deterministic `PARTIAL` boundaries.
- Input is the immutable base revision bound when the session opens. Target
  worktree modifications, deletions and untracked files are outside that
  authority and are never read. The target index, including unmerged entries,
  is not an input. Tracked symlinks produce boundaries rather than source.
  Only UTF-8 source is parsed; another encoding produces a per-file boundary
  and blocking obligation.
- Supported facts: source file, class/function/async-function declaration,
  import/import-from, decorator identifier (never its arguments), and
  syntactic call name. There is no framework-specific route semantics.
- No dependency installation, virtualenv activation, project-local helper,
  plugin loading, target-code execution, ambient `PYTHONPATH`, parser
  subprocess or interpreter locator. Git lazy fetch and every transport
  protocol are disabled, so a missing promised object fails without network or
  remote-helper execution.
- Durable facts contain identifiers, paths, ranges and hashes, but no comments,
  docstrings, default values, decorator arguments or other source literals.
  Context intentionally contains private,
  user-requested source windows under the existing per-window/per-response
  budgets; this is analysis evidence rather than a privacy leak.

## Work graph

```text
P0 baseline and contracts
  -> P1 language/session admission
  -> P2 Python model and isolated parser
  -> P3 adapter facts and honest completeness
  -> P4 generation/context integration
  -> P5 generic fixtures and fail-closed tests
  -> P6 PlanItForMe cold/warm analysis E2E
  -> P7 PlanItForMe navigation cutover
  -> P8 independent implementation review and final verification
```

## Steps and Definition of Done

### P0 — Baseline and oracle

- Record PlanItForMe HEAD, cleanliness, Python version, tracked Python file
  count/bytes, and current `tools/build_ast_index.py --check` result.
- Select at least three practical analysis questions covering an API boundary,
  a service dependency and tests/callers.
- Freeze these PlanItForMe questions and expected surfaces:
  1. Task creation: `create_task_item`, `TaskItemCreateRequest`,
     `test_task_item_actions`; primary `backend/planning_items_api.py`,
     additional production `backend/planning_items.py` and
     `backend/planning_item_schemas.py`, test
     `backend/tests/test_task_item_actions.py`.
  2. Agent apply boundary: `post_agent_planner_apply`,
     `apply_agent_planner_mutation`, `own_agent_parity`; primary
     `backend/agent_access_api.py`, additional production
     `backend/agent_access.py`, test `backend/tests/test_own_agent_parity.py`.
  3. Yandex LLM transport: `_yandex_request`, `resolve_model_runtime`,
     `test_yandex_transport_failure_is_wrapped_as_llm_error`; primary
     `backend/llm_client.py`, additional production `backend/byom_profiles.py`,
     test `backend/tests/test_llm_client.py`.

DoD: baseline is reproducible, target repository is unchanged, and expected
files/symbols are frozen from canonical docs plus the existing AST overlay.

### P1 — Language and session authority

- Add `SessionLanguage::Python`, CLI `--language python`, URI handling and exact
  Python selector validation.
- Reject Python model-cache modes other than `NON_CACHEABLE`.
- Make all language routing exhaustive in `session`, `valid_compilation`,
  `generation_service`, `context_v2` and `lib.rs`.
- Change both mutation guards (`main.rs` and `task_run_v2.rs`) plus candidate
  generation to allowlist Kotlin, so Python and Rust fail before candidate
  creation.

DoD: valid Python sessions open; malformed selectors, cache modes and mutation
attempts fail before analysis or target writes; a negative test rejects sibling
roots such as `python:backend#tools`; Kotlin and Rust behavior is unchanged.

### P2 — Generic project model and in-process parser

- Add a path-free Python project model bound to grammar/package versions,
  parser protocol and snapshot authority.
- Parse source bytes from CAS through the pinned in-process grammar. Do not
  consult a Python executable, target cwd, environment or filesystem path.
- Traverse with explicit node/fact budgets and canonicalize all parser output.
- Build a selector-scoped snapshot directly from the exact base commit before
  reading blobs or writing CAS; never create the generic source worktree or
  reuse repository-wide/Rust capture. Git plumbing disables hooks, fsmonitor,
  replacement refs and ambient global/system configuration.

DoD: parser handles classes, sync/async/nested functions, imports, decorators,
calls and syntax errors deterministically; base-revision/symlink and target
index/dirty/untracked exclusion is exact; malicious paths, non-UTF-8 sources, oversized data,
excessive nesting and parser failures are typed and fail closed or emit the
specified per-file boundary; a fixture
proves target module top-level code is never executed. Parser versions and
adapter digest change together and are visible in non-path evidence.

### P3 — Adapter and evidence

- Register a Python adapter through the existing language adapter protocol.
- Publish granular CAS facts in deterministic batches and validate every
  repository-relative path/range/schema before publication.
- Emit explicit boundaries and completeness obligations for dynamic imports,
  monkey patching, descriptor dispatch and unresolved calls. Decorator
  arguments, docstrings and arbitrary literals are never copied into facts;
  selected context may show them under the normal source-window budget.

DoD: identical input produces byte-identical generation/query identities;
natural identifier aliases find declarations/importers/call-sites; no absolute
path or unbounded source body appears in fact metadata, and context snippets
remain within the existing source-window and 64 KiB projection budgets;
partial authority cannot be upgraded to `COMPLETE`.

### P4 — Generation and bounded context

- Add the Python generation branch using snapshot/model/adapter authority,
  existing DAG, generation merge, bounded query index and incremental receipt.
- Use grammar version as the existing generic `compilerVersion` display field
  and report zero worker requests; do not fabricate compiler/worker activity.
- Keep first implementation full-analysis plus immutable warm reuse; no daemon
  or project-local cache is introduced.
- Extend context projection to `language:python`.

DoD: first context creates a Python generation, subsequent contexts reuse it
without invoking the parser again, stdout stays within 64 KiB, corruption/tamper
checks remain fail-closed, and unrelated language tests remain green.

### P5 — Generic acceptance fixtures

- Add a framework-neutral Python fixture with packages, relative/absolute
  imports, decorators with literals, nested/async declarations, test callers,
  a syntax-error boundary and a module whose top-level code would create a
  sentinel if executed.
- Cover managed CLI admission, query selection, deterministic output,
  base-revision snapshot authority, limits and mutation refusal.

DoD: focused tests pass without network or installed target dependencies, the
sentinel is absent, source/test navigation is useful, and the fixture contains
no PlanItForMe names or rules.

### P6 — PlanItForMe product-repo proof

- Open one `python:.#backend` session at the exact current ref/HEAD.
- Run at least three frozen analysis tasks and compare returned files/facts with
  canonical docs, the generated AST oracle and a timed default `rg` pass.
- Record cold phase timing, warm timing, fact/query-index sizes, coverage and
  missing evidence. Do not run product data or live integrations. For each
  frozen question, its named primary production file must rank in the top 5
  and its named test file in the top 10. Require 3/3
  questions to pass, cold context <= 120 s, warm context <= 30 s, projection
  <= 64 KiB and Python generation plus query-index payload <= 64 MiB.

DoD: Codeclew meets every numeric recall/latency/size threshold, results
disclose `UNSURE`, and PlanItForMe remains clean. If either of two bounded
optimization attempts misses a threshold, stop without P7 and report the
measured gap instead of adding project-specific routing.

### P7 — PlanItForMe navigation cutover

- Update PlanItForMe agent instructions and its AST navigation skill so Python
  work starts with Codeclew `python:.#backend`; keep the generated AST index for
  JavaScript/JSX and as an explicit fallback when Codeclew is unavailable.
- Update canonical verification/service documentation because the source
  navigation workflow changes. Do not add a personal absolute path or a second
  Codeclew launcher.
- Portable local discovery is explicit: when uncommitted
  `CODECLEW_SOURCE_ROOT` names a Codeclew checkout, execute its `./clew` with
  PlanItForMe passed through `--repo`; when unset/unavailable, use the existing
  AST overlay. The repository does not guess sibling paths or install a shim.

DoD: instructions contain a portable launcher discovery rule, exact commands,
  fallback behavior and honest read-only limitation; AST check and
  `bin/audit_contour.sh` are run; no personal paths, secrets or generated run
  reports are committed.

### P8 — Independent review and completion

- Run fmt, clippy, the full Rust suite, bootstrap tests affected by runtime
  identity, generic Python acceptance and the PlanItForMe E2E.
- Give the exact diff, tests and E2E evidence to an independent reviewer; fix
  every critical/major finding and repeat focused verification.
- Commit Codeclew and PlanItForMe changes separately. Push only on explicit
  request.

DoD: independent verdict is `PASS`, Codeclew changes and the exact PlanItForMe
  documentation hunks are committed separately without absorbing unrelated
  dirty work, managed sessions and temporary artifacts are closed/collected,
  commits are reported, and the remaining boundary to Python mutation is
  explicit.

## Stop conditions

- Do not weaken `UNSURE` or publication blocking to make an E2E pass.
- Do not execute or import target modules, install project dependencies, read
  `.env`, or use PlanItForMe's generated index as Codeclew runtime input.
- If the in-process grammar cannot remain path-private and bounded, stop before
  integrating the adapter and replace the parser design.
- If PlanItForMe requires project-specific routing rules for acceptable recall,
  keep it as comparison evidence and improve generic declaration/import/call
  facts instead.
