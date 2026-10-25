# Owned thread-order exercise

This synthetic Kotlin program makes source-to-sequence mistakes observable.
The ordinary saved-Work workflow uses static walks of delivered bodies; this
executable exercise supplies controlled practice traces.
It has no network, database or application-service dependency. Its traces describe
only the configurations in `main`; they do not establish another application's
runtime dispatch, callback behavior or collector membership/order.

Copy this directory to a new output root before running it. With JDK 21 and an
existing Gradle wrapper, run:

```sh
/absolute/path/to/gradlew --project-dir /absolute/copied-example run
```

The project pins Kotlin 2.4.10. Add `--offline` when those dependencies are already
cached. Lines beginning with `TRACE` contain the scenario, observed outcome and
ordered executed events. Keep generated builds and execution logs outside the
installed skill. No `clew` or paid model call is implicit in this command.

## Sketch the containing statements first

Read `ReportService.generate`, then the declared receiver/default-method bodies
needed for the requested thread. A compact source sketch for this example is:

```text
request.blocked -> throw BlockedRequest
versionId ?: lookupClient.getForEntity("active") -> missing version throws
collectors.forEach: collectForReport -> collect(preview=false)
prepared = preparer.prepare(request) { collect(preview=false); copy(marked=true) }
collectors.forEach: collect(prepared) -> interface default preview=true
result = engine.calculate(prepared, version)                  // outside callback
result == null -> return null; artifactKey == null -> return null
lineCount < 1 -> throw EmptyReport
Reply(requestKey <- request.jobKey, version <- resolved version,
      artifactKey <- result.artifactKey, lines <- result.lineCount)
```

Keep the `LocalEngine` backend-null `EngineUnavailable` exit and the caught
`AuditCollector` `LocalWriteFailed` outcome beside their own stages. The service's
nullable-result branch does not replace either helper outcome.

## Compare a few paths with the proposed tree

| Walk | Observed counterexample | Consequence for the sequence |
| --- | --- | --- |
| `blocked` / `no-version` | No `engine.call` occurs | Keep these exits before calculation. |
| `null-result` | `engine.call` occurs before `result.null` | A result check cannot gate the call that produces that result. |
| `callback-skipped` | No `callback.enrich`; `engine.call` still occurs after `prepare.return` | An optional callback group contains only the callback body. |
| `backend-failure` / `empty-report` | Distinct named exceptions, with no `reply.map` | Keep helper failures distinct from nullable results; no HTTP status is supplied here. |
| `write-failure` | The caught write failure is followed by calculation and reply | A write attempt does not establish durable storage, and this catch continues. |
| `success` | `reply:JOB-7,V1,ART-9,12` | Map each output field to its actual source; request and artifact keys differ. |
| `explicit-version` | No `lookup.local`; reply uses `MANUAL` | This is an explicit argument to the same defaulted function, not a shown overload. |

These traces give a reviewer concrete paths to compare with source and draft.
They are not a general semantic approval Boolean or a regex grader for prose.
A native structure/coverage pass still leaves meaning review separate.

## Close receiver and argument context through supported Work reads

In saved service Work, query the declared type rather than guessing transport
from a method name. For this example useful bounded queries are:

```json
{"query":{"kind":"SYMBOL","symbolContains":"ReportService"}}
{"query":{"kind":"SYMBOL","symbolContains":"VersionIndex"}}
{"query":{"kind":"SYMBOL","symbolContains":"Collector"}}
{"query":{"kind":"SYMBOL","symbolContains":"Preparer"}}
{"query":{"kind":"SYMBOL","symbolContains":"Engine"}}
{"query":{"kind":"SYMBOL","symbolContains":"Request"}}
{"query":{"kind":"SYMBOL","symbolContains":"Reply"}}
```

Exhaust each selection and read its returned `sourceReferences` in groups of up
to eight, retaining the same selection with each cursor. The class body exposes
`lookupClient: VersionIndex` and `Set<Collector>`; the interface body supplies
`collectForReport` forwarding `preview=false`. For that initial call, this excludes
`HistoryCollector`'s history-only branch. The subsequent `collect(prepared)` call
uses default `true` and reaches that branch in the configured runs. A Set declaration alone
does not select configured implementations or their runtime iteration order.

`VersionIndex.getForEntity` is this example's local interface method. Its name
alone does not establish HTTP GET. Reading a declared type/default method can
support a bounded source explanation even when compiler call-target resolution
is unavailable. Describe candidate implementations conditionally until linkage
is established; keep unread bodies as acquisition work rather than absence.


## Keep callback and subsequent call modes separate

The callback calls `collect(value, preview=false)` explicitly. The call after
`prepare` returns omits that argument and uses the interface default `true`.
In the configured success trace the callback's audit-write attempt precedes
`prepare.return`, then the default-mode history read precedes `engine.call`.
With `callback-skipped`, the callback's write is absent but the later history
read still occurs. Replacing the latter call with `preview=false` loses that
history read. Preserve both stage placement and argument context; the earlier
`collectForReport` forwarding `false` does not change either call's arguments.

For an owned grammar repair exercise, retain all conditional siblings when a
single nested `return` incorrectly contains children: correct only the diagnosed
node's grouping. A passing grammar check after dropping other conditional groups
would not establish that the requested thread was preserved. Compare source and
unchanged siblings as well as the affected node; do not treat a structure check
as semantic acceptance.
