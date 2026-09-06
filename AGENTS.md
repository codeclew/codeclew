# Developing Codeclew

## Scope and authority

This checkout is the Codeclew product source. For ordinary implementation,
debugging, and review, use native repository tools and edit the requested source
directly. `./clew` is the source-development launcher; installed `clew` is the
public-release launcher. Direct capsule binaries are unsupported.

The bundled `codeclew` skill describes using the installed product to obtain
managed evidence or prepare and publish managed changes. Apply it when the user
requests that workflow or the task specifically needs its evidence contract.
The repository name alone does not select that workflow. Its installed-release
admission and publication rules do not gate ordinary edits to this checkout.
If an explicitly selected managed workflow fails admission, report that failure;
do not silently replace its evidence or publication guarantees with native tools.

Follow the user's current scope and authorization. Treat dated plans, benchmark
protocols, and recorded results as scoped evidence, not new task instructions.
`docs/stabilization-first.md` and the stabilization controller apply to their
research and qualification runs; receipts are not prerequisites for ordinary
product development or CI. Preserve historical model IDs and measured results.

## Working loop

- Start with the nearest reviewable result. Read only the files needed to
  understand the change and its callers or tests; use `rg` for discovery.
- Carry authorized work through implementation and relevant verification.
  Resolve routine choices from context. Ask only when missing information
  materially changes scope, correctness, or an irreversible action.
- Preserve unrelated working-tree changes. Do not reset, clean, or rewrite
  user work to obtain a passing check. Do not edit private `CODECLEW_HOME`
  objects directly; use supported lifecycle commands when needed.
- Keep one primary workflow per task. Add process or documentation only when
  it contributes to the requested result; routine development needs no
  evidence ledger, attestation, or separate approval layer.
- After 30 minutes or 100 tool calls without a new artifact, confirmed test
  fact, or resolved blocker, report the facts and narrow the next slice.
  Let an already-running necessary command finish.
- Report the result, relevant checks, and remaining limitations concisely.
  Distinguish observed behavior from inference and unverified claims.

## Repository map

- `crates/clew`: Rust core and CLI; use `Cargo.toml` for active workspace members.
- `workers/kotlin*`: version-specific Kotlin compiler workers.
- `bootstrap` and `clew`: source runtime bootstrap and launcher.
- `schemas` and `fixtures`: protocol contracts and executable test inputs.
- `scripts` and `tools`: validation, packaging, and research harnesses.
- `skills/codeclew`: canonical portable skill. When changing the package, keep
  `.agents/skills/codeclew` and `.claude/skills/codeclew` copies identical.
- `docs/operations`: operational runbooks; `docs/plans` and `docs/product/validation`
  contain plans and evidence whose status and revision must be checked before use.

## Verification

Use the pinned Rust toolchain, Python 3.11+, JDK 21, and repository Gradle
wrapper. See `README.md` for complete source-build requirements.

Choose checks from the affected behavior:

- Rust: `cargo fmt --all --check`, then the relevant tests, for example
  `cargo test --locked -p clew --lib 'operations::tests::' -- --test-threads=1`.
  CLI behavior may require a focused `managed_cli` integration test.
- Python tooling: run the corresponding existing `scripts/test_*.py` or
  `tools/test_*.py` suite with `python3 -I -S`.
- Skill packaging: `python3 -I -S scripts/test_agent_skill.py`.
- Documentation or configuration only: validate affected syntax, links, and
  `python3 -I -S scripts/check_english_content.py`; do not rebuild the runtime.
- Kotlin or protocol changes: run the affected worker or contract checks;
  Rust-only verification does not establish worker correctness.

`./scripts/ci-verify.sh` is the full development/CI gate. Run it for a requested
full verification or merge-ready code change. During iteration, use focused
checks. Run expensive benchmarks, cold-runtime gates, self-hosting, release
qualification, or paid pilot arms only when those outcomes are in scope.
Do not repeat successful checks without a material change or a required final
gate. Add tests for changed behavior or regressions, not to mirror prose or
configuration literals. Keep repository content in English and private data
out of committed files; use the repository privacy check before publication.
