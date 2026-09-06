# Spring computation roots

Goal: enumerate HTTP handlers, Kafka listeners and scheduled jobs from Kotlin
K2 evidence, across every explicitly selected compilation and repository, and
retain exact callable identities that can seed flow and explanation documents.

The framework contract uses stable Spring annotation class identities. It does
not select behavior by the application's Spring Boot patch version. K2 proves
annotation and callable identities; framework rules derive trigger descriptions.
Bean activation, profiles, property placeholders, SpEL, proxy configuration and
programmatic registrations require runtime evidence and must remain explicit.

Required implementation and acceptance:

1. K2 extraction: resolved annotation arguments, constants, import aliases,
   composed annotations and AliasFor, repeats/containers, inherited mappings,
   class-level Kafka listeners and exact overloaded handler symbols. Cover both
   source and dependency annotation declarations, including Spring's real jars.
2. HTTP: RequestMapping and its five method shortcuts, class/method path products,
   unrestricted methods, params/headers/consumes/produces, controller registration
   evidence, interface/base-class mappings and inherited handler implementations.
3. Kafka: method and class listeners, KafkaHandler dispatch, repeats, topics,
   topicPattern, topicPartitions, group/id/container/batch/concurrency metadata.
4. Scheduling: Scheduled/Schedules, cron/zone, fixed delay/rate, string variants,
   initial delay and time unit, disabled cron and repeatable triggers.
5. Public paged catalogue over sealed generations, with repository, revision,
   compilation, source, full symbol and evidence references. Enumerate without
   search-term caps; distinguish missing extraction from an empty catalogue.
   Support one session and all members of a multi-repository thread.
6. Preserve annotation bindings through normalization, validation, incremental
   reuse and CAS. Bind a selected catalogue root to existing flow/explanation
   commands without treating transport names as cross-repository call proof.
7. Tests: compiler-backed Spring fixtures, same-name impostors, composition,
   inheritance, repeated/dynamic/disabled triggers, byte coordinates, catalogue
   pagination and multi-repository identity. Document commands and limitations.

Primary references:

- https://docs.spring.io/spring-framework/docs/6.1.2/javadoc-api/org/springframework/web/bind/annotation/RequestMapping.html
- https://github.com/spring-projects/spring-framework/wiki/Spring-Annotation-Programming-Model
- https://docs.spring.io/spring-kafka/docs/3.0.16/api/org/springframework/kafka/annotation/KafkaListener.html
- https://docs.spring.io/spring-kafka/reference/kafka/receiving-messages/class-level-kafkalistener.html
