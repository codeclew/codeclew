# Kotlin analysis option qualification

The Rust admission gate and Kotlin worker qualify these exact option values for
the packaged **2.4.10 analyzer**, independently of the project's patch version.
Project/compiler compatibility, language/API modes, JVM targets and plugins are
checked separately by the existing admission and compiler paths. Option support
does not introduce another project-version or JVM-target whitelist.

| Option | Qualified values | Compiler evidence |
| --- | --- | --- |
| `-Xjsr305` | `strict`, `warn`, `ignore` | A Java `@ParametersAreNonnullByDefault` API called with `null` produces an error, a warning, or no warning respectively. |
| `-Xjvm-default` | `disable`, `all`, `all-compatibility` | Inherited interface dispatch returns the same value; interface default methods and callable `DefaultImpls` bridges match each mode. |
| `-Xconsistent-data-class-copy-visibility` | Presence, for project Kotlin 2.0.20 or newer | Generated `copy` has the private constructor's visibility; external copy calls fail under both native and analyzer compilers. |
| `-Xjvm-enable-preview` | Presence | JVM 25 output retains major 69 and minor 65535; JDK 25 accepts it with `--enable-preview` and refuses it without that runtime flag. |

Use the single-token `-Xname=value` spelling for valued options and the exact
bare spelling for presence options, with one distinct value per option.
The arguments are forwarded unchanged. Special JSR-305 annotation rules,
`under-migration` settings, conflicting values and old JVM-default spellings
remain unqualified. Existing exact
compiler routes and the separately qualified annotation-default-target option
retain their existing behavior.

This is compatible analysis, not native compiler ABI equivalence. Analysis still
uses language/API 2.0 for 1.9 projects, records those boundaries, and disables BTA
eligibility. The native application arguments are not changed. No claim is made
about all Kotlin 1.9 programs or arbitrary interactions with compiler plugins.

`Kotlin19OptionQualificationTest` launches real compiler JVMs on JDK 21. Its
sample covers 1.9.0, 1.9.24, 1.9.25, 2.0.21 and 2.4.10 with JVM target 17, plus
the analyzer with JVM targets 1.8 and 21. The oracle compiler dependencies are
isolated from the worker. The 21 JSR-305 and 21 JVM-default cases check diagnostics,
reflection, runtime dispatch and compatibility bridge invocation. These are
regression samples, not an exhaustive proof for every source compiler version.
Both flags are present in each invocation: the JSR-305 cases retain
`-Xjvm-default=all-compatibility`, and the JVM-default cases retain
`-Xjsr305=strict`.
Rust and worker tests additionally check accepted values across project patches
and JVM targets, argument preservation, conflicting/unknown option rejection,
and continued refusal of unsupported project language/compiler modes. These
checks are included in `scripts/ci-verify.sh`.

`KotlinJvm25QualificationTest` compares the native 2.3.21 compiler and packaged
2.4.10 analyzer with each required presence option separately and both together
at JVM target 25. The compiler plugin must retain exact FIR receipts and JDK API
references; compiler success alone does not establish accepted facts. A real
Gradle project uses toolchain 25 while the Gradle daemon and analyzer stay on JDK
21. Its options and target are preserved. The model selects the effective Kotlin
task toolchain before the Java launcher fallback. Toolchain resolution errors do
not silently substitute the build JVM; legacy JDK layouts remain supported.

Full verification and the release qualification job require an installed JDK 25
at `CODECLEW_TEST_JDK25_HOME`, in addition to build/analyzer JDK 21. The CI and
release workflows provision both. This JDK is a test dependency, not a change to
the packaged analyzer JVM or the application's settings.

```sh
cargo test --locked -p clew --lib kotlin_engine::tests:: -- --test-threads=1
./gradlew --no-daemon :workers:kotlin:test \
  --tests dev.semanticthread.worker.KotlinEngineCompatibilityTest \
  --tests dev.semanticthread.worker.Kotlin19OptionQualificationTest \
  --tests dev.semanticthread.worker.KotlinJvm25QualificationTest
```

The reported private projects were unavailable during this qualification.
The supported option policy does not require recovering their exact patch
versions. Their full project admission and analysis still need a real run;
the public matrix is not a reproduction of their private configuration.

References: [JSR-305 semantics](https://kotlinlang.org/docs/java-interop.html#jsr-305-support)
and [JVM-default mode mapping](https://kotlinlang.org/docs/gradle-compiler-options.html#migrate-freecompilerargs).
The qualification is based on executed compilers, not solely their documentation.
