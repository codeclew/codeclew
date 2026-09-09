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

Diagnostics omit raw output, environment values, settings, paths and arguments
that could contain private data. They include the stage, actual exit status or
signal, separate stdout/stderr byte counts, retained stdout bytes and limit:

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
