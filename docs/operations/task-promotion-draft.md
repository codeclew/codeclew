# Reproduce the TaskStateTransitions.promote draft

This recipe prepares one source-local process explanation for the public Java
fixture's `TaskStateTransitions.promote(Task, Instant)` method. It uses the
compiler-backed Maven profile and the configured one-author draft path. It does
not review or publish the resulting explanation.

The checked-in [service input](../../fixtures/documentation-internal-process/documentation/service.json)
uses the synthetic repository ID and `https://example.invalid/dispatch-fixture`
remote. The [process input](../../fixtures/documentation-internal-process/documentation/process.json)
selects the exact owner, method, parameter types and Maven scope. These values
are fixture metadata, not a real service or remote.

## Prepare isolated checkouts

Install the public release and confirm `clew --version` reports `0.13.0`. This
recipe needs Git, JDK 21, Maven and a configured author driver. The fixture's
Maven compiler target is Java 17; JDK 21 is the analysis host requirement.

Copy the fixture from the release tag into a new Git repository with a clean
`main` branch. Give that local repository the fixture's synthetic remote; do
not push to it.

```sh
curl -fsSL https://codeclew.github.io/codeclew/install.sh | CODECLEW_VERSION=v0.13.0 sh
clew --version

mkdir -p /tmp/task-promotion-example
git clone --branch v0.13.0 --single-branch https://github.com/codeclew/codeclew.git /tmp/task-promotion-example/codeclew-v0.13.0
mkdir -p /tmp/task-promotion-example/dispatch
cp -R /tmp/task-promotion-example/codeclew-v0.13.0/fixtures/documentation-internal-process/. /tmp/task-promotion-example/dispatch/
cd /tmp/task-promotion-example/dispatch
git init --initial-branch=main
git remote add origin https://example.invalid/dispatch-fixture
git add .
git -c user.name='Fixture Maintainer' -c user.email='maintainer@example.invalid' commit -m 'Add dispatch fixture'
git status --short --branch
git remote get-url origin
```

The last commands should show a clean `main` branch and the literal
`https://example.invalid/dispatch-fixture` remote. The metadata is intentionally
synthetic and requires no network access.

## Capture and select the callable

Keep the documentation root separate from the fixture checkout. Read each
returned `inputDigest` and use its full value for the next catalogue write.
Copy the service input digest from the first `service list` response.

```sh
clew docs init --root /tmp/task-promotion-example/docs --title 'Task promotion draft'
clew docs service list --root /tmp/task-promotion-example/docs
clew docs service add --root /tmp/task-promotion-example/docs --input /tmp/task-promotion-example/codeclew-v0.13.0/fixtures/documentation-internal-process/documentation/service.json --expected-input-digest SERVICE_INPUT_DIGEST
clew docs bind --root /tmp/task-promotion-example/docs --service dispatch --repo /tmp/task-promotion-example/dispatch
clew docs check --root /tmp/task-promotion-example/docs --service dispatch
```

Save the exact `snapshot` returned by `docs check` as `ORIGINAL_CAPTURE`. Inspect
the selected callable in that retained snapshot and copy its returned SYMBOL
observation ID. Verify its `ownerIdentity` is
`class:example.dispatch.TaskStateTransitions`, its name is `promote`, its
`parameterTypes` are `example.dispatch.Task` and `java.time.Instant`, and its
scope is `:/main`.

```sh
clew docs context --root /tmp/task-promotion-example/docs --service dispatch --snapshot ORIGINAL_CAPTURE --symbol example.dispatch.TaskStateTransitions.promote --format raw --limit 100
clew docs process candidates --root /tmp/task-promotion-example/docs --service dispatch --snapshot ORIGINAL_CAPTURE --declaration SYMBOL_OBSERVATION_ID --lane internal
```

The context selector is a qualified declaration name. Confirm all four fields
above in its returned observation before using `SYMBOL_OBSERVATION_ID`; do not
guess an ID or substitute another overload. The candidates command is structural
navigation, not a business-process decision. Inspect its status and gaps.

An optional graph artifact can help inspect the same compiler-backed retained
call graph:

```sh
clew docs process graph --root /tmp/task-promotion-example/docs --service dispatch --declaration SYMBOL_OBSERVATION_ID --snapshot ORIGINAL_CAPTURE --output /tmp/task-promotion-example/task-promotion-graph.json
```

That artifact preserves retained call and source evidence. It does not establish
runtime dispatch, execution order, persistence, or graph completeness.

## Save and prepare the process request

Inspect the public process input against the original capture before saving it.
Then read the current catalogue digest from `process list`, save the definition
with that exact digest, and recompose from the original capture. Use the new
snapshot returned by recomposition for preparation.

```sh
clew docs process inspect --root /tmp/task-promotion-example/docs --input /tmp/task-promotion-example/codeclew-v0.13.0/fixtures/documentation-internal-process/documentation/process.json --snapshot ORIGINAL_CAPTURE
clew docs process list --root /tmp/task-promotion-example/docs --limit 100
clew docs process put --root /tmp/task-promotion-example/docs --input /tmp/task-promotion-example/codeclew-v0.13.0/fixtures/documentation-internal-process/documentation/process.json --expected-input-digest PROCESS_INPUT_DIGEST
clew docs recompose --root /tmp/task-promotion-example/docs --snapshot ORIGINAL_CAPTURE
clew docs process prepare --root /tmp/task-promotion-example/docs --id task-promotion --question 'Explain the decision and data flow in TaskStateTransitions.promote, its repository-call boundaries, and what retained source cannot establish about persistence or runtime behavior.' --language en --snapshot RECOMPOSED_SNAPSHOT
```

`ORIGINAL_CAPTURE` is the immutable `snapshot` returned by the original
`docs check`. `process inspect` is transient and saves nothing. `process put`
stores only the explicit process definition. `docs recompose` runs no analyzer and does not
change the latest-check pointer. Save the `snapshot` from its response as
`RECOMPOSED_SNAPSHOT`; save the `work` returned by `process prepare` as
`WORK_ID`.

## Run one author draft

Use the existing author-only execution configuration described in the
[service documentation skill, “Draft one internal service process explanation”](../../skills/codeclew/references/service-documentation.md#draft-one-internal-service-process-explanation).
The operator chooses the model, driver, runtime reads, finite caps and budget.
There is no default author, credential or spend authorization. Use a
configuration file prepared for this release at `PATH_TO_OPERATION_DRAFT_CONFIG`.

```sh
clew docs work run --root /tmp/task-promotion-example/docs --work WORK_ID --config PATH_TO_OPERATION_DRAFT_CONFIG --draft
clew docs work status --root /tmp/task-promotion-example/docs --work WORK_ID
```

The command makes one configured author call and writes local draft views under
`.codeclew/drafts/WORK_ID`, including `answer.json`, `operation.md` and
`index.html`. The authored decision tree can be displayed as pseudocode.
Separately, the source-local process-flow diagram is projected from retained
source syntax, not from the authored steps. Neither view is a runtime trace. The
Work stays `DRAFT` / `UNREVIEWED`; this path creates no proposal and publishes
nothing.

The checked-in sample in
[`fixtures/documentation-internal-process/documentation/sample/`](../../fixtures/documentation-internal-process/documentation/sample/)
is the actual output for this scenario. After cloning the release, open its
`index.html` locally in a browser; GitHub's file view does not render it as a
page. The sample answer and reader packet are bound to the original Work and
snapshot, so inspect them as output examples and do not pair them with a fresh
Work. Their hashes and run provenance are recorded in
[`provenance.json`](../../fixtures/documentation-internal-process/documentation/sample/provenance.json).

If a call has already saved a raw result, rerunning this command for the same
Work and configuration reuses that result. It does not dispatch an author repair
or create another invocation. A retained `ANSWER_INVALID` result is revalidated
only when the terminal checkpoint and report digests match the durable saved
result; malformed or mismatched recovery bindings fail closed. Revalidation
does not repair the answer, and a result that still fails validation remains
invalid.

Process-definition citations keep their declared authority: the definition is
user intention, and a declared continuation does not show that a call ran. The
question and outcomes request an explanation; they do not prove state changes,
database persistence or runtime behavior. This experiment covers only the
source-local `promote` decision method, not the later dispatch path or concrete
repository implementation.

For the checked-in sample, one author call took 164.078 seconds. Native
saved-result replay then validated the same Work, run, invocation and raw
answer; the saved-result file hash and attempt/accounting were unchanged, and
the replay made no new author dispatch. The source-FIT review and Chrome browser
review accepted this scenario; the browser review checked desktop at 1365 px
and mobile at 390 px and confirmed its citation links. The result remains `DRAFT` / `UNREVIEWED` and is
not published. These results cover this one source-local method and rendered
sample, not complete-workflow qualification or runtime and persistence claims.
The single-run timing is diagnostic, not a latency claim. Use only the public
fixture and synthetic paths in this runbook; do not copy private application
paths, source or experiment inputs into public notes.
