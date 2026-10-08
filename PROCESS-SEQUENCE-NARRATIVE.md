# Observation: authoring internal processes as sequence operations produces a technical dump

Historical issue report translated from the original Russian note. Counts,
examples and alternatives below retain the original report's context; they do
not establish runtime execution or impose new implementation requirements.

## Observation

To include an internal service process (an orchestrator such as
`AggregateDealService.process`) as a separate documented operation, it is
accepted as a **sequence operation**: an entrypoint with participants/events.
Render requires **every** structural branch in the source (`required_sequence_flows`
and `sequence_event_kinds` in `render.rs`) to be covered by a sequence event,
otherwise it rejects the operation:

```text
sequence omits a source-backed condition or return: motor-deal-service:flow:<id>
```

To pass this validation, one event per flow was generated (IF/TRY/DEFERRED/
RETURN/THROW plus optional CALL), with automatic prose. English translation of
the originally emitted Russian example:

```text
The source contains fetchAllContactsAndBuildMap(subjectRefs, integrationId) (CALL).
The record captures lexical structure.
```

## Problem

1. **The reader gains little value.** These lines describe the authoring
   mechanism's technical trace rather than the business process. They get the
   operation through validation but fill the document with dozens of lines:
   the reported `proc_process` had up to 106 flow events.
2. **CALL events are optional** (`sequence_event_kinds("CALL") = []`), but the
   current approach still generated them, creating most of the noise.
3. **Mandatory branches** (IF/TRY/DEFERRED/RETURN/THROW) need events, but their
   source lexemes do not explain the business decision.

## Root of the problem

The reported mechanism of accepting a process as a sequence operation reflects
**lexical code structure**, rather than process meaning. Writing a readable
process explanation in an Overview/Purpose section gave a clean result, but
then the process did not appear as a separate operation or diagram.

## Alternatives originally proposed

### A. Separate mandatory branches from decorative events

Allow an accepted operation to include a **minimal event set**: request,
external calls, response and mandatory IF/TRY/DEFERRED/RETURN/THROW. Do not
require CALL coverage or display optional technical calls. In the reported
approach, CALL events were both generated and rendered.

### B. Give branches meaningful text

Allow the author to state the branch's **business meaning**, rather than repeat
a source lexeme. When render requires an event for each flow, its text should
explain the supported condition or decision, rather than say “the source
contains ...”.

### C. Represent a process as prose rather than a sequence

For many internal processes, the appropriate form may be an accepted **prose
explanation** in a section such as `section-process-*`, rather than a sequence
diagram. Custom `section-process-*` operations were rejected as duplicate or
out-of-scope because they were absent from the fixed `sections::REQUIRED` list.

### D. Improve diagnostics

The old error `sequence omits a source-backed condition or return: <flow>` did
not explain which event to add. Report the missing flow and its required event
kind (`alt`, `opt`, etc.).

## Original recommendation

For readable internal-process documentation with the reported engine:

- Explain the process **as prose** in existing sections, without a sequence
  operation: clean output, but no separate operation.
- Or accept a sequence operation, **remove optional CALL events**, and give
  mandatory branches meaningful text. This required manual work and could
  still remain noisy.

The original recommendation was to implement A, improve branch text (B), and
improve diagnostics (D). Useful business calls must still remain available;
these alternatives do not establish runtime chronology or justify dropping
mandatory source-backed branch coverage.

## Relevant code

- `crates/clew/src/documentation/render.rs`: `required_sequence_flows`,
  `sequence_event_kinds`, `sequence_step_requires_endpoints`, and missing-flow
  validation.
- `crates/clew/src/documentation/sections.rs`: the fixed REQUIRED section list.
- `crates/clew/src/documentation/notes.rs`: expected narrative IDs.
