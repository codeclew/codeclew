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
  --output "$NEW_OUTPUT_DIRECTORY" --expect abi-boundary
```

The harness uses the repository's Gradle wrapper and makes separate clean
checkouts for native compilation and Clew Check. It requires both native
generated calls. The current boundary is `COMPILER_ROW` with
`KAPT_NATIVE_AUTHORITY_REQUIRED`, retained by `support collect`.

Run with `--expect captured` when qualifying this row. That mode requires a
successful Check and both compiler-exact calls in the export for the same
revision, compilation and tracked source ranges.
