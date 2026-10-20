# Java/Spring thread completeness regression

Owned synthetic source, not a reproduction of a private project. The independent
oracle was reviewed before product implementation. It expects five direct HTTP
roots and two inherited/composed source surfaces. The excluded other-scope tree
is deliberately not a compilation/source root. Helpers are not public roots.

Copy source/ into a disposable Git checkout, commit it, and capture the same
revision with service-compiler.json and service-syntax.json in separate docs
roots. JDK 21 and real Maven are required for the compiler lane. No application,
external client, database or model is executed. See oracle.json for A1–A6.

Focused contributor verification (from the Codeclew checkout, with JDK 21 and
the fixture's public Maven dependencies available locally):

```sh
MAVEN_ARGS=-o CODECLEW_DOCS_PROGRESS=off cargo test --locked -p clew --lib documentation::thread_context_tests:: -- --include-ignored --test-threads=1
```

This exercises real javac extraction plus portable docs projection and Work,
and committed SOURCE_SYNTAX capture plus Work. It is not a packaged-launcher,
full managed-session, full CI or private-project qualification. Set
`CODECLEW_THREAD_FIXTURE_OUTPUT` to a new local directory to retain the owned
source evidence, inventory and save-thread packets for independent review.
