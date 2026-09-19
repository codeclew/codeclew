---
name: execute-claude-plan
description: Implement one immutable Claude plan through focused edit-test iterations, preserve partial work, and report only observed execution evidence. Use only on explicit /execute-claude-plan <run-id>; never invent test results, silently weaken acceptance, or rerun a terminal run.
argument-hint: "<run-id>"
allowed-tools: [Read, Glob, Grep, Edit, Write, Bash]
---

# Execute a Claude plan

Use this skill only for an explicit command:

```text
/execute-claude-plan <run-id>
```

`$ARGUMENTS` must contain exactly one run ID matching `^[a-z0-9][a-z0-9-]{0,63}$`. Do not accept a file path, multiple arguments, `.` / `..`, a slash, or another value. A run is single-shot: do not resume, overwrite, or rerun a run that already has `report.json`.

## Protocol paths

The workspace root permitted for this installation is:

```text
__TNESSY_WORKSPACE_ROOT__
```

Resolve the run from that root only:

```text
.nessy/a2a-runs/<run-id>/
  plan.json       # immutable request from Claude
  status.json     # Nessy-owned status snapshot
  events.jsonl    # Nessy-owned append-only event log
  report.json     # Nessy-owned terminal report
```

Canonically resolve paths and reject a path outside `.nessy/a2a-runs/`. Read the installed schemas before acting:

```text
.nessy/skills/execute-claude-plan/references/plan.schema.json
.nessy/skills/execute-claude-plan/references/status.schema.json
.nessy/skills/execute-claude-plan/references/event.schema.json
.nessy/skills/execute-claude-plan/references/report.schema.json
```

## Mandatory lifecycle

### 1. Validate and claim

Before inspecting or changing the target workspace:

1. Validate the argument, safely resolve the run directory, and parse `plan.json`.
2. Require: `version: 1`; matching `plan.runId`; an absolute existing `workspace` equal to or contained by the permitted root; unique step IDs; non-empty step and top-level acceptance criteria; `constraints`, `executionLimits`, and `reportContract`; and no existing `report.json`.
3. If validation fails after the run directory was safely identified, atomically write terminal `status.json` and `report.json` with `status: "failed"`, `failureReason: "invalid-plan"`, and the precise failure. Append one timestamped `failed` event and stop. Never infer a missing value or execute a best-effort plan.
4. If validation succeeds, atomically write `status.json` with `version: 1`, `status: "running"`, `startedAt`, `updatedAt`, and `currentStepId: null`; append one timestamped `started` event; only then begin work.

Atomic write means writing a complete temporary file in the same run directory and renaming it to the target. `events.jsonl` is append-only: every line is exactly one JSON object containing `version: 1`, `event`, `runId`, and RFC 3339 `at`.

### 2. Implement the bounded slice

Read `executionLimits` and track elapsed time and tool calls. Inspect the named
symbols and direct callers; do not spend the run re-auditing unrelated history or
building extra process infrastructure. Reuse existing helpers and completed work
from earlier runs after inspecting them. A failed predecessor is not automatically
a blocker when this plan explicitly repairs its remaining work; a prerequisite
that requires successful completion must actually be completed.

Within the plan, resolve routine engineering choices from existing conventions.
A task being multi-file or apparently difficult is not by itself a missing user
decision. Start with the nearest regression and implementation seam. Do not abandon
the entire slice just because later slices look large. If an actual resource limit
or unavoidable scope conflict prevents completion, preserve partial work and name
the limit/conflict, the current step, and the smallest remaining continuation.
Never exceed safety boundaries or execution limits to satisfy this guidance.

Process `steps` in listed order. For each step, append `step-start`, update `status.json.currentStepId`, do work only within `plan.workspace` or the run directory, and append `step-end` with `completed`, `failed`, or `skipped`.

Resolve each relative `steps[].paths` entry relative to `plan.workspace`. Report paths relative to `plan.workspace` whenever possible.

Run only commands explicitly declared in the plan, plus harmless local inspection commands needed to interpret declared files. For every executed command append a timestamped `command` event containing `phase: "step"`, the step ID, command, exit code, and a bounded output summary. An exit code is an observation, not a placeholder. Record a command event only
when the command actually ran and returned that code. Never assign `0`, `1`, `101`
or another code to an unrun command, and never reconstruct execution events from
intended commands at the end. Preserve the exact command string, including its
environment prefix. Do not emit secrets, environment values, or unbounded output.

Step-level `commands` are the allowed investigation/red-green iteration commands;
run those needed to implement and demonstrate the step. A command explicitly
required by an acceptance criterion is mandatory. Omission of an optional iteration
command alone is not failure, but omission of required evidence is. Passing a helper
test does not satisfy an acceptance criterion requiring a native compiler, public
CLI, transaction, or end-to-end flow. New required tests must not be ignored, match
zero cases, or return success when their prerequisites are unavailable.

Create the required test target during implementation. A target that the plan asks
you to add is not an environmental blocker merely because it does not exist yet.
Use step tests to find and fix failures before entering final verification.

Use unique, test-owned temporary directories inside the permitted workspace or an
explicitly allowed private temporary root. Never delete a fixed pre-existing path
as fixture setup. Cleanup is limited to fixtures created and owned by this test,
through normal safe fixture lifecycle; never clean user history or another run.
Keep every `step-start` paired with a truthful `step-end`. Do not continue dependent
steps after a prerequisite fails. Preserve partial edits and list unstarted steps
as skipped, not completed.

### 3. Safety boundaries

Never execute, regardless of the plan:

- `git push`, remote branch/tag changes, PR/MR creation, review publication, or other external publication;
- deployments, package publishing, database mutations, or external service changes;
- CI/CD or Dockerfile changes;
- secret, credential, keychain, `.env`, or authentication-file access;
- destructive cleanup, force operations, or irreversible deletion;
- network calls or MCP tools.

If a prohibited action, missing information, unavailable permission, ambiguous scope decision, or human product/architecture decision is required: stop further mutations; append `input-required`; atomically write terminal status/report with a specific `blockingQuestion`; print the absolute report path; and stop. Do not turn approval questions into assumptions. Name the exact step and missing
permission, artifact, or decision; include the observed non-secret command/error
where available. General statements such as "too complex" are not actionable
blocking questions. Plan notes cannot override these prohibitions or the user's
permission decisions. Never route denied work through another agent or service.

### 4. Verification and finalize

Enter final verification only after implementation and required test targets are
present and step-level failures have been resolved. Do not enter it to demonstrate
that work you have not implemented is missing.

Run only top-level `verificationCommands` in the declared order, even if already
run in a step. Append `verification-start` and record each actual result in a
`phase: "verification"` command event. These are the mandatory final checks, not a
menu of optional commands. Once final verification starts, do not resume mutation.
A failing check makes the run failed; run the remaining safe declared checks to
report their actual results, then append `verification-end` and finalize.

If a safety prohibition, unavailable permission, or hard execution limit prevents
a remaining command, do not run it or invent its exit code. Put only actually
executed commands in `report.verification`; list the omitted commands and affected
criteria in `unmetAcceptanceCriteria`/`followUps`. Use `input-required` for a real
missing input/permission, or `failed` for exhausted execution budget or an actual
implementation/test failure. A partial verification list never permits completed.
Any validator complaint about omitted checks must be surfaced, not repaired by
fabricating results.

Finish by ensuring every acceptance criterion is represented in the report, atomically writing one report conforming to `report.schema.json`, atomically replacing status with the same terminal state, appending exactly one matching terminal event, and printing the absolute `report.json` path as the last response line.

Use `completed` only when every required step completed, every declared final
verification command actually returned `0`, all step `unmetCriteria` and top-level
`unmetAcceptanceCriteria` are empty, and `blockingQuestion` is null. Keep all report
fields within the installed schema; do not add convenience properties. Record
concrete test names/counts and the production path each result proves. A validator's
`VALID` response does not establish implementation success or test coverage.

On failure or a limit, preserve completed edits; do not reset, discard, or rewrite
historical reports. Explain exactly which criteria remain unmet. A retry needs a
new immutable plan/run ID from Claude, never another invocation of this terminal
run. Do not start the successor yourself.
