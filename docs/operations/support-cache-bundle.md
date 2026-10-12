# Private support-cache bundle

`clew support collect` copies docs-local cache into a private offline bundle outside docs root.

```sh
clew support collect --root /path/to/docs-root --include-cache --work 'PASTE_64_HEX_WORK_ID' --output /path/to/new-bundle
```

Paste the saved Work's 64-hex ID. `--include-cache` is required; default remains metadata-only. `--work` selects its immutable snapshot and conflicts with `--snapshot`; otherwise use `--snapshot` or latest. `--cache-max-bytes`: 268435456 bytes default (256 MiB), max 2 GiB. `--service` only filters report metadata; copied cache remains whole.

`PRIVATE_CACHE` is a relocated docs root with `.codeclew/cache` (SQLite, stable raw WAL/journal, manifests/pins), docs manifest/catalog, selected Work receipts, latest job's selected checkpoints. Frozen invocation `job-inputs`/`job-results`/`job-model-inputs`/`job-model-wire-results` JSON, fixed-role `model-ID-map` JSON, known Work/run draft JSON directories, plus selected proposal JSON; inline review stays in saved job. Paths derive from validated IDs; persisted `outputDirectory` is never followed.

```sh
clew docs work read --root BUNDLE/PRIVATE_CACHE --work 'PASTE_64_HEX_WORK_ID' --input selection.json
```

`PRIVATE_SOURCE_CACHE` may hold unsanitized cached fragments/private records, not a fresh repository snapshot. Excludes source bindings, Git history and global `CODECLEW_HOME`. No live-environment inspection, compiler/doctor/model/network actions, or upload. `COPIED_NOT_HYDRATED` leaves semantic/cache-object closure unverified; corruption is preserved and reports may be `PARTIAL`.
