# Source Spring declaration qualification

The Java and Kotlin fixtures describe the same `POST /orders/reserve` endpoint.
Kotlin uses an explicit import alias. `docsys_t06_*` runs them with Git available
and build/compiler tools absent, checks the separate source authority, and tests
unrelated/ambiguous/local-shadowing annotations, unresolved constants, conflicting
aliases, inheritance and runtime expressions. Shared-rule tests cover defaults,
unavailable composed annotations and wrong request-method enum types.

| Configuration | Observed checks | Limit |
| --- | --- | --- |
| Spring Boot 3.3.0 managed Spring MVC 6.1.8; Java 17 bytecode; test JVM 21 | `boot33` MockMvc accepts POST with a quantity and returns its value; GET returns 405 | Standalone controller registration; no application auto-configuration or deployment claim |
| Java and Kotlin 1.9 declaration syntax, no compiler/framework runtime | Same derived route; Kotlin import alias and array arguments; explicit source/import authority | Names are not compiler-resolved; controller activation remains unproven |
| Existing Kotlin 1.9.25 / Spring Web 6.1.2 compiler fixture | T05 native evidence, HTTP/Kafka facts and source enrichment passed | Separate compiler qualification; no Boot version inference; language-upgrade boundary retained |
| Other Boot 3.3 patches and later Boot releases | Not qualified in this slice | Re-run the exact dependency/runtime fixture before extending this matrix |

```sh
cargo test --locked -p clew --test documentation_system docsys_t06_ -- --test-threads=1
mvn -q -f fixtures/documentation-system/spring/boot33/pom.xml test
```

Use JDK 21 for the recorded Maven check; the fixture targets Java 17. The tests
register the Java controller explicitly, so they do not prove arbitrary component
scanning, security filters, profiles, proxy behavior or production registration.
The duplicate Java file under `boot33` is the exact runtime test input.

`source-annotation-facts/1.0` is an additive Rust source contract. The existing
`jvm-annotation-facts/1.0` compiler schema and Kotlin worker bridges are unchanged.
Source facts cannot claim compiler authority, binary origins, resolved inheritance
or class/nested annotation identities. Fully qualified spelling or a matching
explicit import supplies name qualification. Wildcard/ambiguous/local-shadowed
names remain unresolved; arbitrary source-defined annotation composition and
member defaults remain gaps. Framework rules consume normalized facts once in
Rust for both languages. No source annotation executes configuration or tools.
