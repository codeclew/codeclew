# Auto-Generated Process Descriptions in codeclew — Design

> **Status:** Approved design (2026-09-23).
> **Scope:** Hybrid process documentation — render an evidence-backed process flow by default, add declarative state diagrams, emit PlantUML, and bind output to evidence.

## Goal

Make codeclew **auto-generate** process descriptions that match a validated local architecture mockup:

- A **state diagram** (STATE FLOW) derived from a **declarative state schema**, each transition
  bound to evidence (operation boundary symbol).
- A **detailed process flow** per operation, auto-built from **FLOW_STEP evidence +
  source deepening** (branches, calls, DB writes, returns, catch), each step bound to evidence.
- Output as **PlantUML (.puml) + pre-rendered SVG** (for autodoc/MDX).
- **Hybrid authoring:** auto by default; a human-authored proposal takes priority.

## Current State (context)

- codeclew today does **not** auto-generate narrative — it accepts human-authored proposals
  (summary + steps) that bind to evidence, and renders a client-side sequence SVG.
- A saved scenario (`scenarios/task-lifecycle-management.yaml`) has one root method
  (`TaskService#changeTaskStatus`); its `process-overview` section is summary-only, the body
  operation carries the sequence.
- Retained `FLOW_STEP` evidence for a method can be **shallow** (a single `BOUNDARY` step), so
  full detail (branches, DB writes) must be **deepened from the transformed source**
  (`TRANSFORMED_SOURCE`), as demonstrated in the mockup.

## Design Decisions (validated with user)

1. **Generation stage:** Hybrid.
   - Render auto-shows the evidence-backed flow as a detailed process flow.
   - State diagram comes from a separate declarative file (statuses/transitions), bound to evidence.
2. **Output format:** PlantUML — codeclew emits `.puml` and pre-renders to SVG (for autodoc/MDX).
3. **State source:** a new declarative schema file; clew validates each transition against evidence
   and renders the state diagram.
4. **Authoring:** auto by default; a human-authored proposal overrides/augments the auto flow.
5. **Flow source:** FLOW_STEP as the base, deepened from source where FLOW_STEP is shallow.

## Architecture

Three components:

### Component 1 — Process-flow renderer

- At render time, for a process/scenario operation, build the step sequence from **FLOW_STEP**
  evidence, **deepening from transformed source** where FLOW_STEP is shallow (if/else branches,
  service calls, DB writes, returns, catch).
- Emit a PlantUML activity diagram (`@startuml ... @enduml`), pre-render to SVG.
- Each step carries **source/dependency evidence refs** (existing claim-evidence model).
- **Auto by default:** rendered whenever no human-authored sequence exists. If a proposal authored
  steps, those take priority (auto is not emitted).

### Component 2 — Declarative state schema

- New file `scenarios/<id>-states.yaml`, schema `codeclew-documentation-process-states/1.0`.
- Declares `states` (name + description), `initial`, `final`, and `transitions`
  (`from`, `to`, `label`, `operation`, `evidence[]`).
- clew **validates** each transition: the referenced operation/boundary must exist in retained
  evidence; unresolved transitions are reported as gaps/limitations (not silently dropped).
- Emit a PlantUML state diagram (`stateDiagram`-style), pre-render to SVG. Transitions labeled
  with the operation.

### Component 3 — PlantUML generator + SVG

- A module that renders the flow (Component 1) and state (Component 2) into `.puml` and, when a
  PlantUML binary is available, pre-renders to SVG.
- For autodoc/MDX: commit `.puml` sources + generated SVGs (autodoc-3 pattern: `docs/diagrams/*.puml`
  → `static/img/*.svg`), referenced by absolute site path.
- Style: `!theme plain` + `!pragma layout smetana` (as validated in the mockup).

## Declarative state schema (reference, from the validated example)

```yaml
schema: codeclew-documentation-process-states/1.0
id: task-lifecycle-management
title: Task lifecycle management
initial: WAIT
final: [FINISHED, ERROR]
states:
  WAIT:          "waiting"
  PROCESSING:    "processing"
  WAIT_DECISION: "waiting for a decision"
  FINISHED:      "finished"
  ERROR:         "error"
transitions:
  - from: PROCESSING
    to: WAIT_DECISION
    label: "status → WAIT_DECISION"
    operation: changeTaskStatus
    evidence: [task-manager:symbol:65a49248775beae815197acc]
  - from: ERROR
    to: WAIT
    label: "restart (closeErrorsAndSetWaitStatus)"
    operation: restartTask
    evidence: [task-manager:symbol:...]
```

## Output format (reference, from the validated example)

State diagram (generated from the schema):
```
[*] --> WAIT
WAIT --> PROCESSING : worker claimed the task [takeTaskForProcessing]
PROCESSING --> WAIT_DECISION : status → WAIT_DECISION [changeTaskStatus]
ERROR --> WAIT : restart (closeErrorsAndSetWaitStatus) [restartTask]
```

Detailed process flow (activity diagram generated from FLOW_STEP + source):
```
start
Entry: changeTaskStatus(taskId, ChangeTaskStatusRequest);
if (anyTask) then (yes)
  :taskHandlingService.changeAnyTaskStatus(...);
else (no)
  :taskHandlingService.changeTaskStatus(...);
endif
if (FINISHED == taskInstance.getTaskStatus()) then (yes)
  :taskStatusesMetrics.incFinished(taskType);
endif
:return createResponse(...);
:catch (Exception e) → return createResponse(..., "500", ...);
stop
```

## Implementation Slices

Large feature — decomposed into two independently shippable slices:

### Slice A — Process-flow renderer
- Build a flow sequence from FLOW_STEP, deepened from transformed source.
- Emit PlantUML activity → SVG.
- Bind each step to source/dependency evidence.
- Manual proposal overrides auto (auto emitted only when no authored steps).
- Tests: unit (flow→puml, source deepening, evidence binding), integration (task-manager).

### Slice B — Declarative state diagram
- New state schema (`codeclew-documentation-process-states/1.0`) + CLI/file load.
- Validate transitions against evidence; report unresolved as gaps.
- Emit PlantUML state → SVG.
- Tests: schema validation, evidence binding, render output.

## Constraints / Limitations

- Source deepening requires parsing the method body (branches/calls) — a new capability; current
  flow analysis is shallow (see mockup notes: `LAMBDA_EXECUTION_NOT_EXPANDED`,
  `SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW`, `EXCEPTION_FLOW_REQUIRES_SOURCE_REVIEW`).
- State machine is not auto-derived from code; it comes from the declarative file (validated
  against evidence), per the approved decision.
- PlantUML SVG pre-render requires a PlantUML binary (locally installed, e.g. `~/tools/plantuml.jar`);
  when absent, codeclew still emits `.puml` and defers SVG to the docs build.
- Evidence binding reuses the existing claim-evidence model (source refs must resolve).

## Open items (tracked for implementation)

- Exact CLI/API surface for the state schema file (load/validate/render commands).
- Rendering integration point in `render.rs` for the new auto flow + state diagram.
- How the auto flow coexists with the existing proposal/sequence path at the data-model level
  (a marker that steps are "auto" vs "authored").
