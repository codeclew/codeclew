# Portable publication bindings

New publications use `codeclew-documentation-bindings/1.6`. Their
`docs/generated/<publication>/bindings.json` is a small manifest. It references
typed service, subject and shared partitions, with immutable payloads under
`docs/evidence/objects/`. Unchanged payloads are shared across publications.
Original inline 1.4/1.5 publications remain readable and are not rewritten.

## Commit, copy and restore

Commit the documentation catalogue, authored inputs, `docs/index.html`, selected
generated bundle and its entire declared portable object closure. Keep every
historical bundle and its closure that must remain readable. Keeping the whole
`docs/generated/` and `docs/evidence/objects/` directories is the straightforward
backup choice; this release provides no object garbage collection.

Inspect a selected publication's closure through the ordinary launcher:

```sh
clew docs history show --root ROOT --id PUBLICATION --kind bindings-closure --limit 100
```

Each row gives a root-relative object path, schema, size and digest. Follow the
returned pagination cursor until complete. Do not copy only the first page or
only `bindings.json`. Every listed object is required, including a chunk that
contains additional unselected records from an earlier packing. Missing, corrupt
or unsafe objects fail admission; commands do not repair immutable evidence.

A cold checkout with the manifest, generated files and complete portable closure
supports read-only history and unprotected baseline inspection without the
original source checkout or private SQLite database. It does not create a local
cache. It is not a complete authoring backup: preserve the original `.codeclew/`
state for Work, pins, local bindings and protected paragraph source snapshots.
Protected text requires its original snapshot; a fresh capture cannot substitute
it. Mutations still require the normal documentation repository admission.

Upgrade readers before writing the new schema. Public 0.13.43 and earlier cannot
consume 1.6. To downgrade, restore a complete older supported publication and its
matching pointer and authoring state; replacing a schema field or retaining a
new pointer with an old manifest is invalid. Never edit generated manifests or
the portable pool by hand.

## Reading exports

Embedded standalone exports include `frozen-bindings.json` and the complete
declared object closure. Docusaurus puts these under `static/clew-bindings/`.
Copy the whole output directory. Export inventories bind actual emitted bytes;
`frozen-publication.json` distinguishes the original publication metadata from
the reading export's own file hashes.

Remote exports resolve selected logical records, apply the existing source URL
policy and create a fresh reading-only closure. They never copy original pool
chunks, which could contain unused source text. Authored quotations, contracts,
source provenance and exact historical URLs remain. Remote bindings are reading
artifacts and cannot be reimported as evidence. Exporting a reading site still
requires the selected saved snapshot and any original protected contexts; a
cold publication alone is not a replacement for those inputs.

## Bounds and interpretation

The manifest and each partition index are limited to 1 MiB. The entire physical
bindings closure and reconstructed compact logical bindings each retain a
128 MiB bound. Full Pages retains a separate 64 MiB cumulative frozen-input
budget, charging the larger bindings representation plus other frozen files.
Sharding does not bypass either limit. Structural limits also reject excessive
membership, nesting and repeated or conflicting references before hydration.

The resolver currently hydrates selected bindings eagerly, and ordinary fragment
expansion can clone evidence. Reduced stored bytes across generations do not
establish bounded memory, faster commands or support for an arbitrarily large
service estate. Scale measurements use the fixed synthetic fixture in
`fixtures/bindings-storage-scale/spec.json`; they are not customer benchmarks.
