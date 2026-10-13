# Thread call bindings: implementation and acceptance handoff

Base: `370a5f20b4676ad926a80cbdb3ad3be212898e37` (main, workspace 0.13.32).
Branch: `feat/thread-call-bindings`.
Scope: the first causal-thread slice, compiler-selected Kotlin call targets and
source expression -> formal parameter bindings. The separate reachability
experiment `420c034` is not included. No release change or provider authoring.

## Problem and result

Existing K2 projections already retained exact targets and parameter mappings,
but process graphs ignored CALL_RELATION records. A valid call could remain
absent from traversal when no independent FLOW call was captured, and its actual
expressions, named parameter order and default omissions were invisible to the
reader. The new projection follows only exact same-scope descriptor candidates,
retains separate relation occurrences, binds argument text to exact UTF-8 bytes
and presents the selected receiving declaration without promising runtime dispatch.

## Acceptance corpus

| Case | Expected result | Check |
| --- | --- | --- |
| Two overloads | Follow only the exact JVM target; no simple-name fallback | Graph regression plus wrong-overload rejection |
| Named arguments out of formal order | Preserve source order and formal indices 1 then 0 | Projection regression |
| Omitted default | Keep omitted index; default value UNKNOWN | Projection regression |
| No old mapping | UNKNOWN and no invented arguments | Compatibility regression |
| Duplicate/unscoped/foreign target | No followed target; precise frontier | Projection and graph regressions |
| Changed digest/revision/call range or split UTF-8 | Fail closed | Corruption regression |
| No FLOW call | Relation discovers callee; events stay empty | Graph regression |
| Call source exceeds budget | Explicit cut before retaining call/method | Graph regression |
| Receiver/return/exception unsupported | Keep UNKNOWN independently of exact target | Projection and reader regressions |
| Real K2 importer -> publisher | Map actual `request` to formal slot 0 and retain unknown transfer | Extended existing ignored Kotlin 1.9 native capture test |

The hand-built compiler fact fixtures test the consumer contract. They are not
native/compiler/runtime proof. The existing native test was extended to traverse
actual trusted compiler output; it is invoked by full CI, not by the pure fixture
suite. Source bodies and provenance remain available, and no new thread narrative
is authored.

## Verification and continuation

Local JavaScript syntax and reader tests passed: 69 passed, two optional fixtures
skipped. English/privacy worktree and whitespace checks passed during development;
repeat them on the final commit. Rust 1.92, JDK 21 and .NET 10 are unavailable on
this cloud host (Rust 1.91 formatting and JDK 17 are available). No full native
crate check or compiler replay has been claimed here. CI's existing macOS recovery
job now runs focused pure binding/graph tests before its existing tests; the full
verification job retains its original privacy, Clippy, worker and native gates.

```sh
cargo fmt --all --check
cargo test --locked -p clew --lib documentation::process_call_bindings::tests:: -- --test-threads=1
cargo test --locked -p clew --lib documentation::process_graph::tests:: -- --test-threads=1
cargo test --locked -p clew --lib documentation::kotlin::tests::exact_call_argument_binding_envelope_preserves_source_order_and_optional_names -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
./scripts/ci-verify.sh
```

Use pinned Rust 1.92 and the native worker environment. Resolve all compile/test
failures before integration. GitHub connector commits use a personal identity;
maintainer integration must preserve the repository privacy policy rather than
weakening it. A locally authored maintainer patch is supplied separately.

Draft PR: https://github.com/codeclew/codeclew/pull/24. The first CI verification
run stopped at history privacy before compilation: both base commit `370a5f2`
and the connector-created feature commit have non-generic author/committer
identities. This is not a Rust test result. The independent macOS job remains
pending verification; no passed Rust or native replay result is asserted.
The local patch commits use `Codeclew Maintainers`; the existing main history
requires a separate maintainer decision, without disabling the privacy gate.

## Next causal slices

Receiver object/dispatch identity needs qualified receiver and allocation/alias
facts. Kotlin normal return transfer needs exact argument-definition and result
bindings before reusing `thread_value_calls`; source expressions are insufficient.
Exception and effect transfer needs matched call/return/throw summaries. These
remain bounded follow-up work, with precision/recall measured on a labeled public
corpus. No full end-to-end causal completeness or private-project acceptance is
claimed by this slice.
