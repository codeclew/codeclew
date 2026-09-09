---
name: codeclew
description: Use Codeclew for bounded compiler- or syntax-backed code context, safe changes, multi-repository analysis, freshness checks, recovery, and privacy-safe diagnostics. Trigger when a task asks to inspect, explain, trace, or change supported source code through the clew CLI, or to diagnose a Codeclew run. Development of Codeclew itself follows its repository-native contributor workflow.
license: Apache-2.0
metadata:
  author: codeclew
  version: "0.7.0"
  repository: https://github.com/codeclew/codeclew-skill
---

# Codeclew

Use the installed Codeclew product for bounded source evidence and qualified
managed changes. Developing Codeclew itself uses that checkout's native tools.
Preserve the user's scope, source selection, authority and publication approval.

## Start with the requested result

Read this entrypoint once. Do not load all references or discover commands that
are already specified here. Keep a short private evidence checklist containing
only the requested facts. Stop when each fact is supported or explicitly unknown.
Keep predicates, typed outcomes, qualifiers and mutation ordering in the answer;
include preserved as well as changed cases of a requested transformation.
A helper is relevant only when it answers an open item. No separate ledger file,
plan file, diagram or test run is needed for a source explanation unless the
request or an unresolved behavioral question needs that artifact.

Batch independent searches in one tool invocation. Once paths are known, read
needed implementation and test ranges together. Do not split already-known
reads across model turns or collect evidence that does not answer an open item.
For source analysis, identifying tests does not require executing them. A supplied
test command is guidance, not an execution request; run it only when requested
or when source leaves a behavioral question unresolved. Changes still require
the applicable validation and regression tests.

Use the exact installed launcher supplied by the caller. Otherwise resolve
`clew` from PATH once. Do not use source/capsule binaries, `CODECLEW_RUNTIME_SEED`,
or direct changes inside `CODECLEW_HOME`. An absent or rejected installed
launcher stops the managed workflow. Do not build a replacement.

When the repository, exact ref, language, profile and compilation are supplied,
use them directly. Otherwise run `clew doctor repository --repo <absolute-repo>`
once and select a matching `READY_FOR_TASK_DOCTOR` contour. CLI language values
are lowercase (`java`, `kotlin`, `rust`, `python`, `typescript`, `javascript`);
discovery's uppercase language labels are not CLI arguments. Do not repeat
Git cleanliness, JDK/version checks, capabilities, doctor task or build-file
reads already covered by atomic admission. Discovery is not admission.

## Read a named declaration

For an identifier explicitly present in the request, start with one command:

```bash
clew nav query --repo <absolute-repo> --target-ref <exact-ref> \
  --language <language> --profile <profile-id> --compilation <compilation> \
  --term <task-identifier> --decision-identifier <task-identifier> --source
```

Use only relevant selected compilations; do not add test compilations merely to
search for test files. Multiple `--term` values must be discriminating request
identifiers. Do not invent the decision identifier from a ranked result. Omit
`--decision-identifier` and `--source` for initial discovery without an exact
identifier. Retain session/context IDs and the newest child context.

The returned admission must be PASS, `runtimeMode=RELEASE`, with
`codeclew-agent-contract/1.0`, `launcherAuthority=INSTALLED_RELEASE` and
`sourceFallbackAllowed=false`. Preserve typed blockers and next actions. A
successful exit does not override a read-only or ACTION_REQUIRED result.

Read returned source before another command. Reuse `sourceDelivery.status=RETURNED`;
do not reread its exact window with native tools. Missing helpers can be selected
in one call from returned cards/references:

```bash
clew nav expand --session <id> --from <context-id> \
  --candidate <returned-candidate-id> --reference <returned-reference> --source
```

Repeat `--reference` for up to three useful references from that candidate.
Alternatively select up to three returned `--candidate` IDs with `--source`.
When an exact identifier and its file are already established, fetch one to
three same-file declarations together:

```bash
clew nav expand --session <id> --from <context-id> \
  --term <exact-identifier> --file <repository-relative-file> --source
```

Repeat `--term` only for identifiers established in the same file. Never request
a facet to rediscover helpers; request callers, callees or tests only for a
needed relation. Unsupported facets are not empty relations. Returned lexical
references are not resolved calls; source can prove a declaration's behavior
without proving that an unresolved reference targets it.

A single explicit identifier with `--source` uses declaration-name lookup.
`completeness.queryScope` states that scope; complete name lookup does not mean
complete callers, references or tests. Overloads still require an exact identity.
A SUPPORTED decision selects the exact requested identity, not a correct answer.
On ABSTAIN, do not follow the first ranked candidate: use the returned structured
`codeclew-navigation-actions/1.0` action only if its precondition is satisfied.
Never replay `repeatSameRequest=false`; STOP_UNRESOLVED leaves the affected item
unproven. Preserve query truncation, omitted scopes and conditional authority.
For decision refinements, relation facets, delta reconstruction or branch
coverage rules, read the relevant part of
[the navigation contract](references/workflow-details.md#navigate-to-relevant-code).

Use ordinary bounded tools only for evidence not already returned and within the
caller's authorized workflow. Never label their results as managed evidence.
A failed managed admission remains a failure even if native work is permitted.
Do not run native tests just to restate visible source behavior. When execution
is needed, run the focused test once and capture its result in the same call.

## Change code

The exact profile must support mutation. Open the session and context atomically:

```bash
clew context open --repo <absolute-repo> --target-ref <exact-ref> \
  --language <language> --profile <profile-id> --compilation <compilation> \
  --operation mutation --intent <task-intent> --term <identifier>
```

Before planning, read only
[Prepare a change](references/workflow-details.md#prepare-a-change).
Use a closed immutable plan, candidate validation, freshness checks and managed
publication. `change prepare` waits for the first actionable state; do not poll
PREPARING or restart it. Inspect the exact diff, checks and obligations before
publication. Existing explicit user authorization remains effective. Conditional
publication requires the exact prepared authority digest, `--allow-conditional`
and every obligation. Never reset, clean or replay user work to obtain freshness.

## Select specialized work only when needed

- Durable service documentation: [service documentation](references/service-documentation.md).
  Its `clew docs` path performs admission internally; do not add the generic loop.
- Saved edits, committed analysis, Java/Maven settings or compatibility profiles:
  [Resolve and admit](references/workflow-details.md#resolve-and-admit).
  Caller-local `codeclew.yaml` may select private Maven settings; never read,
  publish or replace credentials. Current broad Java/Kotlin analysis profiles
  do not acquire mutation permission.
- Endpoint inventory: [Catalogue Spring computation roots](references/workflow-details.md#catalogue-spring-computation-roots).
- Diagrams or saved-change reports: [Explain with source-bound diagrams](references/workflow-details.md#explain-with-source-bound-diagrams)
  and [Explain current saved edits](references/workflow-details.md#explain-current-saved-edits).
- Multi-repository work: [Work across repositories](references/workflow-details.md#work-across-repositories).

## Finish and recover

Cite the source supporting each requested conclusion and identify unknowns.
Syntax, compiler relations and executed tests are different authorities.
Use `--help` only for missing syntax or a command rejection. On a typed readiness
failure run only its named diagnostic once; report nextAction and stop that path.
Retry a crashed worker once only if the typed error is retryable. Recovery,
freshness and GC details are in
[Recover and report safely](references/workflow-details.md#recover-and-report-safely)
and [Prepare a change](references/workflow-details.md#prepare-a-change).

Keep raw outputs, source, paths, arguments and managed state private. Use
`clew support summarize --input <private-result.json>` for external diagnostics;
share only a returned SAFE_TO_SHARE summary and separately generated capabilities
and doctor JSON. A rejected summary remains private. Retain analysis sessions
for follow-up evidence; finishing an answer does not require help, status or
cleanup calls. When the evidence is no longer needed, use
`clew session close --session <id>` and supported lifecycle commands only.
