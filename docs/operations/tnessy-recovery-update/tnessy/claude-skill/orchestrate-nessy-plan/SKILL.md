---
name: orchestrate-nessy-plan
description: Create small executable immutable Nessy plans with concrete code seams and test scope; review actual reports and route honest partial failures into fresh successors. Use only on explicit /orchestrate-nessy-plan delegation or status requests; never execute the delegated implementation.
argument-hint: "<workspace> <task description> | status <workspace> <run-id>"
allowed-tools: [Read, Glob, Grep, Write, Bash]
---

# Orchestrate a Nessy plan

This skill orchestrates a user-mediated shared session with Nessy. It creates the Claude-owned plan, asks the user to invoke Nessy, and validates Nessy-owned artifacts afterwards. It never starts Nessy, invokes its daemon/API/ACP mode, writes Nessy-owned artifacts, or executes the delegated implementation itself.

## Invocation

Create and dispatch a plan:

```text
/orchestrate-nessy-plan <absolute-workspace-path> <task description>
```

Check a completed or waiting run:

```text
/orchestrate-nessy-plan status <absolute-workspace-path> <run-id>
```

`workspace` must be an absolute, existing directory. `run-id` must match `^[a-z0-9][a-z0-9-]{0,63}$`.

## Create and dispatch

1. Resolve the workspace path canonically. Confirm the Nessy counterpart is installed at:

   ```text
   <workspace>/.nessy/skills/execute-claude-plan/SKILL.md
   ```

   If it is absent, stop and tell the user to install the paired package. Do not create a partial plan.

2. Read these installed shared contracts before writing a plan:

   ```text
   <workspace>/.nessy/skills/execute-claude-plan/references/plan.schema.json
   <workspace>/.nessy/skills/execute-claude-plan/references/report.schema.json
   ```

3. Investigate only enough to produce a bounded, executable plan. Create a new lower-case run ID, then create:

   ```text
   <workspace>/.nessy/a2a-runs/<run-id>/plan.json
   ```

   The plan must conform to `plan.schema.json` and include an explicit target workspace, ordered steps with unique IDs, step and top-level acceptance criteria, explicit `commands` and `verificationCommands`, limits, and constraints. Default constraints must prohibit remote publication, deployment, CI/CD or Dockerfile changes, secret access, destructive cleanup, network actions, and MCP use.

   Before writing, make the plan executable rather than an architectural wish list:

   - Prefer one reviewable invariant per run and a few ordered implementation
     steps. Split source readers, identity, lifecycle cleanup and native integration
     when they can be tested independently; do not require the entire end-to-end
     system to pass in every local slice.
   - Name verified files/functions, existing fixture helpers, the first regression,
     the intended data shape and the minimal caller changes. Read current code:
     earlier audit claims may be wrong or already fixed. State concrete corrections.
   - State what local tests prove and assign native/public/transaction qualification
     to a named final gate when needed. Helper tests cannot replace that gate.
   - Separate step iteration commands from mandatory final verification. Include
     the commands needed for normal edit-test iterations; verify command syntax,
     supported flags, test target creation order and offline fixture availability.
   - Avoid unrelated validation frameworks or broad cleanup. Preserve useful prior
     edits, name non-goals and require unique owned fixture roots.
   - Validate the complete plan against the installed schema before exclusive
     creation. Do not claim a lightweight lifecycle validator checks every schema
     or semantic acceptance rule.
   - For a dependency chain, require accepted `completed` predecessors, not merely
     validator exit zero. A failed terminal run cannot later become completed.
     Create fresh downstream plans with corrected predecessor IDs when replacing
     a failed predecessor; keep all old plans and reports immutable.

4. Once `plan.json` exists, do not edit it. Do not create `status.json`, `events.jsonl`, or `report.json`: those are owned by Nessy.

5. Tell the user exactly what to do next:

   ```text
   Start Nessy from <workspace>, then run:
   /execute-claude-plan <run-id>
   ```

   State the expected report path. Do not say the delegated work is complete.

## Check a run

1. Resolve the same workspace and safe run directory at:

   ```text
   <workspace>/.nessy/a2a-runs/<run-id>/
   ```

   Reject paths outside that root. Read `plan.json`, `status.json`, `events.jsonl`, and `report.json`.

2. If `report.json` is absent, report only the observable state from `status.json`/`events.jsonl`; the run is not complete. Never infer success from an assistant message, a partial log, or a `running` snapshot.

3. If `report.json` exists, run the installed read-only validator:

   ```text
   python3 <workspace>/.nessy/skills/execute-claude-plan/scripts/validate_run.py <run-id>
   ```

   Do not run plan commands or modify the target repository. If the validator is unavailable or fails, report that the terminal artifact cannot be accepted until the protocol issue is resolved.

4. Compare `report.json` against `plan.json`:

   - `completed`: accept only if validator passes, the report conforms to the
     installed schema, ordered step IDs match and required steps are completed,
     step/top-level unmet lists are empty, `blockingQuestion` is null, and the exact
     ordered declared verification list has matching actual execution events with
     exit code `0`. Inspect concrete test evidence for the requested production path;
     skipped tests, zero matching tests or a helper-only substitute do not count;
   - `failed`: clearly report failure reason and affected steps; do not retry automatically;
   - `input-required`: quote `blockingQuestion` and give a concise recommendation. Do not silently authorize the action or alter the plan;
   - `canceled`: report it as canceled without treating it as successful.

5. Cross-check claims against events and changed files. An entry described as
   "not executed" has no observed exit code, even if the report assigns one.
   Do not infer success from `VALID`, a completed preparation step, or narrative
   confidence. Distinguish missing evidence, structural protocol errors and actual
   implementation defects. Do not rerun implementation commands in status mode.

6. For an authorized retry, preserve usable partial edits and replace only the
   unmet scope with smaller concrete slices. A vague complexity-based stop calls
   for narrower seams and test ownership, not relaxed safety or fictional approval.
   A concrete permission or dependency blocker must be surfaced to the user.
   Compare elapsed work with declared limits without guessing hidden constraints.

7. A retry always uses a new run ID and a new immutable plan with `retryOfRunId`;
   never overwrite a terminal run, automatically restart Nessy, or mutate old
   reports to make downstream prerequisites pass.
