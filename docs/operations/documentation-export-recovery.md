# Recover selected captures from an offline export

Use export recovery when the original documentation store is unavailable but a
stopped portable object-store export and its saved Check snapshot are available.
The export is an immutable input: keep its source store stopped while exporting,
and do not recover directly from a live SQLite database or one with a nonempty
WAL or journal.

Initialize an empty destination first, then name the exact source snapshot and
the capture-manifest basenames to recover:

```sh
clew docs snapshot recover --root docs \
  --from-export /recovery/docs-export \
  --source-snapshot "$snapshot" \
  --capture orders-cache-key.json \
  --capture inventory-cache-key.json
```

The snapshot handle must come from the same export. Capture names are basenames
under its `cache/` directory; select each service at most once. The command
loads only the selected service declarations and capture closures. It does not
restore old entities, scenarios, notes, Work, proposals, publications, bindings,
or the export's latest-check pointer.

The result reports the recovered `snapshot`, selected services and captures,
and the count and byte total of unique selected-source references accounted
for by the import, including metadata and provenance. These figures are not a
physical SQLite row count or on-disk size. Its source authority is
`RETAINED_SOURCE_NOT_REVERIFIED`: recovery validates the saved object closure
and preserves the producer's cacheability and reason, but does not check out
repositories, rerun analyzers, or establish current source freshness.

The destination must be an initialized empty documentation root, or the exact
destination of an earlier import with the same selection. Repeating that import
is idempotent; a different selection or unrelated destination content is a
conflict. The command does not publish a reader or change `latest-check.json`.
