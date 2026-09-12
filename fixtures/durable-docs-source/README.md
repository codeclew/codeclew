# Build-independent documentation fixtures

These synthetic Python, Java and Kotlin 1.9 services intentionally refer to
unavailable dependencies. Source capture and structural documentation must work
without importing Python modules, running Maven/Gradle, or starting K2/javac.
They are documentation inputs, not deployed applications.

The acceptance cases distinguish source occurrences from logical documentation
IDs and compiler symbols. They cover declarations, as-written annotations,
lexical calls, guards, returns, loops, lambdas, Kotlin safe calls/Elvis/`when`,
extension and suspend functions, docstrings, and non-ASCII byte ranges.

Mutation checks must detect helper/configuration/literal/docstring changes and
new or deleted files, including a previously empty query gaining a declaration.
Blank-line relocation must update exact source links without claiming semantic
change. Ambiguous rename/duplicate correspondence and parser errors must remain
explicit. No syntax-only result establishes resolved call targets or callback
execution. Restoring a semantic provider must enrich the same service record.

Java and Kotlin Maven files exist to exercise optional enrichment failures.
The missing policy/notifier types are intentional; baseline documentation must
not try to install them or execute the build. A Java test can restore compilation
by adding a source stub for `example.missing.ExternalPolicy` in its private copy.
