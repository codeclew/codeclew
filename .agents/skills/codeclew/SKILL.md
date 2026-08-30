---
name: codeclew
description: Use Codeclew for bounded compiler- or syntax-backed code context, safe changes, multi-repository analysis, freshness checks, recovery, and privacy-safe diagnostics. Trigger when a task asks to inspect, explain, trace, or change supported source code through the clew CLI, or to diagnose a Codeclew run.
license: Apache-2.0
metadata:
  author: codeclew
  version: "0.2.3"
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
3. Admit the task through one of the two public atomic paths below. Their
   `admission` object is the runtime authority; do not spend a separate call on
   `clew capabilities` during an ordinary task. Require `runtimeMode=RELEASE`,
   `agentContract.schema=codeclew-agent-contract/1.0`,
   `agentContract.launcherAuthority=INSTALLED_RELEASE`, and
   `agentContract.sourceFallbackAllowed=false`. Use standalone capabilities
   only when the user explicitly asks about product support or runtime status.
4. For bounded
   read-only navigation, prefer `nav query`. For broader analysis or mutation,
   open the admitted task and first bounded context atomically:
   `clew context open --repo <absolute-repo> --target-ref <exact-ref> --language
   <language> --profile <profile-id> --compilation <compilation> --operation
   <analysis-or-mutation> --intent <intent> --term <term>`. Repeat
   `--compilation` and `--term` only for explicit additional authorities and
   roots. Require `admission.status=PASS` and retain the returned session and
   context identifiers.
5. On a typed readiness failure, run only the named diagnostic (`clew doctor
   attach` or the same exact `clew doctor task ...`) once, report its
   `nextAction`, and stop. `doctor provision` is a maintainer/bootstrap
   diagnostic and is not an admission step for an installed product task. A
   successful process exit never overrides `ACTION_REQUIRED` or a read-only
   profile.

Never describe syntax-only, partial, declared, conditional, or unsure evidence
as compiler-verified behavior.

Codeclew sessions require write access to private `CODECLEW_HOME` managed state
and to Codeclew-owned Git worktree administration under the repository's Git
common directory. The source and candidate worktrees themselves may be used for
analysis and validation. These writes are expected even for a read-only task;
an agent sandbox or benchmark harness must allow them. They do not authorize
editing the user's checkout or changing the bound target ref. Only an explicit
mutation workflow may edit a candidate, and only `change publish` may update
the target ref. Treat a sandbox that blocks required managed-state or worktree
metadata writes as an environment admission failure, not an evidence failure.

## Navigate to relevant code

Use the public atomic query when a task starts from names or search terms:

```bash
clew nav query \
  --repo <absolute-repo> \
  --target-ref <exact-ref> \
  --language <language> \
  --profile <profile-id> \
  --compilation <compilation> \
  --term <term> \
  --source
```

Pass two to four discriminative code tokens from the request together when they
are available. These are agent-selected lexical identifiers, not requirements
inferred by Codeclew. `--intent` is optional and does not affect retrieval. Do
not pass a positional search phrase. Retain the returned session and context
identifiers. When these arguments are already known, run the command directly
instead of spending a call on `--help`.

The first response contains at most three fact-bound decision cards with exact
one-line previews. `--source` additionally returns the exact retained source
for only the highest-ranked card; omit it when names and locations are enough.
Select up to three other useful cards in one call by repeating `--candidate`;
each returns its retained fact and exact source window. Request relations only
when they are needed for those selected symbols:

```bash
clew nav expand \
  --session <session-id> \
  --from <context-id> \
  --candidate <candidate-id> \
  --candidate <candidate-id> \
  --source \
  --facet <callers-or-callees-or-tests>
```

Both source and facet are optional. Never reread a returned exact source window
with `sed`, `nl`, or another search tool unless a later observation contradicts
its bound digest. If none of the cards is sufficient, add only the missing
identifier with `nav expand --session <session-id> --from <context-id> --term
<term>`. If the exact identifier and file are already established, combine the
refinement and selection in one call: `nav expand --session <session-id> --from
<context-id> --term <exact-identifier> --file <repository-relative-file>
--source`. This fails closed on no match or same-file ambiguity. There is no
`--all`; narrow a reported truncation and do not expand
merely to collect every match. Term expansion returns a reconstructable patch:
apply upserts and removals, then `candidateOrder`; unchanged cards are omitted.
The child `contextId` and `evidenceDigest` bind the complete immutable evidence.

Admission already binds the exact base revision. Do not run a preliminary
`git rev-parse` or cleanliness check for read-only analysis; use one final
`git status --short` only when proving that the task made no repository change.

## Build bounded context

Open one session for the exact repository, ref, language, and compilation. Keep
the returned session and context identifiers. Use the atomic navigation path
above for name-led read-only work; use `context create` for a broader admitted
session. Expand an existing context only when the task needs additional bounded
evidence. Do not replace Codeclew context with a broad repository crawl before
admission.

Use `clew <command> --help` only when the relevant syntax is not already given
by this skill or the installed command rejects it. Treat session and context
output as evidence tied to their recorded base commit.

## Prepare a change

Mutation is allowed only when the active support matrix marks the exact profile
as mutation-capable.

1. Reuse the mutation-admitted session and context returned by `context open`,
   create a closed immutable edit plan, and use `change prepare`. The high-level
   prepare call waits for the first actionable run state; do not poll a live
   `PREPARING` run or restart it from another launcher.
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

Session/thread GC removes only proven-owned worktrees and terminal derived
metadata. For managed CAS space, use the default dry-run `clew storage gc`;
run `clew storage gc --apply` only when physical reclamation is explicitly in
scope. Never delete `CODECLEW_HOME` objects by path. Physical GC preserves every
transitively reachable retained root and waits for active readers/writers.

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
