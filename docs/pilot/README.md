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

[case-template.json](case-template.json) defines the closed case schema; do not
fill it by assertion; its placeholders are deliberately ineligible. Create one
private pilot attestation key before the first case and retain it for all 20:

```bash
python3 -I -S scripts/pilot_case_record.py keygen \
  --output /private/absolute/pilot-attestation.key
```

Save the `change open`, both `change prepare`, terminal
`change status`, and both `change publish` JSON results as physical `0600`
files in a private directory outside every repository. The second prepare and
publish are the required idempotency probes.

While the run is READY and before publish, capture the source authority:

```bash
python3 -I -S scripts/pilot_case_record.py snapshot \
  --repo /absolute/product/repo \
  --opened /private/absolute/opened.json \
  --terminal /private/absolute/terminal.json \
  --output /private/absolute/prepublish.json
```

After publish (or a typed non-published terminal outcome), derive the case with
`pilot_case_record.py record`. Pass the six managed artifacts plus the snapshot,
stage durations, repository/ref, local case ID, and a new private output path.
Omit both publish artifacts only for a non-published outcome. The recorder binds
session, run, Git ref, prepublish snapshot, final commit, validation, and retries;
it also requires `--attestation-key` and binds the record to the raw-artifact
digest. Stdout contains only a digest receipt. Use `--manual-cleanup-used` if any
cache or state was changed outside the managed commands.

Never record repository paths, names, intent, source, diff, command output, user
identity, or credentials in a case record. A detected path, email, or credential
marks `privateDataLeak`; it can never pass the release gate.

Filled cases must not exist anywhere under the repository. In particular,
`docs/pilot/results/` is both ignored and rejected by the privacy scanner, even
if force-added to Git. The template and digest-only recorder receipts are not
case evidence.

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
- at least 19/20 reach explicit `PUBLISHED` or `PUBLISHED_CONDITIONAL`;
- 20/20 preserve source and target ref before explicit publish;
- 20/20 idempotent retries retain the same run and publication authority;
- every failure and recovery outcome is typed and actionable;
- no case leaks private data or commits filled evidence.

If a threshold fails, select the most frequent typed blocker as the next
milestone. Do not add another language, build system, or runtime contour.

Assemble the 20 records into one private `0600` file outside the repository:

```json
{"cases":[{"...":"20 closed case records"}],"schema":"codeclew-pilot-case-set/1.0"}
```

Evaluate it without copying evidence into Git:

```bash
python3 -I -S scripts/pilot_release_gate.py \
  --cases /private/absolute/cases.json \
  --attestation-key /private/absolute/pilot-attestation.key \
  --receipt /private/absolute/release-decision.json
```

The command verifies all 20 HMAC attestations and emits only aggregate counts, a canonical `caseSetDigest`, and
`SIGNED_RELEASE_ELIGIBLE` or `NOT_ELIGIBLE`; it never emits case IDs. The case
set and decision receipt are rejected by the repository privacy scanner.
A signed release requires 20/20 RELEASE-mode cases and must bind the exact PASS
receipt, its digest, and the case-set digest.

Assurance cadence is intentionally split: pull requests run targeted checks and
one usability smoke; manual/weekly qualification runs the strict warm audit and
three-case pilot on Linux and macOS; a future release gate may add signing and
portable installation only after this 20-case decision.
