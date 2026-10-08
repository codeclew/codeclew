# Save a diagnostic bundle

Available in Codeclew v0.13.19. Use the installed `clew` launcher, or `./clew`
when developing from the source checkout. Both expose the same switches.

To diagnose an existing documentation Check, save its metadata without another
source capture:

```sh
clew support collect --root /absolute/docs-root \
  --output /absolute/new-support-bundle
```

To select a historical Check and one service, copy the complete handle returned
by `docs check`:

```sh
clew support collect --root /absolute/docs-root \
  --snapshot 'sha256:<exact digest>/<exact size>' --service orders \
  --output /absolute/new-orders-bundle
```

The output must be a **new directory outside the documentation root**. An existing
file, directory or symlink is refused. The parent must already exist; its canonical
location is used. Directory permissions are `0700`; files are `0600`.

`report.json` records the exact snapshot and `manifestBytes`, separate
`checkStatus=CHECKED|UNRESOLVED`, input/context digests, hashed service/revision
identities, known producer/runtime metadata, selection/retention flags, captured
entrypoint counts, typed original failure codes, static remediation identifiers,
and allowlisted worker/Maven process metadata. `status=COMPLETE` describes the
**diagnostic inspection**, and does not mean source capture succeeded. A complete
bundle can describe an `UNRESOLVED` Check with no captured services.

The default latest manifest is read once and pinned by its exact bytes. Collection
reads only that Check manifest and the source-selection envelope. It does not
open source checkouts, run compilers, call doctor, contact a model or network,
hydrate source/fact payloads, copy `.codeclew`, or write store/lock files.
`inspectionScope=MANIFEST_METADATA` and `heavyObjects=NOT_VERIFIED` explicitly
exclude verification of the full snapshot closure. Current declarations are
`NOT_INSPECTED`; a historical Check remains useful after declarations change.
A cold **source launcher** may first build its own runtime; a prepared installed
runtime does not need build tools for collection.

Missing/corrupt metadata, unavailable runtime metadata, active nonempty SQLite
WAL/rollback journals, or concurrent store changes produce `PARTIAL` with typed
issues. Finish the active writer and retry collection into a new directory.
Collection does not checkpoint, repair or ignore an active WAL. It reads at most
8 MiB per selected metadata object. Service reporting is capped at 256 records
and a byte budget; `serviceRecordTruncation` counts omitted records. Select one
service to inspect its metadata without the aggregate limit.

For the **next normal invocation**, including runtime preparation failures:

```sh
clew --diagnostics /absolute/new-run-bundle \
  docs check --root /absolute/docs-root --service orders
```

The command runs once. Its stdout/stderr are streamed through unchanged, and
its exit status is preserved, including nonzero `UNRESOLVED` documentation checks.
Additional diagnostic notifications go to stderr. If diagnostic collection fails,
the original command still runs and retains its result. Existing output paths
are not overwritten. Only the exact Check returned by this invocation is inspected;
a failure before saving does not follow an older latest Check. A canonical snapshot
identity attached to a reader error is recorded as error context without recapture.
Invocation metadata includes UTC start/end, elapsed milliseconds, and a static
command name such as `docs/check`; argument values are not recorded.

stdin remains inherited. In this opt-in mode the child sees pipes for stdout and
stderr, so terminal detection, colors or progress presentation may differ.
SIGINT, SIGTERM and SIGHUP are forwarded to the child process group; interruption
escalates termination even after both output pipes close. Help/version or argument
errors can produce a partial package without core metadata.

## Private logs

Default diagnostic artifacts contain no raw output, arbitrary error text,
arguments, environment values, URLs, absolute paths, symbols or source. Original
command output itself keeps its existing contract and is not assumed shareable.
Content digests and hashed identities can still correlate repositories: review
`report.json` and `manifest.json` before sending the bundle. No upload occurs.

Opt in to private output when reproducing an error:

```sh
clew --diagnostics /absolute/new-private-run --include-private-logs \
  docs check --root /absolute/docs-root --service orders
```

The package is marked `PRIVATE_REVIEW_REQUIRED`. `PRIVATE_stdout.tail` and
`PRIVATE_stderr.tail` retain at most 1 MiB per stream. Observed, retained and
dropped byte counts distinguish truncation from no output. A live compiler worker
failure also copies its already bounded 64 KiB stderr tail directly into the
wrapper-owned `private/` directory. For `docs check`, the same run uses the existing
Maven `--debug-output` capture in that directory unless the caller already supplied
an external debug output. External log paths are never imported. Worker and Maven
artifacts share a 2 MiB/128-file budget; exhaustion reports unavailable capture.

`PRIVATE_SAVED_DETAILS.json` contains a bounded tail of original saved failure
details and can contain private paths or other sensitive text. A selected service
scope also limits these saved details. It does not read files named in historical
errors: post-hoc `support collect --include-private-logs` can export saved error
text, but cannot recover original worker/Maven log files. Reproduce once with the
wrapper to retain live logs. Existing private worker state capture is preserved.

Each artifact is bounded by 1 MiB. Standalone bundles have a 4 MiB total budget;
invocation bundles have an 8 MiB total budget. Artifacts use relative names and
content digests; `manifest.json` is atomically finalized last. A directory without
a finalized manifest is an incomplete collection, not a complete bundle.

The narrower [support summary workflow](p0-runbook.md) remains available for a
single caller-owned JSON artifact. [Maven diagnostics](maven-build-failure-diagnostics.md)
describe the underlying private stage captures.
