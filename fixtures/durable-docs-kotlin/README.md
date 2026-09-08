# Kotlin documentation acceptance input

Synthetic Kotlin 1.9.25 / Maven / JVM 17 input for the documentation extractor.
The current semantic engine preserves declared project semantics and reports
analyzer compatibility boundaries. It discovers a Spring HTTP root and a Kafka
listener, retains a negative-stock guard, and resolves an outbound Kafka call.

Run the native compiler acceptance test from the Codeclew root with JDK 21:

```sh
cargo test --locked -p clew --lib documentation::kotlin::tests::native_kotlin_19_maven_documentation -- --ignored --exact --test-threads=1
```

This proves static extraction, not broker delivery or application startup.
The ordinary documentation unit tests independently exercise eight declared
service participants, rejection of a ninth service, and narrative evidence.
