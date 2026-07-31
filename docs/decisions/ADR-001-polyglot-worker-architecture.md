# ADR-001: Polyglot worker architecture

## Context
Compiler ecosystems have incompatible runtimes and version lifecycles.
## Decision
Keep canonical analysis/transactions in Rust and Kotlin PSI/compiler work in a long-lived JVM process over framed Protobuf.
## Alternatives considered
JNI, a JVM-only system, and a Rust parser-only implementation.
## Consequences
Language adapters are replaceable and compiler crashes do not corrupt core state; serialization is explicit.
## Failure modes
Protocol mismatch, worker crash, oversized frame, or poisoned stdout fails the request.
## Compatibility implications
Envelope additions remain optional and worker versions negotiate capabilities.

