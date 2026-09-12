# Durable documentation system: approved target

Status: product target approved; implementation plan pending separate approval.
Approval: the user replied "yes, I approve" on 2026-09-12 to the complete target
system together with the responsibility and verification extension. The following
is a consolidated specification of that approval, not a claim of implemented
production readiness. Product scope changes require a new explicit decision.

Implementation baseline: `a2dcefe2296e65f8f98e1cb205b2bbe3589e7989`, including the
development source-documentation change from PR #8. It is not the installed
release baseline. See [current scenarios](current-scenario-baseline.md),
[scenario cards](scenario-cards.md), [decisions](decisions.md),
[increment impact](increments/durable-documentation-impact.md), and the
[implementation plan](../../plans/documentation-system-implementation-plan.md).

## Product purpose and users

Maintain useful, explainable documentation of multiple source repositories in one
central documentation repository. Developers and analysts read responsibilities,
entities, contracts and processes. Documentation authors ask an agent to add or
revise sections. Service teams contribute protected notes and own architecture
statements. CI operators connect source changes to automatic verification and
publication without coupling the product to one model, runner or deployment scheme.

The initial operating envelope is approximately 40 services, fewer than five
changed repositories per day, and services with around 40 endpoints. This is an
acceptance workload, not a measured capacity result or a promised latency SLO.

## G1. Durable authoring and consistent presentation

Registration generates a standard service overview, responsibilities, domain entity
catalogue, and incoming/outgoing contracts. Internal methods support evidence;
they are not all promoted to separate reader-facing operations. Discovered public
boundaries must be described or represented by explicit actionable gaps. Ambiguous
boundary discovery cannot be presented as complete coverage.

The same documentation skill supports requests such as "document reservation
across Orders and Inventory" and "show the data flow of Reservation". A saved
request creates a stable section definition, navigation entry, evidence bindings,
and maintained explanation. Ad hoc questions remain transient unless persistence
is requested. No separate skill is required for each section type.

Predefined sections have versioned schemas, standard layouts and required content.
Missing information occupies a visible gap rather than silently removing a required
section. A single accepted explanation supplies its prose, tables and diagrams.
A common pinned renderer controls visual structure; model wording is not promised
to be byte-identical. New view modules declare rendering, validation and dependency
behavior instead of bypassing the shared model.

Arbitrary Markdown notes coexist with these sections. They can be imported or newly
written and associated with service/entity/contract/process IDs. They are searchable
through the documentation interface and appear in appropriate navigation or related
material. Their assessments have weaker, explicit integrity than structured sections.

## G2. Source-first evidence with optional modules

Initial modules cover Kotlin 1.9+, Java 17+, Spring Boot 3.3+ and OpenAPI. The release
must publish a tested compatibility matrix, separate declared project versions from
analyzer versions, and expose unsupported combinations. A plus sign is not a promise
that unknown future versions are compiler-validated.

The mandatory baseline reads committed source, configuration and contracts without
executing project code, resolving dependencies or requiring a build. It retains exact
revision-bound fragments, declaration/annotation facts, lexical control structure,
scope inventories and parsing limitations. Unsupported or omitted inputs remain
visible. A malformed file must not erase unrelated available evidence.

Explicitly configured K2, javac and other already supported Codeclew tools may enrich
the same objects. Providers report identity/version, applicability, input revisions,
coverage, limitations and source mapping. Facts must not be attached by an ambiguous
name or incompatible revision. Semantic failure leaves the source baseline readable
and invalidates claims that relied on the lost provider. Missing evidence is not
repaired by changing the model or silently upgrading syntax authority.

Spring interpretation runs in Rust on normalized declarations and annotations from
language providers. Source-only annotation candidates and compiler-resolved annotation
types retain different authority. No matching annotation alone establishes runtime
activation. Future Kafka, Avro, Protobuf, SOAP and additional provider modules can be
added without redesigning the document model. Preserve existing Kafka documentation
behavior while those future modules are deferred.

## G3. No silently current affected structured documentation

Every checkable claim has stable identity, a version and an explicit influence
boundary. Dependencies include direct facts, relevant source/configuration/contracts,
query scope and membership (including negative results), and shared documentation
objects. Paragraphs, contract rows, process steps and all rendered views inherit the
appropriate dependencies transitively.

Codeclew captures evidence reads and expansions through its supported authoring
interface. Reading outside that mechanism must widen the registered influence scope
or leave coverage incomplete. It is not acceptable to assert complete tracking from
citations alone. Imported evidence and human claims retain their own provenance.

An accepted source change invalidates all potentially affected claims and dependent
views. When precise influence is unknown, use a wider service/repository boundary.
New/deleted files, changed helpers, imports, constants, configuration and contracts
are included. Provider/rule changes are inputs even if source bytes are unchanged.
A query that formerly found nothing can become affected when a matching member appears.

Freshness is relative to an explicit revision set and recorded influence boundary.
Complete coverage of the modeled inputs is required for a structured claim to be
verified-current. Missing history, incomplete analysis, untracked relevant reads or
ambiguous identity produce an unverified result; they must not silently produce
current. This is a conditional dependency guarantee, not proof of universal program
semantics or arbitrary model prose. Coverage is evaluated for the particular claim
and its required capabilities: absent K2 does not prevent a fully covered syntax
claim from being current. An unresolved dispatch-dependent claim remains unverified
without downgrading an unrelated source-supported guard explanation.

Line movement with unchanged meaningful inputs rebinds coordinates. A rename, split,
merge, duplicate or removed object must establish unique correspondence or require
review. A content digest is not permission to overwrite a logical identity.

## G4. Automatic, best-effort maintenance

When the workflow learns that accepted revisions advanced, affected or potentially
affected sections lose their current status before agent generation. Status publication
and gap reporting require no model call. A missed external event cannot be detected
without a later revision reconciliation; both event delivery and reconciliation are
integration responsibilities exposed by the product.

Automatic authoring, review and publication are the normal path. No routine human
approval gate is imposed. Work and review have configured finite time/call/token
budgets across author, reviewer, repair and fallback attempts. Missing execution
configuration leaves a visible generation gap while deterministic updates proceed.
Budget values and latency objectives are installation-specific; none were approved
as universal numerical limits in this product scope.

If rechecking fails, retain the last accepted explanation with its actual source
revision, stale/unverified status, missing information and next action. A section
with no accepted content receives a gap. Unaffected sections continue publishing.
A new proposal is not silently substituted for accepted verified content. Status
metadata changes do not pretend that old prose describes new code.

All views of an explanation use the same accepted version. A publication can
legitimately contain independently versioned sections, including clearly marked older
ones. Corrupt or incomplete publication files never replace the prior valid output;
report the failed attempt. No stale section or missing provider globally blocks
otherwise valid updates.

## G5. Version history without deployment coupling

The normal view follows configured accepted source references for each repository.
Resolve them to immutable commits at the update boundary. Every published snapshot
records the target revision vector, per-section evidence and last-verified revisions,
explanation versions, provider/rule versions, and optional labels/tags.

Tags are labels, not a substitute for commit identity. Record their observed target;
a moved tag must not change historical snapshots. Retain previous documentation and
evidence for inspection/comparison. Saved processes and views are definitions that
can be inspected in a historical snapshot, not prompts that require regeneration.

Deployment systems may label a snapshot or supply a revision set in the future.
The first release does not infer deployed state or require any particular tag scheme.

## G6. Connected entities, contracts and processes

A domain entity has a stable ID distinct from classes, DTOs, messages and database
representations. Record who creates, changes, stores a copy of, reads or owns the
entity. Proposed ownership inferred from code is visibly different from an explicit
team statement; automatic publication does not make an inference authoritative.

Contracts describe observed input/output structure, operations/events, constraints,
errors and producer/consumer roles. Declared OpenAPI and source-derived contracts
remain separate evidence. Wire compatibility and runtime behavior require appropriate
evidence; matching field names or method signatures are insufficient.

Saved processes describe triggers, actions, decisions, outcomes and failure paths.
Data-flow views describe reads, transformations, writes and transfers for an entity.
Every edge keeps evidence and uncertainty. A matching name is not entity lineage.
Large processes use linked bounded subviews rather than an unbounded all-service
sequence. Existing bounded scenarios remain supported during migration.

## G7. Human material and assessment ownership

Agents cannot rewrite or delete human-authored text, tags or metadata. Keep authored
records separate from generated explanations and assessments. Reader pages compose
these layers using stable object IDs. Concurrent human edits cannot be overwritten
by an update prepared against an older revision.

An agent may attach evidence, an inconsistency finding, a limitation or a proposed
correction. Its assessment can itself become stale independently of the original.
Historical claims are evaluated in their applicable period. Current absence of
support is not proof that a historic explanation was false. Policies, intentions and
opinions are not silently converted into factual claims about current implementation.

## G8. Bounded agents and explicit verification

Codeclew owns source capture, provider execution, normalization, work preparation,
evidence/read tracking, dependency closure, schema/coverage checks, acceptance state,
manual protection and publication. The authoring agent owns bounded business
interpretation, wording, diagram meaning, explicit uncertainty and evidence requests.
It does not hand-maintain internal dependency graphs or claim its output verified.

A work package contains scope/audience, relevant facts and exact fragments, authority
and limitations, coverage obligations, existing content and human contributions,
changes and reasons for review, focused expansion references and a constrained
proposal format. Necessary helper/type/configuration context must be included or
explicitly obtainable. Bounded output must disclose omissions.

Verification separates evidence integrity, structural validity, coverage, supported
structured claim checks and meaning review. Structured checks establish only the
predicates they can evaluate. A separate review invocation checks prose and diagram
meaning against source and obligations. Authors cannot mint or reuse a review result
for another proposal or revision. A review invocation is separate, but model errors
can be correlated; no LLM verdict is advertised as mathematical proof.

Maintain separate freshness, verification, coverage and authority axes. Fresh input
is not proof of a correct explanation. Contradicted claims require correction;
unsupported obligations require an explicit gap. Only the validation/acceptance
path assigns acceptance state, including any verified-with-limitations outcome.

The configured routine agent may initially be DeepSeek-V4-Flash-0731. A more capable
configured agent can handle demonstrated reasoning difficulties after bounded repair.
Configuration can select gpt-6-astra, gpt-5.6-sol, gpt-5.6-terra or other compatible
models; names are examples, not fixed protocol values or tested capability claims.
Escalate on failed required checks, persistent contradictions or unsuccessful process
composition, not on model branding. Missing inputs produce gaps or provider requests.
The complete author/reviewer/repair/fallback contour must be measured.

## G9. Portable integration and production qualification

Codeclew exposes versioned job inputs and results. GitLab and the user's sandbox are
initial adapters, not core assumptions. Agent execution may be external; credentials
and vendor clients stay in integration configuration. Analysis is read-only with
respect to application repositories. Authoring capabilities are limited to the
approved documentation workspace and supported evidence requests.

Analyze changed repositories independently. Transfer immutable, validated evidence
packages to a central update workflow. Repeated events are idempotent; out-of-order
results cannot overwrite newer accepted targets. Recheck publication preconditions
against concurrent source/definition/note changes. An installation can implement the
workflow with CI jobs; a new always-on service is not a mandatory prerequisite.

Worker indexes/caches in CODECLEW_HOME are reconstructible technical state. They are
not a shared mutable global store for unrelated CI jobs. Durable authored definitions,
evidence, accepted explanations, assessments and publication manifests have explicit
storage/retention. Losing an index cache must not lose history or evidence needed to
inspect a retained snapshot. Use supported lifecycle and portable artifact interfaces,
not direct edits/copies of private session objects as a cross-worker protocol.

Before claiming production suitability, exercise the approved workload and failure
matrix, measure scale and full model cost, and document restoration, access control,
retention and operational configuration. Existing local CI or a passing plan review
is not evidence that this target has already been implemented or qualified.

## Acceptance contract and traceability

The scenario cards carry user journeys. This table is the canonical acceptance
contract for the increment; tasks refer to these IDs without changing their meaning.

| ID | Requirement and acceptance outcome | Scenarios |
| --- | --- | --- |
| AC01 | A code change publishes stale/unverified status before any model succeeds; every dependent retained view shows the same explanation status. | S04, S05 |
| AC02 | A missing provider or failed section preserves readable old content/gap while a valid unrelated update publishes; invalid publication files preserve the old pointer. | S04, S05, S12 |
| AC03 | Bounded work packages expose required evidence, obligations, omissions and expansion; supplied reads and relevant external inputs cannot escape influence tracking while claiming complete coverage. | S02, S08 |
| AC04 | Proposal submission needs only the constrained authoring format; Codeclew validates references/coverage/structured claims and materializes canonical bindings and views. | S02, S08 |
| AC05 | A separate meaning review is bound to proposal/evidence versions; author self-approval, stale review reuse, contradictions and budget exhaustion cannot silently accept content. | S08 |
| AC06 | Configured routine, reviewer, repair and fallback roles execute through portable adapters; missing evidence is distinguished from reasoning failure; all attempts remain accounted. | S08, S12 |
| AC07 | Kotlin/Java syntax survives absent or failed K2/javac; optional same-revision facts and explicit module/rule versions enrich existing identities with correct authority. | S01, S11 |
| AC08 | Spring Boot rules consume shared Rust annotation/declaration facts; syntax candidates, composed/dynamic cases and runtime uncertainty stay explicit across Kotlin/Java. | S07, S11 |
| AC09 | OpenAPI module facts preserve version, references and constraints; source-derived and declared contract claims remain distinct, with missing/unsupported references visible. | S07, S11 |
| AC10 | Service registration creates the standard overview/responsibility/entity/contract sections or explicit gaps; two differing fixture repositories share the same reader structure. | S01, S04, S07 |
| AC11 | Entity identities distinguish domain ownership from DTO/table representations and declared ownership from agent proposals; links and changes propagate without guessed correspondence. | S07, S09 |
| AC12 | Human Markdown/text/tags/metadata survive regeneration, import, rename association and concurrent edits; related assessments are separately versioned, staleable and visible. | S06, S10 |
| AC13 | A requested process becomes a saved discoverable section and updates transitively; ad hoc answers are not persisted unintentionally; bounded subviews preserve uncertainty. | S03, S09 |
| AC14 | A saved entity data-flow view binds transforms/reads/writes/transfers and unknown edges to evidence; changing a mapper updates all dependent representations. | S09 |
| AC15 | Portable per-service evidence can be validated/imported without private session copying or a live source checkout; wrong revisions, corrupt packages and incompatible producers are rejected locally. | S01, S11, S12 |
| AC16 | Exact target and per-section revision vectors, immutable snapshots and tag observations survive duplicate/out-of-order events, concurrent updates and history inspection. | S04, S06, S12 |
| AC17 | GitLab-triggered and vendor-neutral local jobs demonstrate the same event/job/result contract, sandbox boundaries and failure behavior without product-specific credentials in documents. | S12 |
| AC18 | Mutation tests cover helpers/configuration/imports/negative queries/new and deleted members/provider changes/identity ambiguity and transitive views, with no affected test claim falsely current. | S05, S07, S09, S10, S11 |
| AC19 | Cache loss, worker termination, incomplete artifacts, concurrent human edits and one failed service preserve valid publications, history and ownership at the 40-service fixture workload. | S06, S12 |
| AC20 | Actual configured routine/fallback combinations are qualified on representative authoring and maintenance tasks; correctness, gaps, retries, escalations, latency and total usage are reported without invented savings or universal guarantees. | S08, S12 |

## Explicit scope boundaries

No generic optimal context planner, universal static analysis, runtime execution proof,
or mandatory hosted agent is included. New Kafka/Avro/Protobuf/SOAP modules are
extension targets, not initial implementation requirements. No deployment discovery,
new browser-based editing product, custom identity provider, or training of models is
required. Source snapshots are committed inputs; uncommitted application editing is
outside this documentation workflow. Performance SLOs and installation budgets remain
measured/configured operational parameters, not silently approved numerical promises.
