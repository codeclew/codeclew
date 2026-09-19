# Source-scope lifecycle obligation map (2026-09-17)

This run (`20260917-source-scope-lifecycle`) is the retry of source-authority
closure. It owns the source/profile/processor and temporary-disposal obligations
below. Other obligations from the practical documentation audit are assigned to
their successor runs and are **not** claimed here.

## Owned by this run (`20260917-source-scope-lifecycle`)

- Correct per-compilation indexed/persisted source bytes survive capture and
  reopen; identical relative paths with different contents stay distinct.
- Processor/profile authority is carried per-service/per-compilation and binds
  the compiler index, persisted transformed files, descriptors and reuse checks
  to a canonical source-state manifest.
- Native annotation-processor admission (effective-POM and per-service
  `annotationProcessorPaths`) is qualified with genuine offline compiler
  execution; unadmitted processor widening is rejected and compiled classes are
  preserved.
- Owned sealed attempts are disposed safely on success and on failure paths with
  observable errors, without following symlinks, touching retained CAS or
  changing borrowed/user repositories, and without scan-and-delete of history.
- The strict supplemental acceptance checker (`scripts/validate_nessy_acceptance.py`)
  and its contract tests live in this repository and are enabled.

## Transferred to `20260917-docs-snapshot-store` (NOT claimed here)

- Actual oversized render/refresh transaction qualification: publish_root
  expected-root compare-and-swap, review_portable_limits oversized publication,
  and cache-normalization fact/index integration.
- Replacing whole-map sources/observations/dependencies storage with granular
  indexed pages and bounded reads.
- Removing inline heavy copies from bindings on publication.
- Consuming a pinned immutable snapshot without a new build as snapshot evidence.

## Explicitly out of scope for all product runs here

- Customer repository builds, mutation, migration, cleanup or publication.
- Historical cache deletion or private CODECLEW_HOME edits.
- Remote publication, network access, package publishing or CI/CD changes.

## Fixture provenance (native tests)

Native Maven/Lombok tests use a test-owned offline repository built by
hard-linking curated artifacts from the ambient read-only `~/.m2/repository`,
with remote markers stripped and plugin-group/per-artifact metadata written so
Maven resolves unpinned plugin versions offline. JDK 21 is used for fixture
qualification; the live customer issue was observed on JDK 17 and fixture
qualification does not prove a live JDK 17 run.