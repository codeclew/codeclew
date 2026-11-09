# Documentation state diagnostics and safe authored writes

The bounded stdin transport and structured diagnostics below belong to the
accompanying source change. Check the installed command's `--help` before using
them. Codeclew 0.14.8 already returns `inputDigest` on successful writes; that
return value is not a new feature.

## A selected service fails because of another declaration

Documentation input identity covers the global authored catalogue. Selecting
`docs check --service orders` narrows acquisition, not declaration integrity.
An invalid scenario elsewhere must still fail before source acquisition. Process
errors identify `recordId`, `recordPath`, `field`, `reason` and `constraint`.
Closed JSON/YAML errors identify the input and failing schema field without
including submitted values. Global validation failure reports
`inputDigestStatus: UNAVAILABLE`; it does not manufacture a digest.

Correct the named authored record and retry the inspection. An external actor is
legal when registered, even with `sourceCapture: NOT_CAPTURED`. It remains an
authored destination: it cannot become a captured process root, method or retained
page binding. Unknown actors need registration or correction, not removal of all
external participants. Duplicate IDs and invalid bounds remain errors.

If invalid global input prevents a trustworthy CAS identity, preserve a copy and
correct only the named authored YAML/JSON declaration with an editor. These are
authored files, not private SQLite objects. Inspect again after the correction to
obtain a fresh digest for subsequent puts; do not invent an expected digest or
disable validation to force the repair through a write command.

Deleting Work, proposal or latest-check files cannot repair malformed scenario
YAML. Do not silently filter a bad scenario to make a service check pass.

## Storage ownership

| State | Storage and meaning |
| --- | --- |
| Root identity and authored declarations | `codeclew-docs.yaml`, `catalog/`, `scenarios/`, notes and authored narrative files. Preserve their bytes, comments and manual content. |
| Local bindings | `.codeclew/bindings/`; preserve source/repository associations separately from captured evidence. |
| Native Work | `.codeclew/work/ID/work.json` and read manifests; referenced immutable payloads and shared tables can live in the object database. A Work manifest alone is not its evidence closure. |
| Native proposals | `.codeclew/proposals/ID.json`; associated diagnostics and immutable evidence can reference stored objects. Proposal state is not entirely SQLite. |
| Retained objects | The database selected by `.codeclew/cache/object-layout.json`, including its required consistent SQLite state. Do not guess a database from its filename or edit private objects. |
| Latest check | `.codeclew/cache/latest-check.json` is updated by saving a check snapshot, not only by a successful export. It is a convenience pointer, not a replacement for the retained snapshot's complete object closure. |

Compose-original Markdown pages use their original fragments, citation manifests,
page maps and checked export route. They are distinct from native
Work/proposal/publication state; deleting a native proposal does not recreate a
compose-original source map or repair its source evidence.

## Classify before recovering

| Diagnostic | Meaning and safe next action |
| --- | --- |
| `LEGACY_ROOT_SCHEMA` | The manifest explicitly declares the known 1.0 schema. Preserve the original; a compatible historical reader or archive remains distinct from fresh indexing. Restoring a legacy backup alone does not migrate its schema. |
| `ROOT_SCHEMA_UNSUPPORTED` | The manifest declares an unsupported schema; no particular object layout is inferred. |
| `ROOT_NOT_INITIALIZED` | No documentation manifest or state was found. Initialize only if a new empty root is intended. |
| `ROOT_MANIFEST_MISSING` | State or authored directories remain without their root manifest. Preserve them and restore a consistent backup; do not treat this as an empty root. |
| `OBJECT_LAYOUT_MARKER_MISSING` | Modern database files were observed without a selecting marker. Do not choose a database or invent a marker. |
| `OBJECT_LAYOUT_UNDETERMINED` | The marker is absent and the layout cannot be established. Legacy object directories are evidence of old files, not proof that the whole root is legacy. A damaged modern root can look similar. |
| `OBJECT_LAYOUT_MARKER_INVALID` / `OBJECT_LAYOUT_UNSUPPORTED` | The marker is unreadable, violates its schema or declares unsupported metadata. Preserve it for diagnosis; these errors alone do not establish a legacy layout. |
| `OBJECT_DATABASE_MISSING` / `OBJECT_DATABASE_INVALID` | The selected database is missing, inaccessible, corrupt or incompatible. No empty replacement database is created. Preserve the database and related state. |

None of these diagnostics automatically resets or deletes state.

| Available material | Recovery |
| --- | --- |
| Consistent complete backup | Restore the complete consistent root backup into a separate directory, including the matching database and required SQLite recovery files. Keep the damaged original; do not mix files copied from different live states. Verify the complete manifest/object closure before reusing its retained handles. |
| Sources available, no consistent backup | Preserve catalogue, bindings and manual content. Initialize a **different empty root**, register declarations, bind the available sources and explicitly reindex there. Review and reapply authored material against the newly obtained evidence. This produces new evidence and does not recover the lost snapshot, Work or review authority. |
| Neither sources nor consistent backup | Preserve remaining files and existing exports as historical artifacts. Report the missing evidence closure and stop source-bound mutation/publication that requires it. No lost snapshot can be reconstructed from rendered HTML, a pointer or a partial database. |

For an intentionally new root, ordinary setup begins with `clew docs init --root
NEW_EMPTY_ROOT --title TITLE`. After registering a service, use `clew docs bind
--root NEW_EMPTY_ROOT --service orders --repo AVAILABLE_CHECKOUT` and an explicit
`clew docs check --root NEW_EMPTY_ROOT --service orders`. These are new indexing
steps, not a repair command to run inside the damaged root.

## Chain writes and reconcile conflicts

Use the `inputDigest` returned by `SAVED` or `CURRENT` as the next write's
`--expected-input-digest`. A changing write with a stale or absent digest fails
without mutation. When the writer holds the existing lock and can compute the
current identity, `WW_CONFLICT.details.currentInputDigest` provides it. A failed
global validation or an unavailable lock cannot promise such an identity.

Reload the current catalogue, compare the intervening changes and reconcile the
intended edit before explicitly retrying. Do not blindly copy a conflict digest
into an automatic retry. An identical write can return `CURRENT`; this is not a
create-only contract or permission for a missing digest to overwrite state.

## Bounded stdin for authored puts

`docs external put`, `docs interaction put` and `docs process put` explicitly
accept `--input -`. Supply one JSON or YAML record and close stdin. The receiver
reads at most the existing limit plus one byte before rejecting oversized input.
Limits remain 64 KiB, 128 MiB and 256 KiB respectively. Empty, invalid or oversized
input is not applied. Regular-file input still rejects symlinks and nonregular
files. These CLI byte limits are separate from chat/transport message limits.

For example, replace `EXPECTED_DIGEST` with a returned digest:

```sh
clew docs external put --root OWNED_DOCS --expected-input-digest EXPECTED_DIGEST --input - <<'JSON'
{"schema":"codeclew-documentation-external-service/1.0","id":"remote","title":"Remote","description":"Declared destination","declaration":{"origin":"agent-proposal","rationale":"Maintainer request"}}
JSON
```

Only these authored put commands gain stdin support; other `--input` consumers
keep their documented transport. Batch writes and automatic conflict retries
remain outside this change.
