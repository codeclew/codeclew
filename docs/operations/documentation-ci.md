# Documentation jobs and GitLab operation

The portable job runner is `python3 -I -S scripts/documentation_ci.py`. Local and
GitLab jobs consume the same versioned event/job files and invoke the supported
`clew` launcher. The [GitLab recipe](../../examples/documentation-ci/gitlab-ci.yml)
requires an operator-configured runner and artifact channel. Repository tests use
fake commands and platform responses; they do not establish live GitLab readiness.

## Install and configure

Install the trusted script, launcher and agent runtime outside both application
and documentation repositories. Python 3.11+ is required. Core agent isolation
currently supports `macos-seatbelt-stdio/1.0`; use a macOS GitLab runner for agent
jobs. Linux can coordinate or capture supported evidence, but it cannot run this
agent adapter: `ISOLATION_UNAVAILABLE` remains an actionable local generation gap.
No unrestricted subprocess is substituted for an unavailable sandbox.

Register services and accepted-ref policies as described in the
[evidence workflow](../../skills/codeclew/references/documentation-evidence.md).
Create an operator-owned installation file with absolute local paths:

```json
{
  "schema": "codeclew-documentation-ci-installation/1.0",
  "clewCommand": ["/opt/codeclew/bin/clew"],
  "docsRoot": "/work/architecture",
  "artifactRoot": "/work/job/artifacts",
  "audience": "Authorized service maintainers",
  "executionConfig": "/operator/execution.json",
  "maxWork": 8,
  "timeoutSeconds": 600
}
```

For this source checkout use its absolute `clew` path in `clewCommand`. Both roots
must already exist. Producer configuration points to its registered/bound local
source capture repository; coordinator configuration points to central docs.
In the recipe, `artifactRoot` is the job's absolute `artifacts` directory. Restore
the central documentation repository from its durable store before each job and
persist it after update, even when publication credentials are unavailable.
`executionConfig` is optional: without it, status still refreshes and pending
authoring is reported. Event or source text cannot supply command configuration.

```sh
python3 -I -S scripts/documentation_ci.py build-job --event event.json --id capture-42 --kind capture --output capture-job.json
python3 -I -S scripts/documentation_ci.py run-job --config /operator/producer.json --input capture-job.json --output artifacts/capture-result.json
python3 -I -S scripts/documentation_ci.py build-job --event event.json --id update-42 --kind update --capture-result artifacts/capture-result.json --output update-job.json
python3 -I -S scripts/documentation_ci.py run-job --config /operator/coordinator.json --input update-job.json --output artifacts/update-result.json
```

The source event names a full commit and a configured accepted ref. A trusted
source pipeline supplies it as `DOCUMENTATION_EVENT_JSON`; never accept a model
response or an arbitrary source file as that authority. Capture verifies the
observed revision against the event. Transfer the entire package and capture
result over the authorized artifact channel: a producer's content hash alone
does not authenticate it. Update accepts the target before inspecting the package,
then checks origin, exact revision, compatibility and trusted manifest digest.
Missing evidence leaves conservative status and a retryable integration gap.

Job IDs are immutable idempotency keys. Completed replays return the retained
result; changed content under the same ID is rejected. Failed jobs retry the same
event and expectation. A `reconcile` job supplies an `events` array; a `status`
job refreshes the configured bounded queue. Use a new job ID for a new queue run.
The artifact-root lock rejects concurrent local jobs with
`COORDINATOR_BUSY_RETRY`. GitLab's `resource_group` serializes separate runner
workspaces; the core independently checks catalogue and target versions.

## Roles, permissions and accounting

`documentation_ci.py agent --config /operator/agent-command.json` is an optional
stdio driver for an operator-owned HTTPS model gateway. The
[command configuration example](../../examples/documentation-ci/agent-command.json)
selects separate author/reviewer/fallback endpoints, exact model names and
credential environment references. It sends the existing versioned immutable job
and expects the existing versioned agent result envelope. The gateway must enforce
the job's finite token/cost caps before dispatch and report provider usage in the
outer envelope. Missing usage stays unavailable; the core retains the reservation
maximum. No vendor SDK or hard-coded model is required. A routine model such as
DeepSeek-V4-Flash-0731 or a stronger fallback is an operator choice requiring the
separate quality qualification; this example is not a suitability claim.

Register the Python executable, stdlib, script and gateway configuration as
read-only `runtimeReads` in each core execution role, outside protected roots.
The role command is an absolute argv array, for example Python followed by
`-I -S /opt/documentation/documentation_ci.py agent --config
/operator/agent-command.json`. Allow only that role's credential environment names
and enable network only for the trusted transport. HTTP redirects are refused.
Payload text cannot change endpoints, credentials or executables. The gateway is
trusted transport infrastructure, not an agent-controlled browsing tool.

Source-read/provider permissions belong to capture jobs. Author/reviewer roles
read only captured stdin and registered runtime files; their only result channel
is stdout. The core owns review validation and documentation writes. Publication
credentials belong only to the protected publication job. Credentials must not
appear in service records, events, packages, public manifests or command arguments.

Keep `execution/accounts` as durable, access-controlled coordinator state. It
contains finite reservations and charges, not credentials or machine paths.
Old `.codeclew/accounts` ledgers migrate on the next reservation, including denied
reservations. Before disposing of an older release's cache, migrate/preserve those
ledgers. Never reconstruct an account as empty to recover from a failed job.
Full-contour budgets must include qualification/evaluator calls when those run.

## Retention, failure and publication

Isolate each worker's `CODECLEW_HOME` with the supported launcher configuration;
never share a mutable worker home concurrently or edit its private objects.
Use supported lifecycle cleanup for Codeclew runtime state. Documentation work
cache `.codeclew` is disposable only after durable evidence, accepted records and
budget ledgers are retained and no work is in flight. Work attempts and safe
diagnostic results can be retained privately for troubleshooting.

Persist `catalog`, `updates/events`, authored records, `execution/accounts`,
`evidence/packages`, and `docs/generated` together. The recipe's seven-day GitLab
job artifacts are transfer buffers, not the historical retention policy. Retain
packages as long as any supported snapshot references them, for an audience
authorized to read their exact source. History inspection reports missing or
damaged artifacts explicitly. Capture cannot reconstruct source from a source-free
report. Cache loss cannot erase retained history or recorded budget charges.

SIGINT/SIGTERM and timeouts stop the local command group. Core `docs work cancel`
signals an in-flight role and preserves dispatched reservations; after abrupt
runner termination, inspect the writer lock and retained accounting before retry.
Jobs emit machine-readable results and stable gap reasons; they send no Slack,
email or other notifications. Operators may route result statuses through their
own configured notification system.

The optional `publication` installation object contains `command` (absolute argv),
`credentialEnvironment` (nonempty list of names), and `audience` equal to the
installation audience. `publish` requires a retained completed central result,
unchanged targets and available publication credentials. It sends a versioned
publish request on stdin to the configured publisher. That publisher must compare
the expected remote docs revision, persist the entire durable artifact, enforce
audience access and return a `codeclew-documentation-publish-result/1.0`
envelope with the request’s `jobDigest` and status `PUBLISHED`, `UNCHANGED`,
`CONFLICT` or `PUBLICATION_GAP`. No publisher or credential is inferred.
Missing credentials yield `PUBLICATION_CREDENTIALS_UNAVAILABLE` without discarding
local documentation. Do not expose private source as public Pages output.

## Configured GitLab qualification

`qualify-gitlab --config <installation-file> --output <new-results-directory>`
triggers actual pipelines only with explicit installation configuration. Its
schema is `codeclew-documentation-gitlab-qualification/1.0`, with `apiUrl` (HTTPS API
v4 base), numeric `projectId`, a configured pipeline `ref`, `credentialEnvironment`,
absolute argv `localCommand`, finite `pollSeconds` (1–60), `timeoutSeconds`
(1–3600), and 1–16 `cases`. Each case has `id`, exact `event`, `jobName`, relative
`artifactPath` and `expected` outcome object. The script supplies case ID and event
as pipeline variables, polls the pipeline, and reads the named successful job's
JSON artifact. Configure the fixture project to execute those source revisions
and the requested failure/isolation/retry/concurrency cases using fake models.

The local command and GitLab artifact return
`codeclew-documentation-platform-result/1.0`, `id`, `revision`, and `outcomes` (at most 64 named boolean or integer measurements; no free-form
logs or source excerpts).
Only matching identity, revision and expected outcomes count as a matched case.
Retain source-acceptance, evidence-transfer, denied access, local failure,
idempotency, publication concurrency and recovery evidence in the configured
qualification protocol. Reports retain pipeline/job IDs and explicit failures.
An actual trigger plus a matching report establishes only those configured cases;
it does not prove model quality. T16 owns live execution and readiness assessment.
