---
name: codeclew
description: Use Codeclew for bounded compiler- or syntax-backed code context, safe changes, multi-repository analysis, freshness checks, recovery, and privacy-safe diagnostics. Trigger when a task asks to inspect, explain, trace, or change supported source code through the clew CLI, or to diagnose a Codeclew run.
license: Apache-2.0
metadata:
  author: codeclew
  version: "0.2.0"
  repository: https://github.com/codeclew/codeclew-skill
---

# Codeclew

Use Codeclew to obtain bounded evidence before reading or changing supported
code. The skill supports Codex, Claude Code, and Agent Skills-compatible agents;
it requires the `clew` CLI, Git, and the host dependencies reported by doctor.
Preserve the user's scope and Codeclew's authority boundaries.

## Resolve and admit

1. Resolve `clew` from `PATH` once and use only that installed launcher for the
   entire task. Never use a source `./clew`, a `target` binary, a capsule binary,
   or `CODECLEW_RUNTIME_SEED`, and never edit `CODECLEW_HOME` directly. If the
   installed launcher is absent or rejected, stop the Codeclew workflow; do not
   build Codeclew or silently fall back to a source checkout.
2. Identify the absolute repository path, exact target ref, language, and exact
   compilation. Use explicit user input or authoritative project configuration;
   do not infer them from names. Ask when any remains ambiguous.
3. Run `clew capabilities` once. Require `runtimeMode=RELEASE`,
   `agentContract.schema=codeclew-agent-contract/1.0`,
   `agentContract.launcherAuthority=INSTALLED_RELEASE`, and
   `agentContract.sourceFallbackAllowed=false`. Stop on a missing or mismatched
   contract instead of trying another launcher.
4. Run `clew doctor attach` once, then run the exact task gate:
   `clew doctor task --repo <absolute-repo> --target-ref <exact-ref> --language
   <language> --profile <profile-id> --compilation <compilation> --operation
   <analysis-or-mutation>`. Repeat `--compilation` only for an explicitly
   multi-compilation task. Consume canonical JSON, not the optional `--human`
   view. `doctor provision` is a maintainer/bootstrap diagnostic and is not an
   admission step for an installed product task.
5. Continue only when both doctor results are `PASS`, every required check is
   `PASS`, and the support matrix admits the requested language, profile, and
   operation. A successful process exit does not override `ACTION_REQUIRED` or
   a read-only profile.

Never describe syntax-only, partial, declared, conditional, or unsure evidence
as compiler-verified behavior.

## Build bounded context

Open one session for the exact repository, ref, language, and compilation. Keep
the returned session and context identifiers. Use `context create`, and expand
an existing context only when the task needs additional bounded evidence. Do
not replace Codeclew context with a broad repository crawl before admission.

Use `clew <command> --help` for the installed version's exact arguments. Treat
session and context output as evidence tied to their recorded base commit.

## Prepare a change

Mutation is allowed only when the active support matrix marks the exact profile
as mutation-capable.

1. Use `change open`, retain its session and context identities, create a closed
   immutable edit plan, and use `change prepare`.
2. Run `change check-freshness --session <session-id>` immediately before
   prepare and again before publish.
3. Inspect `change status`, the bounded diff, validation results, authority
   digest, and every obligation. Run the repository's appropriate native tests.
4. Publish only after the user explicitly approves the reviewed candidate.
   Conditional publication additionally requires the exact prepared authority
   digest, `--allow-conditional`, and acknowledgement of every returned
   obligation.
5. Close completed sessions and garbage-collect them when no retained evidence
   is needed.

Freshness results are binding: continue on `FRESH`; stop and preserve developer
work on `DIRTY`; rebuild the session, context, and plan on `STALE`; repair access
on `UNAVAILABLE`; open a new session for more work after `TERMINAL`. Never clean,
reset, rebase, or replay user work to make a result fresh.

## Work across repositories

Use one exact session per repository, language, and compilation. For durable
work, bind the sessions to one mission with a canonical ChangeSpec and open a
workspace from an explicit private catalog. Use workspace context for bounded
cross-repository evidence. Use a raw thread only for ad-hoc read-only analysis
or a qualified thread surface not exposed by workspaces.

Treat catalog relationships as declared until independent authority proves
them. Workspace and thread results never authorize mutation or publication.
Closing them must not close or alter member sessions.

## Recover and report safely

Retry a worker crash once only when the typed error says it is retryable. For
`WORKTREE_RECOVERY_REQUIRED`, use `change recover` with the bound session and
run. For stale targets or compare-and-swap failures, open a new session rather
than replaying an old plan.

Do not paste raw Codeclew output, source, diffs, symbols, repository paths,
arguments, or `CODECLEW_HOME` contents into external issues or chats. Save the
single JSON result in a caller-owned mode-0600 file and run
`clew support summarize --input <absolute-private-result.json>`. Share only a
returned `SAFE_TO_SHARE` summary plus separately generated capabilities and
doctor JSON. If summarization rejects the artifact, keep it private and
escalate without transmitting it.
