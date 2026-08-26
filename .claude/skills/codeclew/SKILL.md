---
name: codeclew
description: Use Codeclew for bounded Kotlin semantic changes, Python or Rust syntax context, multi-repository analysis threads, session freshness checks, recovery, and privacy-safe incident summaries. Trigger when a task asks to inspect or change code through Codeclew, trace behavior across repositories, or diagnose a Codeclew run.
---

# Codeclew

Prefer the installed `clew` command. For source development, use the supported
launcher from the pinned Codeclew checkout; if the current repository is not
that checkout, require `CODECLEW_ROOT` and use `$CODECLEW_ROOT/clew`. Never
invoke a capsule binary or edit `CODECLEW_HOME` directly. In the commands below,
`clew` means the resolved installed or source launcher.

## Admit the task

1. Run `clew capabilities` and inspect the JSON support matrix.
2. Run `clew doctor --repo <absolute-repo> --target-ref <ref>`.
3. Continue only when every required doctor check is `PASS` and the requested
   language/profile supports the requested operation.
4. Require an explicit language and exact compilation. Do not guess them.

Mutation is currently limited to the `kotlin-2.4.10-gradle-single` profile.
Kotlin 2.3 Maven, Python, Rust, and multi-repository threads are read-only.
Never turn partial or unsure evidence into a verified claim.

## Work with one repository

For Kotlin mutation, use `change open`, retain the returned session/context
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
`--language rust` with an exact Cargo target selector. These contours provide
bounded read-only syntax evidence; use the repository's own tests and runtime
checks for dynamic behavior.

## Work across repositories

Open one session per exact repository/language/compilation, then bind two to
eight sessions with `thread open`. Use `thread context` and, for qualified
Kotlin members, `thread callables`, `thread impact`, and `thread validate`.
Treat declared topology as a hypothesis to verify. Thread results are never
mutation or publication authority. Close and garbage-collect the thread; its
member sessions remain separately owned.

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
