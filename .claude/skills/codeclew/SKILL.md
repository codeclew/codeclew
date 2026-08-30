---
name: codeclew
description: Use Codeclew for bounded compiler- or syntax-backed code context, safe changes, multi-repository analysis, freshness checks, recovery, and privacy-safe diagnostics. Trigger when a task asks to inspect, explain, trace, or change supported source code through the clew CLI, or to diagnose a Codeclew run.
license: Apache-2.0
metadata:
  author: codeclew
  version: "0.2.7"
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

Before querying, turn the explicit parts of the user's request into a short
private evidence checklist. Codeclew does not infer this checklist. Search terms
only retrieve candidates and never prove a checklist item. Keep an item open
until an exact returned source window or supported fact proves it; record
conditional or unsure authority without upgrading it. For a multi-part request,
the checklist is the coverage authority for the final answer. Overall
`completeness`, truncation, or an unmatched seed term limits exhaustive claims,
but does not downgrade a specific fact proven by an exact retained source
window. Split a compound requirement into independently observable subclaims;
recursion, collection ordering, representation, failure type, and timing are
not interchangeable evidence and must not be closed as one vague item.

The first response contains at most three fact-bound decision cards with exact
one-line previews. `--source` returns a compact, evidence-bound agent card: the
exact retained source and declaration-to-window bindings for the highest-ranked
card, attested previews for alternatives, and only anchors outside that source
window. Treat its `completeness` and `truncated` fields as authoritative; the
full immutable evidence remains bound by `evidenceDigest`. Omit `--source` when
names and locations are enough.
Treat `nextActions.schema=codeclew-navigation-actions/1.0` as the structured
command contract while legacy `nextAction` strings remain compatible. In
particular, `exactSource.sameFileRequired=true` means every repeated term must have
one already established shared file; never combine terms whose files differ or
are unknown. Use the separate candidate-source and facet actions for their
stated purposes. When `decisionSource` or a selected detail reports
`sourceDelivery.status=RETURNED`, reuse that source from the current result and
do not issue the same source request again.
Select up to three other useful cards in one call by repeating `--candidate`;
each returns its retained fact and exact source window. Request relations only
when they are needed for those selected symbols:

```bash
clew nav expand \
  --session <session-id> \
  --from <context-id> \
  --candidate <candidate-id> \
  --candidate <candidate-id> \
  --source
```

Source is optional. Request a facet in a separate command only when the user
explicitly needs that relation:

```bash
clew nav expand \
  --session <session-id> \
  --from <context-id> \
  --candidate <candidate-id> \
  --facet <callers-or-callees-or-tests>
```

Never reread a returned exact source window
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

Use this command-selection loop while checklist items remain open:

1. After every exact-source result, update the checklist before issuing another
   command. Record every requested visible fact as a separate per-item ledger
   atom: mechanism or value, qualifiers or conditions, typed outcome,
   order/timing relation, source anchor, and certainty. Track evidence as
   `PROVEN` or `UNPROVEN` separately from final output as `PENDING` or `EMITTED`;
   navigation updates only the evidence state. Harvest retained-reference
   choices at the same time.
2. Stop navigation as soon as every atom is `PROVEN` or retained as an explicit
   obligation.
3. Before batching repeated `--term` values, require one already established
   repository-relative file shared by every term; `--file` scopes the entire
   batch. If a term belongs to another or an unknown file, split by file or
   establish its file first. Fetch one to three same-file declarations with one
   `--file` and `--source`:

   ```bash
   clew nav expand \
     --session <session-id> \
     --from <context-id> \
     --term <exact-identifier> \
     --term <exact-identifier> \
     --file <repository-relative-file> \
     --source
   ```

4. Otherwise follow a supported retained reference that can close an open item;
   otherwise refine only the missing identifier. Always use the newest context.
5. Request `callers`, `callees`, or `tests` only for an explicit relation
   subclaim. Never request a facet to rediscover helpers or reread source, and do
   not retry an `UNSUPPORTED` facet.
6. Do not descend below the abstraction boundary in the request. A discovered
   helper is not a new checklist item. Reserve the remaining command budget for
   explicit open items; if it cannot close one, report that item as unproven.

Automatic `nav query --follow-references` follows lexical overlap from the
automatically ranked top card. Omit it when candidate identity matters; select
the intended card first and then use explicit retained-reference follow. Never
present automatic follow as semantic resolution.

When a selected card reports `referenceChoices.status=SUPPORTED` and an open
checklist item depends on a helper, follow one to three relevant retained
references together instead of guessing or issuing separate searches. Choose
at most one path for each terminal name:

```bash
clew nav expand \
  --session <session-id> \
  --from <context-id> \
  --candidate <one-candidate-id> \
  --reference <terminal-or-full-path> \
  --reference <terminal-or-full-path> \
  --source
```

Choose references because they can close an explicit checklist item, not merely
because they are present. Prefer qualified paths and use the newest child
context that contains the selected candidate. `USER_SELECTED_RETAINED_REFERENCE`
records that the agent selected an observed syntax reference; it is not a
resolved call edge. When `targetResolution=UNRESOLVED` or
`semanticRelation=UNKNOWN`, treat returned name matches as bounded discovery
candidates. Exact source returned for such a candidate may prove facts about
that declaration, but not that the observed reference targets it. Do not
re-query source already returned. At the retained-reference step of the ordered
loop above, a supported choice that can close an open item must be followed with
the returned candidate and newest context; do not replace it with a term search.
Use exact `--term ... --file ... --source`
only when the retained-name result lacks sufficient exact source and both the
identifier and file are already established by evidence. Recurse only when the
returned helper body delegates an open checklist item; prioritize the bounded
call that closes the most open items. Preserve reported truncation and do not
infer that an omitted reference is absent.

Before answering, perform one coverage pass over the private checklist. Every
explicitly requested item must be either supported by cited returned evidence
and present in the answer, or reported as conditional/unproven with the missing
evidence named. For each supported item, preserve the concrete mechanism,
predicate or boundary, typed outcome, and before/after mutation order exposed
by the source instead of weakening them into a generic paraphrase. Do not drop
a supported item merely because another item seems more important. If the
command or output budget prevents closing an item, return the remaining item as
an obligation instead of guessing. A guard, limit, failure, or transition item
remains open until the evidence records its comparison predicate, typed outcome,
and relative order to any requested digest, publication, or mutation; a constant
or helper name alone is insufficient. For a structured multi-part response,
emit one evidence or claim entry per checklist item in the request's order; if
the output schema has no such array, use one explicit clause per item. Every
requested exact fact already visible in returned source must appear there.
Answer only after every `PROVEN` ledger atom is marked `EMITTED` in that item’s
final clause. Summarization must not discard a qualifier, ordering, or timing
atom.

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
