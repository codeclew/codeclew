# Maven build failure diagnostics

Java Maven model extraction invokes three stages from the repository root:

1. Effective POM: selected module POM, `-B -q -N help:effective-pom`, temporary output file.
2. Compilation/classpath: root POM, `-pl <module> -am` for submodules,
   `compile` or `test-compile`, and `dependency:build-classpath`.
3. Release: selected module POM, `-q -DforceStdout help:evaluate -Dexpression=maven.compiler.release`.

All stages inherit the process environment, use the repository wrapper when
present (supported non-executable shell wrappers run through their interpreter),
and share the pinned Maven settings and build properties. Reproduce with the
same JDK, PATH, wrapper, settings, selector and cwd. Dependency resolution and
release evaluation alone do not reproduce compilation or effective-POM failures.

Previously the diagnostic recommended only dependency resolution and release
evaluation. A public fixture demonstrates that this command exits zero even when
`compile` exits one because a Java type is missing. The previous implementation
also accumulated both entire streams in memory and rejected combined output above
4 MiB, including successful builds whose model was written to files. That case
was `BUILD_MODEL_OUTPUT_LIMIT`, **not** `BUILD_COMMAND_FAILED`.

Build logs are now drained concurrently with bounded memory. File-output stages
retain neither stream; stdout-based model extraction retains at most 4 MiB of
stdout, and stderr is only counted. The child is waited for and both pipes reach
EOF. No JVM-worker frame protocol participates in these Maven subprocess calls.

`clew doctor task` is admission/preflight only. It validates the selected runtime,
repository, profile, wrapper and settings eligibility, but does **not** launch Maven.
`clew docs check --root <docs-root> --service <service>` opens the source session
and runs the Maven stages above.

For a deliberately reproduced failure, opt in to local raw-tail capture with an
existing private directory:

```sh
mkdir -m 700 /absolute/private/codeclew-maven-debug
clew docs check --root /absolute/docs-root --service <service> \
  --debug-output /absolute/private/codeclew-maven-debug
```

`--debug-output` accepts only a normalized absolute, non-symlink directory owned
by the caller with mode exactly `0700`. Capture is disabled by default, cannot be
enabled through an environment variable, and is not available on `doctor task`.
On a failed Maven stage, Codeclew writes fresh opaque `.stdout` and `.stderr`
artifacts plus a manifest, all mode `0600`. Each artifact contains only the final
64 KiB tail; the manifest records the stage, exit/signal, byte counts, truncation,
and opaque artifact basenames. A manifest is written last, so it exists only after
both tails were saved. If artifact writing fails, the Maven failure is unchanged
and capture is reported as unavailable. Successful Maven stages create no artifacts.

Treat these files as private: they can contain credentials, source paths, repository
URLs, environment-derived values, or application data. Do not attach them to an
issue, evidence package, or chat. Inspect them locally and delete the private
output directory when it is no longer needed.

Normal diagnostics omit raw output, environment values, settings, paths and
arguments that could contain private data. Safe summaries include only the stage,
capture status, exit status or signal, and separate stdout/stderr byte counts:

- `BUILD_COMMAND_FAILED`: observed nonzero status, even with oversized output.
- `BUILD_MODEL_OUTPUT_LIMIT`: successful process, stdout model too large.
- `BUILD_MODEL_OUTPUT_ENCODING`: successful process, invalid UTF-8 stdout model.
- Launcher, wait and output-read failures have distinct reasons.

Limits on effective-POM payloads, classpath entry counts and individual dependency
sizes still apply. Build log size, classpath text size and JAR content size are
separate quantities; summing JAR sizes does not measure process output or framing.

## Public regression evidence

On JDK 21, Maven compiling Java release 17 through a non-executable wrapper:

| Quantity | Bytes |
| --- | ---: |
| Injected compilation stdout | 5,000,000 |
| Injected compilation stderr | 6,000,000 |
| Classpath text in this run (path length varies) | 83 |
| Dependency JAR contents | 19,936 |

The successful build is accepted, including binary logs. Introducing a Java
compilation error yields the compilation stage and exit status 1 without leaking
the source diagnostic. Separate subprocess regressions cover exit 7, signal
termination, oversized stdout models, large stderr and invalid model encoding.

Run under JDK 21:

```sh
cargo test --locked -p clew --lib java_project_model::tests:: -- --test-threads=1
cargo test --locked -p clew --lib \
  java_project_model::tests::maven_large_logs_and_native_compile_failure_are_distinguished \
  -- --exact --ignored --nocapture --test-threads=1
```

Both are included in `scripts/ci-verify.sh`. The private reported project and
original logs were unavailable. Its root cause remains unconfirmed; this public
fixture does not establish that its dependencies or output size caused the
failure. Obtain the original stage, process status/counts, selected compilation,
launcher/JDK/cwd and exact manual command before making that claim. Never include
credentials or private Maven settings in a report.

## Source and processor authority boundaries

A Maven build can rewrite source in place and run annotation processors. The
`java-17plus-maven-writable-then-seal` profile allows this, so the authority for
every emitted source anchor is tied to the exact bytes the analyzer indexed, not
the original commit snapshot:

- **Read-only profiles** (`java-17plus-maven-read-only`, gradle variants) never
  admit arbitrary annotation processors and index the original snapshot. Sources
  are labelled `EXACT_SNAPSHOT_TEXT` with an original-commit link.
- **Writable-then-seal** persists the transformed source bytes as immutable
  generation authority (`codeclew-transformed-source/1.0`) together with the
  source-state/provenance marker. Documentation reads those persisted bytes; it
  does not slice the original snapshot with transformed line numbers. Transformed
  sources are labelled `TRANSFORMED_SOURCE` and carry no original-commit link.
- **Processors** run only when explicitly on the allowlist and are isolated to a
  disposable generated root; their output is never written into the repository.
- **Generation identity** includes the writable-then-seal selection, so a no-op
  transformed run stays distinct from a read-only run and the two do not reuse
  each other's authority.

Unsupported or ambiguous configurations (for example a transform whose emitted
bytes cannot be attributed) are surfaced explicitly as boundaries and partial
evidence rather than silently presented as exact original source. Legacy
read-only generations remain readable.

## Native processor admission and source/profile authority (2026-09-16 closure)

Processor selection and source authority are exercised against the real product
paths, not marker assertions.

- **Processor selection from the effective POM.** Model extraction parses the
  Maven compiler plugin's `annotationProcessorPaths` and `annotationProcessors`
  from the effective POM. Explicit processor paths join the analyzer classpath as
  content-digest authority, and explicit processor names join the allowlist
  (preserving ordered `-processor`/`-processor:` value pairs). A processor that
  cannot be resolved in the local repository is an explicit
  `UnsupportedProjectConfiguration` boundary, never a silent widening. The
  processor authority participates in the model digest and therefore in the
  derived-manifest and generation identity.
- **Native Maven/Lombok admission.** A real offline Maven + Lombok 1.18.38
  fixture is admitted with the explicit processor path/name and the analyzer
  resolves the generated `getName`/`getAge` members on JDK 21. Processor output
  is isolated to a disposable generated root and never written into the
  repository.
- **Semantic identity distinguishes writable from read-only.** The generation key
  (schema `codeclew-generation-key/2.3`) hashes the writable-then-seal selection,
  so a no-op writable run of the same model/classpath has distinct authority from
  a read-only run and the two cannot reuse each other. The writable gate derives
  from the committed-context profile or the working-tree binding profile id, with
  negative reuse (read-only derives no writable gate) and source isolation
  (working-tree sessions do not carry a committed profile).
- **Compilation-scoped source authority.** Transformed-source authority is
  per-compilation, not flattened: distinct module source states assemble into a
  set and reopen without a same-path collision, and each consumer selects the
  authority of its own compilation. Singleton and legacy read-only records remain
  valid.
- **Lifecycle and cleanup guarantees.** The writable-then-seal lifecycle seals
  the materialized tree read-only *before* indexing, after unmounting derived
  state. The sealed digests are re-checked against the captured transformed
  state; a transform that mutates source after capture is refused. Persisted
  transformed bytes are the exact bytes the analyzer indexed and reopen unchanged.
  If a concurrent owner or failed attempt removes the materialized repository
  between capture and seal, sealing reports a typed `InputMutated` boundary, not
  an internal error, so the attempt is never marked `INTERNAL` with partial
  authority. Documentation labels persisted transformed sources
  `TRANSFORMED_SOURCE` with no original-commit link; read-only sources keep
  `EXACT_SNAPSHOT_TEXT` with their original link.
- **Portable-limit lifecycle.** Output verification separates a typed budget spill
  (`SliceBudgetExceeded`, the shared 128 MiB portable bound) from a
  digest/edited conflict (`WwConflict`) and an unavailable output (`InvalidInput`).
  A valid >64 MiB structured publication survives manifest reload, output
  verification and historical readback; an oversized output fails the budget gate
  and never becomes the current generation.

### Fixture provenance

Enabled regression tests stage an owned offline Maven repository as a hard-linked
clone of the ambient `~/.m2/repository` cache (read-only source, never mutated),
strip `_remote.repositories`/`*.lastUpdated` markers, and add pinned plugin
versions (compiler 3.13.0, resources 3.3.1, clean 3.3.2, surefire 3.3.1, jar
3.4.2, install/deploy 3.1.3) plus plugin-group and per-artifact `maven-metadata`
files so short-prefix offline resolution works. It requires only ordinary cached
artifacts (Lombok 1.18.38 and the Maven lifecycle plugins above) and the pinned
JDK 21. No private settings or private project sources are used.

### Explicit unsupported cases

- The full writable-then-seal end-to-end through the managed CLI binary (session
  open -> real Maven compile -> transform -> seal -> index -> persist -> docs) is
  gated on a provisioned runtime capsule and is not driven end-to-end by these
  integration tests. Coverage for the transform/seal/index/persist/reopen/consume
  chain uses the real CAS persistence, the real analyzer and the real
  documentation consumer on a deterministic transformed workspace, plus the
  native Maven/Lombok admission. A full CLI-pipeline fixture would be required to
  claim otherwise.
- The `status::refresh` and `render::commit_bundle` oversized-publication path is
  coupled to the full checked render pipeline and is not driven directly; the
  portable-limit tests exercise the real verifier/reloader/history code and the
  budget gate that prevents an oversized output from becoming current.
