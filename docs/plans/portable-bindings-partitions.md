# Portable bindings partitions

Status: implementation candidate, after public 0.13.43. The user subsequently
authorized independent acceptance, compiler/SOURCE managed-session verification
and a public release after all required checks. The executable fixture is frozen in
`fixtures/bindings-storage-scale/spec.json` before measurements.

## Layout and identity

New `docs/generated/<publication>/bindings.json` writes use
`codeclew-documentation-bindings/1.6`. A small root manifest binds the exact
logical compact 1.4/1.5 value by its canonical digest, a complete flat object
closure, and service/subject/shared partition references. It is not a hidden
inline compatibility copy. Partition membership retains table keys separately
from content identity, including shared scenario records and historical source
occurrences.

Publication `outputHashes` stays in the bounded manifest header as small root
metadata. The common resolver includes it in the exact logical digest check.
Static-site validation can verify the archived root-output role and the flat
declared byte closure without implementing another logical evidence resolver.

Objects live at `docs/evidence/objects/<sha256>.json`, outside the private cache.
They reuse the documentation `ObjectRef` schema/digest/size contract, canonical
hashing and repository safe-path/atomic-write primitives. There is no additional
database, mutable pool index, planner or GC. The pool is a portable projection of
publication evidence; private capture/fact membership and protected Work snapshots
keep their existing contracts.

Leaf chunks contain typed records, with a 1 MiB target and an existing 128 MiB
individual hard bound for a single large structured record. Source occurrence
metadata and source text have distinct identities. A partition member binds both;
reconstitution must preserve the exact revision, range, text digest, authority and
URL. Fragment-local historical observations and sources have explicit membership;
there is no current-source or source-ID-only fallback. Shared scenario data has
explicit shared/subject ownership instead of an invented single-service owner.

Historical memberships are separate sharded entries owned by their fragment's
subject, not one giant nested fragment index. The reference graph is closed at
index-to-leaf depth; leaf data never defines another portable reference.

The previous publication's validated leaf membership is a reuse hint. Unchanged
records retain their existing chunk locations; new records are packed into new
immutable chunks. Membership and referenced bytes remain authoritative, never the
hint. Chunks may contain unselected leaf records; the complete physical chunk is
counted and copied. Those records have no implicit nested portable references.
Remote exports reconstruct only selected logical members, apply the existing
source projection, and pack a fresh reading-only closure. They never copy the
original evidence chunks or expose their unused source bytes.

## Validation, budgets and publication

One resolver validates references, closure ownership, types, membership, sizes,
hashes and reconstructed logical digest before existing binding validation. Live
baseline, history, job/proposal acceptance and exports use that resolver. Existing
authorship snapshot validation remains after resolution; a portable publication
does not replace `.codeclew` authoring backup.

The manifest and each partition index are limited to 1 MiB. The unique physical
closure, including manifest, indices and chunks, is limited to 128 MiB; reconstructed
compact logical input is also charged against 128 MiB. Duplicate/conflicting
membership and references, missing/corrupt objects, unsafe paths and schema/budget
violations are errors. Full Pages keeps its separate 64 MiB cumulative original
input budget, charging every required closure object and other frozen input before
hydration. It charges the larger of physical bindings closure and reconstructed
compact logical bindings, plus other files. Shards and repeated references do not
bypass either aggregate gate. Structural preflight limits a manifest to 8,192
objects/partition references and 32 tables/header fields, a chunk/index to 1,024
records/members, complete membership to 262,144 entries, JSON depth to 128 and
nodes to 1,000,000 per wire object. Keys are limited to 64 KiB. These checks run
before constructing an unrestricted JSON value tree.

All objects are written and verified before a generated bundle and its atomic
`docs/index.html` pointer can become visible. Existing immutable object conflicts
are rejected, never repaired or overwritten. Failure before pointer switch leaves
the previous publication selected. Orphan immutable output is retained; cleanup
and destructive migration are outside scope.

Publication identity includes the storage schema. If an existing immutable
publication already represents the exact logical state, its verified packing is
reused verbatim, even if a different previous publication would pack it differently.
Storage changes therefore neither overwrite old formats nor create an endless
chain of new IDs for an unchanged render.

## Compatibility and export

Inline 1.4/1.5 reads remain supported. New writes use 1.6 without rewriting old
publications. Public 0.13.43 cannot read 1.6 and must reject it as unsupported;
checking out only an older manifest is not a downgrade strategy. Git/copy/backup
must retain the selected bundle, pointer and all objects in its declared closure.
Copying only `bindings.json` is incomplete. Protected authoring contexts still
require their original snapshots.

Embedded standalone and Docusaurus downloads include complete portable bindings
closures. Remote downloads have an explicitly reading-only manifest, freshly
hashed projected objects, preserved authored quotations/contracts and historical
links, and no automatic source attachment bytes. Their export inventories and
identities bind the actual projected files; they are not evidence imports.
Reading publication metadata separates the original publication (and its original
hashes) from the actual export inventory. Docusaurus exposes the raw portable
closure under `static/clew-bindings/`; it does not emit an extra inline compatibility
copy through its ordinary data normalizer. Its existing source appendix still
retains exact historical occurrences and source-policy-approved links.

## Finite verification and honest limits

The frozen 1/10/100-service structured fixture uses three generations: initial,
unchanged evidence, then one-service/operation change. It uses production storage
write/read paths, realistic bounded source/observation/fragment records and shared
scenario dependencies; it does not capture a compiler for each synthetic service.
Counters include all pool files, indices, manifests, total/delta bytes and file
counts. Compare with exact legacy inline bytes of the same logical publications.
Record hydration count/bytes, wall time and process peak RSS where available.
Preserve any honest budget failure; never widen a gate to make 100 pass.

Separate owned compiler and SOURCE fixtures establish semantic parity, historical
occurrences, protected-context requirements, cold copy, export policy and failure
atomicity. A deliberate structured over-budget case is required, not a sparse file.

This first slice still hydrates selected bindings eagerly and existing fragment
expansion can clone evidence. File partitioning does not establish bounded RAM or
latency improvement. Scoped/lazy hydration and any customer-scale claim are deferred.

### Initial native measurement

The fixed fixture passed all three generations for each service count. These
are development-profile storage measurements, including all portable pool,
bundle and root metadata. The inline reference substitutes the exact same
logical bindings and recomputes publication hashes in the same inventory.

| Services | Total bytes after three publications | Inline reference bytes | Reduction | New pool bytes for one-service change | Process peak RSS bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 3,043,723 | 3,280,549 | 7.22% | 61,767 | 43,597,824 |
| 10 | 4,693,935 | 7,529,866 | 37.66% | 64,635 | 86,933,504 |
| 100 | 21,198,152 | 50,087,002 | 57.68% | 64,640 | 507,510,784 |

The unchanged-evidence generation adds no pool bytes; output hash metadata
stays in each bounded root header, and evidence partition identities remain
unchanged. Three deliberately
distinct publication IDs exercise storage reuse. This is not a claim that an
ordinary no-op render creates another publication. Two initial harness failures
incorrectly required this changing metadata to remain identical; their logs are
retained, and only those assertions were corrected. A later static-site
compatibility fix moved output hash metadata from chunks to the bounded header;
the same fixture was rerun and intermediate successful results were preserved.
The fixture and budgets were unchanged. RSS is the per-count test process high-water mark, not a release or
customer workload memory guarantee.

The first publication is larger than the inline reference by 1.46%, 6.76% and
11.78% at these service counts. Partitions and explicit membership have a cost;
the measured benefit is reuse across retained generations.
