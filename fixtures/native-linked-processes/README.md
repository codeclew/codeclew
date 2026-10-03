# Owned native linked-process fixture

This working Java fixture supplies source for a later linked-process projector
slice. Two endpoint/queue/worker journeys call one shared child endpoint. The
child worker prepares a request, attempts a fixture-local gateway call and records
its local response state. All execution is explicit and deterministic: no HTTP,
network, scheduler, database or customer delivery is involved.

The source contains distinct parent guards and call occurrences, a different
`submit` target, a cyclic call graph, and an interface target with no method body.
These are ordinary Java constructs rather than documentation answer annotations.
The current native projector does not automatically expand child processes;
fixture tests and source capture do not qualify that proposed behavior.

## Build and run

Use JDK 21 and Maven on `PATH`. Plugin versions and JUnit 5.12.2 are pinned in
`pom.xml`. The executable `mvnw` follows the existing repository fixture convention:
it delegates to real `mvn`; it does not download Maven or emulate its metadata.
The first ordinary Maven build may download public plugins and test dependencies.
The Java driver itself uses only the JDK and local fixture classes.

From a fresh Codeclew source clone containing this fixture:

```sh
git clone https://github.com/codeclew/codeclew.git codeclew
cd codeclew
export JAVA_HOME=$(/usr/libexec/java_home -v 21)
export PATH="$JAVA_HOME/bin:$PATH"
cd fixtures/native-linked-processes
./mvnw -B -ntp test
java -cp target/classes example.linked.FixtureDriver
```

On Linux, set `JAVA_HOME` to the installed JDK 21 directory instead of using the
macOS `java_home` command. No custom Maven settings or private dependencies are
required. The driver submits one task through each parent, explicitly runs each
parent once and the shared child twice, then prints both parent states, child
state, attempt count and recorded local requests.

JUnit tests check shared-child FIFO ordering, no gateway attempt before the child
runs, empty/null inputs, parent guard precedence, distinct priority thresholds,
null versus blank request defaults, Unicode transformation, child refusal, the
alternative queue, nonzero response and exception propagation. A finite explicit
counter permits a deterministic test of the cyclic methods without assuming that
static graph traversal is complete.

## Capture committed source

Use an isolated copy so its Maven root and Git root coincide. Run the following
from the Codeclew checkout root; keep the source-development requirements from
[the repository README](../../README.md) available for `./clew`.

```sh
LINKED_FIXTURE_SOURCE=$(mktemp -d "${TMPDIR:-/tmp}/linked-source.XXXXXX")
LINKED_FIXTURE_DOCS=$(mktemp -d "${TMPDIR:-/tmp}/linked-docs.XXXXXX")
cp -R fixtures/native-linked-processes/. "$LINKED_FIXTURE_SOURCE/"
git -C "$LINKED_FIXTURE_SOURCE" init -b main
git -C "$LINKED_FIXTURE_SOURCE" remote add origin https://example.invalid/native-linked-processes
git -C "$LINKED_FIXTURE_SOURCE" add .
git -C "$LINKED_FIXTURE_SOURCE" commit -m 'Add owned linked-process fixture'
./clew docs init --root "$LINKED_FIXTURE_DOCS" --title 'Owned linked processes'
./clew docs service list --root "$LINKED_FIXTURE_DOCS"
./clew docs service add --root "$LINKED_FIXTURE_DOCS" \
  --input fixtures/native-linked-processes/service.json \
  --expected-input-digest RETURNED_INPUT_DIGEST
./clew docs bind --root "$LINKED_FIXTURE_DOCS" --service linked \
  --repo "$LINKED_FIXTURE_SOURCE"
./clew docs check --root "$LINKED_FIXTURE_DOCS" --service linked
./clew docs context --root "$LINKED_FIXTURE_DOCS" --service linked \
  --snapshot RETURNED_SNAPSHOT --format raw --limit 100
```

Use a configured Git identity for the local fixture commit. Replace
`RETURNED_INPUT_DIGEST` with the preceding service list's complete `inputDigest`
and `RETURNED_SNAPSHOT` with the check's exact immutable snapshot handle. Follow
returned context cursors when needed. The illustrative origin is a source identity;
no remote fetch or remote HTTP behavior is asserted. Real Maven effective-model,
compile/classpath and release-property extraction run during the native capture.
Dirty/untracked source edits are outside the captured committed revision.

## Exact roots and source boundaries

All classes below are in `example.linked` and the main compilation scope `:/main`.
Obtain actual retained callable observation IDs from the capture; source names
are discovery aids and are not immutable declaration IDs.

| Selected process | Endpoint root | Worker root | Wiring root |
|---|---|---|---|
| Parent A | `ParentAEndpoint.submit(Task)` | `ParentAWorker.runOnce()` | `Composition.wire(Pipeline, Gateway)` |
| Parent B | `ParentBEndpoint.enqueue(Task)` | `ParentBWorker.runOnce()` | `Composition.wire(Pipeline, Gateway)` |
| Shared child | `ChildEndpoint.submit(Task)` | `ChildWorker.runOnce()` | `Composition.wire(Pipeline, Gateway)` |

`Composition.wire` has straight-line local queue allocations and constructor
calls. Each endpoint and worker receives its process's queue directly; both
parent workers receive the same child endpoint object. `Composition.assemble`
returns a holder for the runnable driver, while `wire` is the explicit source
wiring root without an early return.

Additional source roots:

- `ChildWorker.prepare(Task)` holds null defaulting, trimming and prefixing.
- `ParentAWorker.runOnce()` and `ParentBWorker.runOnce()` have distinct
  `child.submit(task)` occurrences and compiler owners.
- `RetargetEndpoint.submit(Task)` has the same name/signature as the child method
  but targets a separate held queue. `ParentBWorker` explicitly selects it when
  `useAlternative` is true.
- `CycleProbe.first(int)` and `CycleProbe.second(int)` call each other. They are
  isolated from the driver pipelines and provide a cycle/frontier case.
- `Gateway.deliver(String)` is a fixture-local interface declaration without a
  body. `ChildWorker.runOnce()` calls that declared target; a source-only consumer
  cannot choose `RecordingGateway` merely because the driver supplies it.

Future source-call links must bind the exact compiler target, call occurrence,
scope and retained body. A declaration with no body, ambiguous target, cycle or
expansion budget must remain a local frontier. Page links do not establish
receiver identity, runtime dispatch, automatic worker invocation or delivery.
The source wiring and the tests' explicitly driven execution have different
qualification scopes.

## Controlled future source mutations

Make each variation in a separate committed fixture copy, capture a fresh snapshot
and compare its generated documentation with the baseline. Existing snapshots
and bundles should remain immutable.

| Change | Meaningful check for the later projector slice |
|---|---|
| In `ChildWorker.prepare`, change `chosen.trim()` to `chosen.strip().toUpperCase(java.util.Locale.ROOT)` | The retained request expression/citation changes; both examined parent roots remain linked to the child and become candidates for source review. |
| In `ParentAWorker.runOnce`, change `task.priority < 1` to `task.priority < 2` | Only Parent A's local guard changes; the equality case in the Java tests distinguishes the behavior. |
| In `ParentBWorker.runOnce`, retarget its sole `child.submit(task)` occurrence to `alternative.submit(task)` | Parent A still calls the child; Parent B's retargeted compiler call must not link to the child by method name. |
| Add `CycleProbe.first(int)` as a root | The exact back edge is retained with a bounded cycle frontier, without unbounded body expansion. |
| Follow `ChildWorker`'s `Gateway.deliver` target | The interface body remains unavailable; the driver implementation is not guessed as the call target body. |

These are planned native-documentation regression cases. Runtime tests establish
only the owned Java examples after an actual successful test run; compiler capture
establishes only the reported retained source/evidence. Neither establishes
corporate usefulness, people-task results, whole-program change safety, canonical
2,000-process navigation or cross-service transport semantics. Captured process
metadata, static navigation and projector expansion remain separate next work.
