# Kotlin 1.9 analysis option qualification

The Rust admission gate and Kotlin worker allow these exact options for project
compilers **1.9.24 and 1.9.25**, language/API **1.9**, JVM target **17**, analyzed
by **2.4.10**:

| Option | Qualified values | Compiler evidence |
| --- | --- | --- |
| `-Xjsr305` | `strict`, `warn`, `ignore` | A Java `@ParametersAreNonnullByDefault` API called with `null` produces an error, a warning, or no warning respectively. |
| `-Xjvm-default` | `disable`, `all`, `all-compatibility` | Inherited interface dispatch returns the same value; interface default methods and callable `DefaultImpls` bridges match each mode. |

Use the single-token `-Xname=value` spelling, with one distinct value per option.
The arguments are forwarded unchanged. Special JSR-305 annotation rules,
`under-migration` settings, conflicting values, old JVM-default spellings,
other compiler patches and JVM targets remain unqualified. Existing exact
compiler routes and the separately qualified annotation-default-target option
retain their existing behavior.

This is compatible analysis, not native compiler ABI equivalence. Analysis still
uses language/API 2.0 for 1.9 projects, records those boundaries, and disables BTA
eligibility. The native application arguments are not changed. No claim is made
about all Kotlin 1.9 programs or arbitrary interactions with compiler plugins.

`Kotlin19OptionQualificationTest` launches real compiler JVMs on JDK 21: the
1.9.24 and 1.9.25 compiler dependencies are isolated from the 2.4.10 worker.
It runs nine JSR-305 cases and nine JVM-default cases, checking diagnostics,
reflection, runtime dispatch and compatibility bridge invocation. Rust and
worker tests additionally check admission, argument preservation and refusal
outside the qualified scope. These checks are included in `scripts/ci-verify.sh`.

```sh
cargo test --locked -p clew --lib kotlin_engine::tests:: -- --test-threads=1
./gradlew --no-daemon :workers:kotlin:test \
  --tests dev.semanticthread.worker.KotlinEngineCompatibilityTest \
  --tests dev.semanticthread.worker.Kotlin19OptionQualificationTest
```

The reported private projects were unavailable during this qualification. Their
exact patch versions and option values still need confirmation before claiming
that their admission failures are resolved. The qualified values above are a
public reproduction matrix, not recovered private project configuration.

References: [JSR-305 semantics](https://kotlinlang.org/docs/java-interop.html#jsr-305-support)
and [JVM-default mode mapping](https://kotlinlang.org/docs/gradle-compiler-options.html#migrate-freecompilerargs).
The qualification is based on executed compilers, not solely their documentation.
