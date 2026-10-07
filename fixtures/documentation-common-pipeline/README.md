# Paired documentation Pipeline

These public synthetic Java and Kotlin projects describe the same source journey:
an endpoint creates and offers a task, a worker polls a task, checks nullable
inputs and eligibility, selects a name through a helper, transforms it, calls a
gateway interface, and assigns a success or rejection state.

The name helper uses ordinary branches so the supported structure can be
compared without hiding a ternary or Elvis boundary. The original Java native
reader fixture remains unchanged. The Unicode and HTML/MDX-looking literal is
source text and must remain inert in both readers.

Run either project with the repository Gradle wrapper, for example
`./gradlew -p fixtures/documentation-common-pipeline/kotlin compileKotlin`.
A standalone copied project also needs the repository Gradle wrapper files.
Both use JDK 21; the Kotlin project pins Kotlin 2.4.10.

These projects do not observe runtime activation or gateway success. Kotlin
source structure and exact compiler targets do not establish queue identity,
receiver dispatch, state effects or predicate truth. The constructor wiring is
an input for a later qualified wiring capability, not a source-only proof.

`DataPipeline` adds a bounded parameter/local example in both languages: an
input selects a guarded local value, a helper copies it, and the caller returns
the transformed result. Its BOM, CRLF and Unicode text exercise original byte
coordinates. Kotlin also retains shadowed compiler identities, reordered named
arguments and an omitted default argument. Default evaluation, external calls,
property getters, custom nullable operators and varargs remain explicit transfer
frontiers; subsequent values are not promoted past them. This fixture is public
synthetic source, not qualification on a production repository.
