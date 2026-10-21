This owned fixture uses Kotlin 2.3.21, Gradle 9.6.1 and a JDK 25 project toolchain.
The main Java source set registers a generator task as its source provider;
`compileKotlin` therefore resolves the generated Java type on a clean checkout.

Run `./gradlew compileKotlin`. The build writes separate generator and selected
compiler markers under `build/`. Model extraction must run the generator while
leaving the selected Kotlin compiler marker absent. Use separate clean checkouts
for native compilation and analyzer capture, so native class files cannot mask
missing analysis inputs.

`input.txt` controls the generated API: `baseline` produces a String-returning
method, `number` changes its return type to int, and `missing` removes the Java
file. The latter two inputs intentionally invalidate the unchanged Kotlin caller;
compiler capture must fail rather than reuse the earlier successful analysis.
