# Native all-open annotation regression

This fixture uses Kotlin 2.4.20, Gradle 9.6.1, JDK 21, and an explicit all-open
annotation option. The base class and override require the option to compile.
It exercises both a superclass call and a call through the base type.

```sh
python3 -I -S scripts/check_kotlin_allopen_24.py \
  --clew "$CLEW" --jdk21 "$JAVA_HOME_21" --output "$NEW_OUTPUT_DIRECTORY"
```

The harness compares the retained witness with a separate native build,
checks both compiler-exact source spans, and verifies that removing the
option makes the native build fail.
