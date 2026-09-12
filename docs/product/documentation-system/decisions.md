# Documentation-system decisions

Status: approved product decisions; technical implementation plan pending approval.
Approval source for D01-D12: the user's explicit "yes, I approve" on 2026-09-12,
in response to the consolidated target system and responsibility/verification
extension, following the recorded clarification of Spring Boot and initial scope.
The [target](target-system.md) is the canonical consolidated product description.
The [impact](increments/durable-documentation-impact.md) and
[plan](../../plans/documentation-system-implementation-plan.md) reference these decisions.

Only approved_by_human or explicitly delegated product decisions may drive the plan.
File/module/command choices in the plan are proposed implementation details submitted
for the second approval; they do not silently amend the approved product boundaries.
There are no unresolved product choices that prevent producing the requested plan.
Installation credentials, retention values and execution budgets are deployment inputs,
not approval of a particular account, cost ceiling or future paid run.

## D01. Hybrid documentation

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** A fixed-only catalogue would prevent the requested free-form context; unrestricted prose alone cannot support strict freshness.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Use predefined strict sections plus arbitrary protected notes and optional assessments. Alternative considered: A fixed-only catalogue would prevent the requested free-form context; unrestricted prose alone cannot support strict freshness.
- **Trade-offs:** Schemas and dependencies apply to structured sections; arbitrary notes have a visibly weaker assessment status.
- **Agent recommendation:** Use predefined strict sections plus arbitrary protected notes and optional assessments.
- **Human decision:** Use predefined strict sections plus arbitrary protected notes and optional assessments.
- **Impact:** G1/G7; S04, S07, S10; AC10/AC12

## D02. Latest accepted source and immutable history

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** Deployment-specific snapshots would couple the product to one installation; moving tags alone are not stable identities.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Follow configured accepted source references; record exact commit vectors, per-section verified revisions and optional tag labels. Alternative considered: Deployment-specific snapshots would couple the product to one installation; moving tags alone are not stable identities.
- **Trade-offs:** History remains reproducible without requiring deployment discovery. Mixed-age sections are explicit.
- **Agent recommendation:** Follow configured accepted source references; record exact commit vectors, per-section verified revisions and optional tag labels.
- **Human decision:** Follow configured accepted source references; record exact commit vectors, per-section verified revisions and optional tag labels.
- **Impact:** G5; S06, S12; AC16

## D03. Automatic best-effort publication

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** A global completeness gate prevents the requested always-useful documentation; silently current old pages are also unacceptable.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Automatically publish accepted updates and independently publish stale/gap status; a failed section never blocks valid unrelated content. Alternative considered: A global completeness gate prevents the requested always-useful documentation; silently current old pages are also unacceptable.
- **Trade-offs:** Preserve previous valid content or a gap with actual revisions and missing information. Corrupt publication attempts retain the prior valid pointer.
- **Agent recommendation:** Automatically publish accepted updates and independently publish stale/gap status; a failed section never blocks valid unrelated content.
- **Human decision:** Automatically publish accepted updates and independently publish stale/gap status; a failed section never blocks valid unrelated content.
- **Impact:** G4; S04, S05, S12; AC01/AC02

## D04. Initial analysis scope and modular growth

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** Implementing every protocol immediately expands the scope; isolated hard-coded paths would obstruct future providers.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Start with Kotlin 1.9+, Java 17+, Spring Boot 3.3+ and OpenAPI; defer new Kafka/Avro/Protobuf/SOAP modules but preserve existing behavior and extension seams. Alternative considered: Implementing every protocol immediately expands the scope; isolated hard-coded paths would obstruct future providers.
- **Trade-offs:** Release compatibility is tested and declared, including explicit unsupported combinations.
- **Agent recommendation:** Start with Kotlin 1.9+, Java 17+, Spring Boot 3.3+ and OpenAPI; defer new Kafka/Avro/Protobuf/SOAP modules but preserve existing behavior and extension seams.
- **Human decision:** Start with Kotlin 1.9+, Java 17+, Spring Boot 3.3+ and OpenAPI; defer new Kafka/Avro/Protobuf/SOAP modules but preserve existing behavior and extension seams.
- **Impact:** G2; S01, S07, S11; AC07-AC09

## D05. Optional semantics and shared framework rules

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** Compiler-only admission loses availability; pretending syntax equals compiler facts loses authority.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Use source analysis as baseline, optionally enrich with K2/javac and applicable existing tools, and interpret Spring facts in shared Rust rules. Alternative considered: Compiler-only admission loses availability; pretending syntax equals compiler facts loses authority.
- **Trade-offs:** Provider identity, version, coverage, mappings and failures remain inputs. Reuse existing Codeclew facts, Spring and adapter seams.
- **Agent recommendation:** Use source analysis as baseline, optionally enrich with K2/javac and applicable existing tools, and interpret Spring facts in shared Rust rules.
- **Human decision:** Use source analysis as baseline, optionally enrich with K2/javac and applicable existing tools, and interpret Spring facts in shared Rust rules.
- **Impact:** G2; S11; AC07/AC08

## D06. Automatic standard pages and requested saved processes

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** Generating every imaginable process is unbounded; treating saved process requests as transient loses durable documentation.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Registration creates standard responsibility/entity/contract sections; detailed process and data-flow sections are saved and maintained when requested. Alternative considered: Generating every imaginable process is unbounded; treating saved process requests as transient loses durable documentation.
- **Trade-offs:** Internal methods support evidence. Public boundary coverage and unknowns remain explicit.
- **Agent recommendation:** Registration creates standard responsibility/entity/contract sections; detailed process and data-flow sections are saved and maintained when requested.
- **Human decision:** Registration creates standard responsibility/entity/contract sections; detailed process and data-flow sections are saved and maintained when requested.
- **Impact:** G1/G6; S07, S09; AC10/AC11/AC13/AC14

## D07. Conditional complete influence tracking

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** Citation-only tracking misses helpers and negative queries; universal semantic dependence cannot be guaranteed for arbitrary code.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Require complete registered influence boundaries for strict verified-current claims; record reads/query membership and use broad watches when uncertain. Alternative considered: Citation-only tracking misses helpers and negative queries; universal semantic dependence cannot be guaranteed for arbitrary code.
- **Trade-offs:** Incomplete evidence is unverified; potentially affected structured documents and derived views become stale transitively.
- **Agent recommendation:** Require complete registered influence boundaries for strict verified-current claims; record reads/query membership and use broad watches when uncertain.
- **Human decision:** Require complete registered influence boundaries for strict verified-current claims; record reads/query membership and use broad watches when uncertain.
- **Impact:** G3; S05, S08, S09; AC03/AC18

## D08. Protected human ownership

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** Inline replacement would destroy historical context; unassessed notes would omit the requested consistency feedback.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Never rewrite/delete human text, tags or metadata; store generated assessments/corrections separately and preserve temporal applicability. Alternative considered: Inline replacement would destroy historical context; unassessed notes would omit the requested consistency feedback.
- **Trade-offs:** Assessment freshness is separate from note ownership and from strict structured-document verification.
- **Agent recommendation:** Never rewrite/delete human text, tags or metadata; store generated assessments/corrections separately and preserve temporal applicability.
- **Human decision:** Never rewrite/delete human text, tags or metadata; store generated assessments/corrections separately and preserve temporal applicability.
- **Impact:** G7; S06, S10; AC12

## D09. Bounded authoring and product-owned mechanics

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** Asking routine agents to manage opaque identities and full graph/publication machinery increases avoidable errors.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Codeclew prepares evidence/obligations, tracks dependencies and constructs validated canonical records from constrained proposals. Alternative considered: Asking routine agents to manage opaque identities and full graph/publication machinery increases avoidable errors.
- **Trade-offs:** Authors own interpretation and wording; they cannot set their own verification state.
- **Agent recommendation:** Codeclew prepares evidence/obligations, tracks dependencies and constructs validated canonical records from constrained proposals.
- **Human decision:** Codeclew prepares evidence/obligations, tracks dependencies and constructs validated canonical records from constrained proposals.
- **Impact:** G8; S02, S08; AC03/AC04

## D10. Separate meaning review and measured escalation

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** Self-approval is inadequate; always using the most expensive model is not required; missing evidence cannot be invented by escalation.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Use deterministic checks plus a separate proposal/evidence-bound meaning review; configure bounded repair and stronger-model fallback on observed failures. Alternative considered: Self-approval is inadequate; always using the most expensive model is not required; missing evidence cannot be invented by escalation.
- **Trade-offs:** Qualify the full configured author/reviewer/repair/fallback contour and disclose correlated model error limits.
- **Agent recommendation:** Use deterministic checks plus a separate proposal/evidence-bound meaning review; configure bounded repair and stronger-model fallback on observed failures.
- **Human decision:** Use deterministic checks plus a separate proposal/evidence-bound meaning review; configure bounded repair and stronger-model fallback on observed failures.
- **Impact:** G8; S08; AC05/AC06/AC20

## D11. Portable CI and agent integration

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** A mandatory hosted service or one model API would constrain the requested integration patterns.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Support GitLab and the existing sandbox through versioned inputs/results and configurable execution adapters without a hard-coded vendor/model. Alternative considered: A mandatory hosted service or one model API would constrain the requested integration patterns.
- **Trade-offs:** Read-only application analysis; isolated worker state; durable portable evidence and idempotent central updates.
- **Agent recommendation:** Support GitLab and the existing sandbox through versioned inputs/results and configurable execution adapters without a hard-coded vendor/model.
- **Human decision:** Support GitLab and the existing sandbox through versioned inputs/results and configurable execution adapters without a hard-coded vendor/model.
- **Impact:** G9; S11, S12; AC15/AC17/AC19

## D12. Operating envelope and qualification honesty

- **Status:** approved_by_human
- **Owner:** Human product owner
- **Decision needed by:** Target-system approval, completed before planning
- **Approval source:** The explicit 2026-09-12 approval recorded above
- **Context:** No latency SLO, provider credentials or universal token-saving threshold was supplied. Inventing them would create an unsupported product promise.
- **Evidence:** [Approved target](target-system.md), [observed baseline](current-scenario-baseline.md)
- **Options:** Use approximately 40 services, fewer than five changed repositories/day and around 40 endpoints/service as the initial workload; configure budgets and measure performance. Alternative considered: No latency SLO, provider credentials or universal token-saving threshold was supplied. Inventing them would create an unsupported product promise.
- **Trade-offs:** The plan includes measured capacity/recovery and actual model qualification. Planning review is not an implemented production-readiness verdict.
- **Agent recommendation:** Use approximately 40 services, fewer than five changed repositories/day and around 40 endpoints/service as the initial workload; configure budgets and measure performance.
- **Human decision:** Use approximately 40 services, fewer than five changed repositories/day and around 40 endpoints/service as the initial workload; configure budgets and measure performance.
- **Impact:** G9; S12; AC19/AC20
