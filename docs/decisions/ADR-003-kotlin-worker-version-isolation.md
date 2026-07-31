# ADR-003: Kotlin worker version isolation

## Context
K2, Analysis API and FIR change between compiler releases.
## Decision
Pin the first worker to Kotlin 2.4.10/JDK 21 and expose only version-neutral DTOs. Keep the PSI/CFG implementation behind worker requests.
## Alternatives considered
Unpinned compiler dependencies, reflection across versions, and compiler objects in Rust.
## Consequences
A compiler upgrade creates another worker build; the current public-distribution workaround uses PSI and K2 Gradle validation.
## Failure modes
Unsupported compiler/project configuration is a typed error, never fallback.
## Compatibility implications
Core protocol versioning is independent of compiler versioning.

