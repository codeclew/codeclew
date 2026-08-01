# Maven end-to-end implementation plan

Design: `docs/superpowers/specs/2026-08-01-maven-end-to-end-design.md`

## 1. Establish the Maven red test

Files:

- add `fixtures/kotlin-maven/pom.xml`
- add Maven fixture main and test Kotlin sources
- add `crates/sthread/tests/maven.rs`

The fixture is a single-module JDK 21 project pinned to Kotlin 2.3.0. The first integration test opens `:/main` and asserts `buildSystem=MAVEN`, exact compiler/worker versions, source roots, classpath, compile goal, and test goal. Run only that test and confirm it fails because the worker still requires Gradle.

## 2. Add exact Kotlin 2.3 worker selection

Files:

- add `workers/kotlin23/build.gradle.kts`
- add a Kotlin 2.3 FIR adapter only if the shared 2.4 adapter does not compile
- update `settings.gradle.kts`
- update `crates/sthread/src/worker.rs`
- extend `crates/sthread/tests/maven.rs`

Add a `Kotlin23` worker variant using compiler 2.3.0. Keep exact-line selection: 2.1 → 2.1 worker, 2.3 → 2.3 worker, 2.4 → 2.4 worker; other lines fail closed. Run the narrow test to observe the next Maven-model failure before implementing extraction.

## 3. Extract and cache the Maven project model

Files:

- add focused project-model helpers under `workers/kotlin/src/main/kotlin/dev/semanticthread/worker/`
- update `Worker.kt`
- extend `crates/sthread/tests/maven.rs`

Introduce build-system detection and a Maven extractor. Prefer `./mvnw`, then `mvn`. In one bounded Maven invocation, write the effective POM and dependency classpath to temporary files. Parse compiler version/options, source roots, dependencies, plugin configuration, Java target, and test plan. Resolve requested Kotlin compiler-plugin artifacts and translate supported plugin options. Include `pom.xml`, `.mvn`, wrapper inputs, source inventory, and normalized artifacts in cache/fingerprint material.

Run the Maven inspection test until green, then add and pass semantic index/resolve assertions. Re-run the existing Kotlin 2.1 and golden Gradle tests to prove worker selection and Gradle extraction remain unchanged.

## 4. Make agent context build-system neutral

Files:

- update `crates/sthread/src/agent_context.rs`
- extend `crates/sthread/tests/agent_context.rs` or `maven.rs`

Add a failing assertion that Maven context contains `mvn -Dtest=<Class> test` and never suggests `gradlew`. Render targeted commands from `buildSystem`; preserve existing Gradle output byte-for-byte. Run both Maven and Gradle agent-context tests.

## 5. Make snapshots and transaction validation build-system neutral

Files:

- update `crates/sthread/src/model.rs`
- update snapshot construction in `crates/sthread/src/main.rs` and benchmark example
- update `crates/sthread/src/transaction.rs`
- update transaction fixtures/tests

Add backward-compatible `buildSystem` to `Snapshot`. First add a transaction regression test that changes the Maven fixture, commits through the semantic transaction path, and requires Maven compile plus the configured test goal. Confirm it fails at the Gradle launcher. Refactor worktree validation to resolve and execute the correct launcher and arguments, record neutral build evidence, and preserve existing Gradle behavior.

Run the Maven transaction test, concurrency matrix, and vertical transaction checks.

## 6. Verify the full Maven vertical on product-repo

Commands:

- build/install all three workers
- run Maven fixture tests
- run full Rust and worker test suites
- run `project inspect`, semantic `index`, and bounded `agent-context` on the historical `product-repo` baseline
- run the repository's narrow oracle test command on the oracle snapshot

The smoke run must select Kotlin 2.3.0, index the target sources with K2 validation, expose Maven validation commands, and resolve the archive/changefeed edit surface without modifying the private-product repository.

Commit the implementation and verification documentation only after fresh verification output is available.

## 7. Create a leak-free benchmark seed

Export revision `56d42d5f` from the source repository, initialize a new single-commit Git repository, and install a hidden acceptance patch outside all agent worktrees. Create three worktrees from the same seed and verify identical HEAD, clean status, dependencies, and test baseline.

Do not expose the historical oracle hash, source repository remotes, method labels, or another agent's output in benchmark prompts.

## 8. Run the three Terra approaches

Run three independent `gpt-5.6-terra` agents with medium reasoning:

1. default tools;
2. ast-indexer;
3. sthreads with one bounded `agent-context` navigation call.

Require one focused commit from each. Capture timestamps and cumulative token counters from tool evidence, not agent self-report. Record commands before edit, total tool calls, navigation stdout, time to edit, time to commit, raw tokens, noncached tokens, tests, and commit.

## 9. Independent acceptance and report

Give a fresh independent agent anonymized patches A/B/C, the task, baseline, hidden acceptance tests, and verification output. Require a verdict for behavior, type safety, absence of N+1 queries, preservation of batching and CREATE/UPDATE behavior, test quality, and scope.

Write a machine-readable report and a concise experiment document. State whether sthreads wins default and ast-indexer on the complete workflow, keep literal lookup/index construction timings separate, and avoid general statistical claims from one run per method. Commit the report and leave all source repositories clean except explicitly documented benchmark worktrees.
