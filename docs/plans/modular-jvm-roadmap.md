# Modular JVM analysis and Spring computation roots

## Outcome

An engineer installs Codeclew and its bundled skill, opens a Kotlin 1.9+ or
Java 17+ repository, and immediately has useful analysis and documentation
assistance. Supported Kotlin 2.4.x projects can additionally use qualified K2
semantics and managed changes. Existing exact engines keep their versions and
stronger guarantees. HTTP endpoints, Kafka listeners and scheduled jobs provide
roots for documenting threads in one or several repositories.

Broad language support means a useful baseline with explicit limitations; it
must never silently relabel syntax observations as compiler-resolved facts.
“2.4.x support” separates project compatibility from the exact packaged compiler
binary. An unqualified combination can still analyze at baseline capability but
cannot inherit advanced mutation permission.

## Existing architecture to build on

- `crates/clew/src/adapter_v2.rs` separates `BuildModelProvider` from
  `LanguageAdapter`, with capability handshakes, compilation descriptors, fact
  shards and completeness receipts.
- `repository_snapshot.rs`, `cas.rs` and `generation_v2.rs` preserve immutable
  inputs and reusable fact generations. `context_v2.rs` and `navigation.rs`
  return bounded evidence to the agent.
- `kotlin_engine.rs` preserves exact compiler identities; `workers/kotlin*`
  isolate version-sensitive extraction. `java_project_model.rs` and
  `java_adapter_v2.rs` provide the Java Compiler API path.
- `thread*.rs` preserve member boundaries; `jvm_navigation.rs` distinguishes a
  compiler identity match from provider artifact ownership and compatibility.
- `workspace_prepare.rs` and `workspace_publish.rs` isolate changes and verify
  publication authority. Read-only capability does not open this path.
- `site/architecture.html` explains these boundaries and the data flow. These
  interfaces are internal extension seams; a third-party runtime plugin loader
  is not required for this outcome.

## Delivery sequence and proof

| Priority | Deliverable | Acceptance evidence |
| --- | --- | --- |
| 1 | Baseline Kotlin/Java analysis with automatic capability selection | Installed-launcher fixtures for Kotlin 1.9 and 2.x, Java 17 and 21, Gradle/Maven, and missing build dependencies. Search returns source anchors and explicit resolution; baseline mutation is rejected. No project compiler downgrade. |
| 2 | Kotlin 2.4.x compatibility independent of packaged engine identity | Real project/engine qualification on representative 2.4 patches. The 2.4.10 engine stays exact; unsupported patches have a typed capability boundary and usable baseline. Existing exact-engine regression checks pass. |
| 3 | Spring computation-root catalogue | Satisfy [the Spring acceptance contract](spring-entrypoints.md): resolved annotation arguments, composition and inheritance, repeated triggers, pagination, stable callable identity, and selected-repository coverage. A missing extractor cannot masquerade as zero roots. |
| 4 | Java parity for Spring extraction | Extend Java annotation facts with resolved attributes and origin; run equivalent Kotlin/Java Spring fixtures including same-name impostors and dynamic values. Feed the common catalogue schema without claiming Java mutation. |
| 5 | Agent documentation workflow across members | One installed-skill scenario lists all pages of roots, selects a callable, follows supported edges and emits a document with citations and unresolved boundaries. Identical names across repositories remain distinct. A same-topic/path match is never sufficient proof of a cross-repository call. |
| 6 | Release and architecture publication | Versioned CLI, bundled skill, README, site and changelog agree on actual support. Targeted tests and package/installer gates pass; publish versioned release assets and deploy the static site. Verify the installed release and public architecture URL. |

Implement the smallest vertical path that preserves the full destination:
useful repository analysis first, then stronger extraction and documentation.
Do not spend the initial budget on a plugin marketplace, general language
framework or unsupported writes. Stages may share a release only when their
acceptance evidence exists; clearly label the remaining stages as planned.

## Agent allocation

Use three bounded workers and one integrating agent; wider parallelism would
increase overlapping changes and verification cost.

1. **Language compatibility worker:** baseline selection, Java 17+ boundaries,
   Kotlin project/engine qualification and focused compatibility fixtures.
2. **Spring extraction worker:** K2 annotation facts and fixture coverage. The
   integrating agent owns the public catalogue, paging and multi-member binding.
3. **Architecture/documentation worker:** stage diagram, source-linked extension
   guide, support claims and this rollout plan. It takes compatibility facts
   from the implementation workers before finalizing claims.
4. **Integrating agent:** contract decisions, catalogue wiring, installed-skill
   end-to-end acceptance, release notes, packaging and publication. Reuse test
   results unless implementation changes invalidate them.

Each handoff reports changed files, the verified capability, its test command,
and remaining boundary. A green unit suite alone does not establish broad
language support or a complete runtime inventory.

## Completeness and extension rules

A catalogue must identify its repositories, revisions, compilations and covered
extractors, alongside truncation and unresolved values. Static analysis alone
cannot prove active beans, deployed profiles, placeholder/SpEL values or dynamic
registrations. Those remain obligations for configuration or runtime evidence.

To extend a build provider, return a validated model. To extend a language
adapter, declare capability and toolchain authority and emit facts plus a
receipt. To extend Spring behavior, keep compiler resolution in the adapter and
framework rules above normalized facts. Change schema/translation authority
when meaning changes, preserve source anchors through caching, and add one
positive and one false-positive regression for each new rule.
