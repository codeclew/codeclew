# Documentation operations compatibility

Saved publications and exact Check/Work handles from 0.13.47 remain the inputs.
Exporting a reading site does not capture source, replace an accepted proposal,
rewrite a historical publication, or substitute a current revision for a citation.

Remote service/process selection uses `--export-subject service:ID` and
`--export-subject scenario:ID`. Without these options, the whole saved publication
is selected. Declaration selectors remain a separate view and do not replace
existing authored service/process documentation. DSL renders do not accept this
publication selection option.

Partitioned bindings are read through their declared selected closure. Legacy
bindings have no selection index and require reading their bounded original
record before filtering. Remote reading bindings are not an authoring or evidence
reimport format. Preserve the local publication and original source contexts.

Shared overview/status/answer context remains global in a selected export. The
export manifest lists omitted service/process subjects; their navigation is
inert. Diagram prefixes that cannot distinguish selected from omitted subjects
are rejected explicitly. The full export remains available for those publications.

Source authority remains distinct: compiler capture, committed syntax-only
capture, declared contracts, authored prose and runtime observations retain their
existing meanings. Successful export does not establish new compiler verification,
runtime behavior or provider availability.

Remote semantic validation preserves compact shared references while checking
expanded fragments one at a time, including protected source-snapshot pins.
A repeated source is not kept as simultaneous copies for every fragment. The
manifest reports compact logical bytes, maximum expanded fragment bytes and
charged expansion work separately. Individual expanded fragments remain bounded
at 128 MiB, cumulative expansion work at 512 MiB and original authoring contexts
at eight. These reading bounds do not change baseline/Work evidence expansion.

## Source browser policy in 0.13.48

Optional service `sourceWeb` separates provider/webBase from the clone origin.
Absent configuration serializes exactly as before. Frozen SOURCE and explicit
sourceLinkTemplate expansion stay unchanged; remote source maps can override
old records without recapture. Java compiler protocol escapes Unicode to retain
exact file names under the closed C locale; source membership validation remains
mandatory.

## Storage lifecycle

Every supported open documentation repository holds a shared storage lease.
Pruning requires an exclusive lease, revalidates the exact root/reference plan,
and refuses concurrent operations. All retained root files remain live; no age
policy is inferred. SQLite deletion creates reusable space rather than promising
a smaller database file. Portable evidence files are separate from local SQLite
and runtime CODECLEW_HOME storage. Service/entity removal changes only the live
registry after checking references; old immutable publications are not rewritten.

## External-service registry

Optional externalServices input records are omitted from serialization when
empty, preserving old input digests. Explicit interaction.externalService
references require an existing external record and an identical to.service.
Legacy external destinations without this field remain identity-only. Captures
and recomposition freeze registry records as declared metadata without source
capture. All exports preserve the declared-information boundary and provenance;
removal blocks current references and never changes historical publications.
