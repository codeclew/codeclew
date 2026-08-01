# ADR-003: Kotlin worker version isolation

## Context
K2, Analysis API and FIR change between compiler releases.
## Decision
Pin each worker to an exact Kotlin compiler/JDK 21 combination and expose only version-neutral DTOs. Keep the PSI/CFG implementation behind worker requests. The first supported variants are Kotlin 2.1.21 and 2.4.10; the Rust client discovers the project compiler through the Gradle model and switches variants before semantic analysis.
## Alternatives considered
Unpinned compiler dependencies, reflection across versions, and compiler objects in Rust.
## Consequences
A compiler upgrade creates another worker build; the current public-distribution workaround uses PSI and K2 Gradle validation.
## Failure modes
Unsupported compiler/project configuration is a typed error, never fallback.
## Compatibility implications
Core protocol versioning is independent of compiler versioning.

Compiler-internal adapters may differ by version. Shared PSI, protocol and project-model code is reused, while the Kotlin 2.1 FIR checker adapter is compiled separately to contain binary/API drift.
