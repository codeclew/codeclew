---
name: codeclew
description: Use Codeclew for bounded Kotlin semantic changes, conditional Python or Rust syntax-backed changes, mission-bound local workspaces, multi-repository analysis threads, session freshness checks, recovery, and privacy-safe incident summaries. Trigger when a task asks to inspect or change code through Codeclew, coordinate or trace behavior across repositories, or diagnose a Codeclew run.
---

# Codeclew

Prefer the installed `clew` command. For source development, use the supported
launcher from the pinned Codeclew checkout; if the current repository is not
that checkout, require `CODECLEW_ROOT` and use `$CODECLEW_ROOT/clew`. Never
invoke a capsule binary or edit `CODECLEW_HOME` directly. In the commands below,
`clew` means the resolved installed or source launcher.

Codeclew emits canonical JSON by default. Agents must not use the optional
`--human` presentation provided by `capabilities` and `doctor`.

## Admit the task

1. Run `clew capabilities` and inspect the JSON support matrix.
2. Run `clew doctor --repo <absolute-repo> --target-ref <ref>`.
3. Continue only when every required doctor check is `PASS` and the requested
   language/profile supports the requested operation.
4. Require an explicit language and exact compilation. Do not guess them.

Strict compiler-backed mutation is limited to
`kotlin-2.4.10-gradle-single`. The operationally `PILOT_READY` `rust-syntax`
and `python-syntax` profiles allow only conditional mutation with their native
validators and explicit obligation acknowledgement. Kotlin previews and
multi-repository threads are read-only. Never turn partial or unsure evidence
into a verified claim.

## Work with one repository

For any admitted mutation profile, use `change open`, retain the returned session/context
identities, create a closed immutable edit plan, then use `change prepare`.
Inspect `change status` and all obligations. Before prepare and again before
publish, run:

```bash
clew change check-freshness --session <session-id>
```

- `FRESH`: continue.
- `DIRTY`: stop. Preserve the developer's work; do not clean or reset it.
- `STALE`: close the old session, open a new session, and rebuild context and
  plan against the new target commit.
- `UNAVAILABLE`: stop and repair the repository locator/access.
- `TERMINAL`: open a new session if more work is required.

Publish only after the user explicitly approves publication of the reviewed
candidate. Conditional validation remains conditional; unresolved obligations
must stay visible. Close and garbage-collect completed sessions.

For Python use `--language python` with
`--compilation 'python:<import-root>#<source-root>'`. For Rust use
`--language rust` with an exact Cargo target selector. Rust plans must contain
only `CARGO` validation. Python plans must contain only `PYTHON` validation with
arguments beginning `-m <safe.module>`; activate the project environment before
prepare. These contours provide bounded syntax evidence, not resolved semantic
authority. Publish only with `--allow-conditional`, the exact prepared authority
digest, and every obligation returned by `change status`, after reviewing the
bounded diff and successful project-native validation.

## Work across repositories

Open one session per exact repository/language/compilation. For a durable
development task, create one mission with a canonical ChangeSpec, then create a
mode-0600 canonical `codeclew-workspace-catalog-input/1.0` that covers exactly
two to four mission sessions and declares only safe member aliases and edges.
Use `workspace open`, `workspace inspect`, and `workspace context`. Treat every
catalog edge as `DECLARED_CATALOG`; compiler shape, artifact ownership, contract,
and runtime axes remain independently `UNKNOWN` unless a later authority proves
them. Closing the workspace must leave every member session open and unchanged.

Use raw `thread open` only for an ad-hoc read-only analysis view or for the
qualified Kotlin `thread callables`, `thread impact`, and `thread validate`
surfaces that do not yet have workspace facades. Thread and workspace results
are never mutation or publication authority. Close the analysis view; member
sessions remain separately owned and must be closed or collected explicitly.

## Recover and report

On a worker crash, retry once only when the typed error says it is retryable.
For `WORKTREE_RECOVERY_REQUIRED`, run `change recover` for the bound session and
run. For stale target or compare-and-swap failure, open a new session rather
than replaying an old plan.

Do not paste raw Codeclew output, source, diffs, symbols, repository paths,
arguments, or `CODECLEW_HOME` contents into an issue or chat. Capture the one
JSON result in a caller-owned mode-0600 file, then run:

```bash
clew support summarize --input /absolute/path/to/private-result.json
```

Share only the returned `SAFE_TO_SHARE` summary plus separately generated
`capabilities` and `doctor` JSON. Keep the original artifact local and private.
If summarization rejects the schema, preserve it locally and escalate without
transmitting it.

Use `docs/operations/p0-runbook.md` in the Codeclew checkout for exact commands,
installation, upgrades, and incident runbooks.
