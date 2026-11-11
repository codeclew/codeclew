# Documentation operations compatibility

Existing docs-root records, source snapshots, saved Work, answer JSON, user
originals and historical exports remain retained inputs. Their presence does not
select a second document generator. New pages and updates use only the
[authored-originals pipeline](../../skills/codeclew/references/compose-original.md#frozen-source-to-reviewed-pages).

Native render/page/DSL/proposal/coordinator publication commands are retired.
Their old HTML/Markdown/MDX files remain historical artifacts; they are never
rewritten by evidence reads or bounded Q&A. Existing structured bindings and
publication history retain their read codecs and integrity checks. Missing or
corrupt historical objects remain explicit errors.

Use the same exact snapshot and source selectors when migrating an authored
page. Prepare fresh declaration component tickets when the implementation digest
changes. Compare canonical payloads and rendered contract HTML before rebinding.
An old previous-index guard must reject changed component identities; explicitly
finalize a new baseline before guarded updates. Never replace digests inside an
old ticket or edit private retained objects to make a migration pass.

Existing originals without declaration components can use their normal guarded
update when their source bindings are unchanged. Preserve the old original,
source manifest, checked index and output directory alongside the new output.
An accepted original may still contain page-level `remove` or `replace` intents
from its previous guarded update. Those are one-shot instructions. When the
current accepted index confirms they were applied, use a new copy without only
those consumed intents; keep all prose, claims and evidence identical. Retain
the old raw original and record the metadata-only difference. A stale intent
must not weaken or bypass the previous-index guard.

A successful export does not establish current-source freshness or prove
arbitrary prose, runtime behavior or provider availability.

Source authority remains distinct: compiler capture, committed syntax-only
capture, declared contracts, authored prose and runtime observations retain their
meanings. Optional service `sourceWeb` separates browser links from clone identity.
Frozen source bytes and explicit source link templates stay unchanged.

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
