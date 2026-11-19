# Combined kapt and compileJava regression

The Gradle 9.4.1 fixture uses Kotlin 2.4.10, the Spring preset, a kapt
processor, and an independent Java source producer reached only from
`compileJava`. The Kotlin callers resolve the two generated types separately.
The compiler toolchain and the bytecode target are JVM 25.

Run the focused check from the repository root with a built package launcher
and the homes of JDK 21 and JDK 25.

```sh
python3 -I -S scripts/check_kapt_combined_24.py \
  --clew "$CLEW" --jdk21 "$JAVA_HOME_21" --jdk25 "$JAVA_HOME_25" \
  --output "$NEW_OUTPUT_DIRECTORY"
```

The harness uses Gradle with JDK 25 and runs the worker and annotation-processor
project on JDK 21. It makes separate clean checkouts for native compilation and
Clew Check. It requires both native generated calls, the Spring effect, and
both compiler-exact calls in the export. It also checks the native compilation
witness against the independent build.

For the pre-qualification package, `--expect abi-boundary` checks the historical
`COMPILER_ROW / KAPT_NATIVE_AUTHORITY_REQUIRED` boundary and its saved support
report.
