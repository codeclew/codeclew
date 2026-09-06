# Codeclew agent admission and documentation plan

## Document authority

- **Status:** proposed.
- **Date:** 2026-09-02.
- **Objective:** make one discovered repository contour usable verbatim by an
  agent, expose a safe diagnostic when a build adapter cannot open that contour,
  and make evidence-bound documentation a predictable follow-on workflow.
- **Scope:** the public `doctor repository`, `doctor task`, `context open`, and
  `nav query` contract; safe diagnostic output; the bundled Codeclew skill; and
  focused acceptance fixtures.

This plan is based on an observed agent run in which a task readiness check
accepted an arbitrary Maven command while the two admission commands required a
different compilation selector. The later classpath-adapter failure was reduced
to one opaque message, so the agent retried non-retryable requests and invented
environmental explanations. The raw run transcript, repository identities,
paths, source, credentials, and managed-state contents are private and are not
part of this plan or its test fixtures.

## Product outcome

An agent documenting an unfamiliar repository can:

1. run one bounded repository discovery;
2. select a `READY_FOR_TASK_DOCTOR` contour and reuse its exact ref, profile,
   language, and compilation selector without translation;
3. open `nav query` or `context open` using that same authority;
4. receive one privacy-safe, actionable build-adapter diagnostic when admission
   cannot proceed; and
5. write documentation and sequence diagrams only from retained, revision-bound
   evidence, while marking unresolved behavior as an obligation.

The product must not claim a whole-repository semantic index, infer runtime
behavior from lexical results, disclose private build configuration, or turn a
successful host-readiness check into proof that a project can be admitted.

## Invariants

1. **One compilation authority.** A `doctor task` `PASS` compilation is accepted
   by both `context open` and `nav query` for the same profile and operation.
2. **Discovery is reproducible.** Every ready contour provides enough exact
   arguments to construct its task-admission command; callers never substitute a
   shell build command for a selector.
3. **Failure is typed and bounded.** A non-retryable adapter failure identifies
   its stage and named remediation without exposing absolute paths, source,
   credentials, raw Maven settings, or arbitrary stderr.
4. **No speculative recovery.** The CLI and bundled skill do not invite retries
   with guessed JDKs, cache modes, environment variables, or external build
   state.
5. **Documentation follows evidence.** Normal Markdown is an agent-authored
   artifact, not a Codeclew output. Every compiler-backed claim retains its base
   revision and evidence binding; conditional and unknown behavior remain
   labelled as such.

## Execution order

```text
A1 shared contour authority
  -> A2 adapter preflight and safe diagnostic
    -> A3 CLI remediation contract
      -> S1 concise admission state machine in the skill
        -> S2 evidence-bound documentation workflow
          -> Q1 end-to-end agent acceptance
```

## A1 — Unify discovered and admitted compilation authority

- **Status:** [ ]
- **Goal:** eliminate disagreement between repository discovery, task readiness,
  `context open`, and `nav query` about what a valid compilation is.
- **Modify:** the shared compilation-selector parser/validator and every public
  command that accepts a compilation.
- **Steps:**
  1. Extract or identify one canonical compilation-authority validator used by
     repository discovery, task doctor, `context open`, and `nav query`.
  2. Make repository discovery emit structured ready contours containing exact
     `targetRef`, `language`, `profileId`, `compilations`, and supported
     operations.
  3. Reject a task-doctor compilation that is not valid for the selected profile
     and repository; do not return `PASS` merely because a free-form value is
     non-empty.
  4. Preserve multi-compilation behavior and existing supported public profiles.
- **Verify:**
  - a ready contour can be copied directly into task doctor and both atomic
    admission commands;
  - an arbitrary shell command is rejected consistently by all relevant entry
    points;
  - existing profile and CLI compatibility tests pass.
- **Definition of Done:** no test can construct `doctor task: PASS` followed by
  `session compilation authority is invalid` with identical authority fields.

## A2 — Add build-adapter preflight diagnostics

- **Status:** [ ]
- **Goal:** turn opaque classpath extraction failure into one safe, causal
  diagnostic.
- **Modify:** Java/Maven adapter readiness code and a public diagnostic surface,
  preferably `doctor task` with an explicit build-preflight result rather than a
  separate hidden debug mode.
- **Diagnostic contract:** report only allowlisted fields:
  - adapter stage, such as wrapper discovery, JDK compatibility, dependency
    resolution, generated-source preparation, or classpath export;
  - normalized tool identity/version when safe;
  - exit category and a bounded remediation identifier;
  - a digest of private command output, not raw output.
- **Steps:**
  1. Run the same adapter preflight used by atomic admission, not an approximate
     host-only check.
  2. Map known failures to stable codes such as `MAVEN_WRAPPER_UNAVAILABLE`,
     `JDK_VERSION_UNSUPPORTED`, `DEPENDENCY_RESOLUTION_FAILED`, and
     `CLASSPATH_EXPORT_FAILED`.
  3. Emit exactly one `nextAction` for each terminal non-retryable result.
  4. Keep detailed private evidence local and compatible with
     `clew support summarize`.
- **Verify:** fixture cases cover a successful wrapper build, a selector error,
  unavailable dependency resolution, generated-source failure, and unsupported
  JDK. Assert that public JSON contains neither paths nor secret-shaped values.
- **Definition of Done:** an agent can distinguish configuration, toolchain, and
  dependency failures without inspecting managed state or guessing environment
  variables.

## A3 — Make remediation command-safe

- **Status:** [ ]
- **Goal:** ensure that help and errors lead to one valid next command instead of
  exploratory retries.
- **Steps:**
  1. Include the selected contour identity and valid compilation choices in
     invalid-input output where privacy policy permits.
  2. For adapter failure, name the diagnostic/remediation ID and state whether a
     retry is allowed.
  3. Do not advertise cache or external-build-state combinations that violate a
     read-only live-source session.
  4. Document that host `doctor attach: PASS` proves launcher readiness only;
     project admission remains separate.
- **Verify:** CLI tests assert `retryable=false` terminal failures contain one
  actionable remediation and do not imply that changing an unrelated flag may
  solve the problem.
- **Definition of Done:** no public error requires an agent to inspect internal
  logs, set undocumented logging variables, or probe a private package registry.

## S1 — Replace the skill's long admission prose with a front-loaded state machine

- **Status:** [ ]
- **Goal:** make correct agent behavior easier than speculative recovery.
- **Modify:** all bundled Codeclew skill copies and their package/digest tests.
- **Required top-of-skill rules:**
  1. Resolve the installed launcher once.
  2. For an unfamiliar repository, run `doctor repository` once and select one
     ready contour; never reuse an aggregate report for another repository.
  3. Copy the contour fields verbatim into `nav query` or `context open`.
  4. Treat a request to "index a repository" as bounded discovery plus a request
     for documentation scope; do not invent a whole-repository index.
  5. On `INVALID_INPUT`, correct syntax once from the returned contract.
  6. On a non-retryable project-configuration failure, run only its named
     diagnostic, report the blocker, and stop the Codeclew branch.
  7. Do not read source, build the user's checkout, inspect `CODECLEW_HOME`,
     read credentials/settings, issue network probes, or use undocumented debug
     variables as a substitute for failed admission.
  8. Never describe a failed command as an opened context or a completed index.
- **Verify:** extend skill tests with positive examples for repository discovery
  and evidence-bound documentation, plus negative assertions for guessed Maven
  commands, repeated non-retryable calls, and direct managed-state inspection.
- **Definition of Done:** the first screen of the skill is sufficient to choose
  the next command for discovery, success, input error, and terminal adapter
  failure.

## S2 — Define the evidence-bound documentation workflow

- **Status:** [ ]
- **Goal:** let an agent create useful architecture notes and sequence diagrams
  without upgrading conditional evidence into facts.
- **Steps:**
  1. Require a documentation intent and two to four task-relevant terms before
     opening a broad context; ask the user for scope when they are absent.
  2. Retain `sessionId`, `contextId`, `baseRevision`, and `evidenceDigest` for
     the document work item.
  3. Retrieve exact source windows for selected participants and request caller
     or callee facets only for an explicit sequence arrow.
  4. Write Markdown in the repository's existing documentation home. If none
     exists, propose `docs/overview.md`, `docs/flows/<flow>.md`, and
     `docs/diagrams/<flow>.mmd`.
  5. Under each diagram, record symbol identities and evidence bindings. Mark
     dynamic dispatch, external integrations, and omitted branches as
     conditional or unproven.
- **Verify:** a fixture documentation case produces a revision-bound overview and
  one Mermaid sequence diagram whose arrows each map to returned evidence or an
  explicit obligation.
- **Definition of Done:** documentation contains no claim that is stronger than
  the authority of its retained Codeclew evidence.

## Q1 — End-to-end acceptance

- **Status:** [ ]
- **Scenario:** a clean supported fixture repository has one ready contour and a
  bounded documentation request.
- **Acceptance sequence:**
  1. `doctor repository` returns a ready contour.
  2. The contour opens successfully through the selected atomic admission path.
  3. An exact source query and one explicit relation retrieval return retained
     evidence.
  4. The generated documentation records the base revision and evidence
     bindings.
  5. A broken adapter fixture returns one safe typed diagnostic and the agent
     stops without retrying with invented parameters.
- **Definition of Done:** automated tests, a packaged skill-digest check, and a
  privacy scan pass; the public acceptance artifact contains no private paths,
  repository identity, source bodies, credentials, or managed-state data.

## Non-goals

- Full-repository semantic indexing without a bounded task scope.
- Automatic documentation publication or changes to a user's repository without
  explicit authorization.
- Leaking effective Maven settings, private command output, artifact URLs, or
  credentials as a debugging aid.
- Treating a successful native build as proof that Codeclew's adapter succeeded.
