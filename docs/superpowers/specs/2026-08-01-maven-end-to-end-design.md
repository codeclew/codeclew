# Maven end-to-end support and product-repo benchmark

Date: 2026-08-01

## Objective

Make Maven a first-class build backend for the supported Kotlin/JVM vertical, then compare default tools, ast-indexer, and `sthread agent-context` on the same realistic Spring task from `product-repo`.

The Maven path is complete only when project inspection, worker selection, semantic indexing, agent context, candidate compilation, configured tests, and transaction commit all use the repository's Maven model and launcher. A source-only fallback is not acceptable because it would make semantic results incomplete while presenting them as validated.

## Supported boundary

The first Maven vertical supports a single-module Kotlin/JVM repository with `pom.xml`, JDK 21, main and test Kotlin source sets, Maven compiler dependencies, and Kotlin compiler plugins. `./mvnw` is preferred when present; otherwise the worker uses `mvn` from `PATH`.

Multi-module Maven, Android, KMP, and mixed build ownership remain explicit unsupported configurations. Gradle behavior and schemas remain backward compatible.

`product-repo` uses Kotlin 2.3.0, so the implementation adds a version-pinned Kotlin 2.3 worker alongside the existing 2.1.21 and 2.4.10 workers. Worker selection must never silently analyze 2.3 sources with a different compiler line.

## Architecture

### Project model extraction

The Kotlin worker detects the build system and delegates to one of two bounded extractors:

- Gradle keeps the existing init-script extraction.
- Maven runs the effective-model and dependency-classpath goals through the selected Maven launcher, writes their results to temporary files, and parses the resolved model.

Both extractors produce the same internal compilation model: module, source set, source files, compile classpath, friend paths, compiler version and options, compiler plugins, JDK home, build metadata, and a validation build plan.

The public project model gains an explicit `buildSystem` and build plan. Maven model inputs include `pom.xml`, `.mvn` configuration, wrapper files when present, effective compiler settings, resolved classpath fingerprints, and compiler-plugin artifacts. Cache keys therefore invalidate when Maven configuration or resolved compilation inputs change.

### Build plan

Build execution is represented as data rather than a Gradle task assumption:

- build system;
- launcher policy;
- compile arguments;
- default test arguments;
- targeted-test command template.

The Rust transaction validator resolves the launcher inside the detached worktree and executes the stored compile and test plans. Gradle continues to use `./gradlew`; Maven uses `./mvnw` or `mvn`. Diagnostics and duration evidence identify the actual build system and command.

`agent-context` renders validation commands from the same build plan. Maven test suggestions use `mvn -Dtest=<ClassName> test`; Gradle suggestions retain the existing `cleanTest --tests` form.

### Compiler plugins

The Maven extractor reads Kotlin plugin configuration and dependencies from the effective POM. It resolves compiler-plugin jars from the local Maven repository after Maven model extraction and translates the supported Kotlin Maven plugin presets/options into direct compiler arguments. Missing requested plugin artifacts fail closed as an unsupported project configuration.

## Error handling

The adapter returns `UNSUPPORTED_PROJECT_CONFIGURATION` for:

- neither Maven wrapper nor `mvn` being executable;
- Maven model extraction failure or missing output;
- unsupported Kotlin compiler line;
- multi-module Maven input;
- missing source roots, classpath entries required by the effective model, or requested compiler plugins.

Error evidence includes bounded Maven output without exposing environment variables or credentials. Temporary model files are always removed.

## Test strategy

Implementation follows red-green-refactor cycles around a committed Maven Kotlin 2.3 fixture.

Required automated evidence:

1. Maven project inspection returns normalized source roots, compiler settings, classpath, build system, build plan, and deterministic fingerprints.
2. Kotlin 2.3 selects the exact worker and resolves symbols semantically.
3. `agent-context` produces bounded useful context and Maven validation commands.
4. A candidate transaction compiles and runs configured Maven tests inside an isolated worktree.
5. Maven model changes invalidate snapshots.
6. Existing Kotlin 2.1, Kotlin 2.4, Gradle vertical, concurrency, and transaction tests remain green.
7. A read-only smoke run indexes the historical `product-repo` baseline and resolves the archive/changefeed chain.

## Benchmark design

The seed is an archive of `product-repo` revision `56d42d5f`, imported into a fresh repository with one baseline commit. The historical oracle `f7c14921` is not reachable from benchmark worktrees or agent context.

Task statement:

> When a non-test product is archived, its `products-changefeed` event with `modification=DELETED` must retain `productId` and include a typed `entity` containing `id`, `code`, and `title`. Do not introduce N+1 queries; preserve batch archiving and CREATE/UPDATE payload behavior. Add regression coverage with human-readable Russian `@DisplayName` annotations.

Three independent `gpt-5.6-terra` agents at medium reasoning use separate worktrees and the same prompt:

1. default repository tools;
2. ast-indexer;
3. one bounded `sthread agent-context` for navigation.

Each run must commit its change and record time to first edit, time to commit, commands before edit, total tool calls, navigation stdout, raw tokens, noncached tokens, tests, and commit SHA. A separate independent agent receives only the task, baseline, patches, and test evidence and judges correctness, scope, batching, and regressions without being told which method produced each patch.

The report distinguishes primitive lookup speed from complete agent workflow efficiency. A single run per approach is treated as a head-to-head experiment, not a statistical population; close or inconsistent results require a repeated series before claiming a general winner.
