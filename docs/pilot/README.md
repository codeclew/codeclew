# Codeclew Kotlin 2.4 pilot

This runbook is the operator contract for a limited team pilot. It is not a
general-availability claim.

## Supported contour

- macOS or Linux, JDK 21, and the pinned Rust toolchain;
- Kotlin 2.4.10 with Gradle and the project wrapper;
- `PROJECT_NATIVE` and one exact compilation;
- a clean, checked-out feature branch whose ref and `HEAD` identify the same
  commit.

Maven, Kotlin 2.1/2.3, Android/KMP, multiple compilations, `EXTERNAL`, and LLM
plan quality are outside this pilot. Do not expand the contour to work around a
failure.

## Operator flow

1. Create and check out a feature branch. Commit or remove every local change;
   Codeclew publication must never target a shared or dirty branch.
2. Run `./clew change open --repo ... --target-ref ... --compilation ...
   --intent ... --term ...`. Keep the returned `sessionId` and `contextId`.
3. Review the bounded context. If required evidence is missing, use the
   advanced `context expand` command and create a new plan against that context.
4. Save the plan outside tracked source and run `./clew change prepare ...`.
5. Poll `./clew change status --run ...`. Review the bounded diff, exact changed
   files, validation evidence, certainty, and every qualified obligation.
6. Publish with `./clew change publish ...`. Conditional publication requires
   the exact prepared digest and every approval ID; it remains `UNSURE`.
7. Run the native project tests, then `session close` and `session gc`.

`change prepare` must not mutate the checked-out source. Repeating prepare or
publish must attach to the same authority and final commit. A committed
candidate is never reset automatically. Use `change recover` only for
`PUBLISHING` or `WORKTREE_RECOVERY_REQUIRED`; otherwise stop and preserve the
typed error.

## Evidence handling

Copy [case-template.json](case-template.json) to a private directory outside the
repository. Record only a local case ID, project class, outcome, bounded stage
durations, runtime mode, and typed error code. Never record repository paths,
names, intent, source, diff, command output, user identity, or credentials.

Filled cases must not exist anywhere under the repository. In particular,
`docs/pilot/results/` is both ignored and rejected by the privacy scanner, even
if force-added to Git. The template is not evidence.

## Stop conditions

Stop the pilot immediately and retain the typed state without ad-hoc cleanup if:

- source or target ref changes before explicit publish;
- a retry creates a different run or final commit for the same authority;
- a failure has no stable error code or requires manual state deletion;
- recovery remains unresolved;
- private data appears in stdout, evidence, Git, or the case record.

Do not turn `UNSURE` into `VERIFIED` manually. Satisfying an obligation requires
a new context, plan, and run backed by the additional deterministic evidence.

## 20-case release decision

Run the controlled `scripts/pilot.py` qualification first, then collect 20 real
feature-branch cases within the supported contour. A signed prebuilt RELEASE
distribution may be planned only when all of these are true:

- at least 19/20 cases (95%) reach a prepared terminal state without manual
  cache or state cleanup;
- 20/20 preserve source and target ref before explicit publish;
- 20/20 idempotent retries retain the same run and publication authority;
- every failure and recovery outcome is typed and actionable;
- no case leaks private data or commits filled evidence.

If a threshold fails, select the most frequent typed blocker as the next
milestone. Do not add another language, build system, or runtime contour.

Assurance cadence is intentionally split: pull requests run targeted checks and
one usability smoke; manual/weekly qualification runs the strict warm audit and
three-case pilot on Linux and macOS; a future release gate may add signing and
portable installation only after this 20-case decision.
