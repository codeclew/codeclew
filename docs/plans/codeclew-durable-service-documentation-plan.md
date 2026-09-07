# Durable multi-service documentation: implementation plan and agent assignment

## Status and authority

- Status: S1-S8 implemented and verified for Codeclew 0.5.0.
- Date: 2026-09-07.
- Product source: the Codeclew repository.
- Knowledge location: a separate, user-owned documentation Git repository.
- Requested deliverable: integrated durable documentation, packaged agent workflow,
  and a new Codeclew release for testing in a protected environment.
- Execution: explicitly initiated by the user on 2026-09-07, including release publication.

The implementation record below is authoritative for the shipped interface.
The original design sections preserve planning context; where layout or command
spelling differs, use the packaged workflow and current CLI help.

## 0.5.0 implementation record

Implemented in `crates/clew/src/documentation` with the public `clew docs` group.
The canonical engineer/agent walkthrough is
[service-documentation.md](../../skills/codeclew/references/service-documentation.md).
The synthetic acceptance case is [durable-docs](../../fixtures/durable-docs/README.md).

| Slice | Delivered behavior |
|---|---|
| S1 | Closed serde records, reference validation, atomic per-record writes, local bindings and input-digest conflict protection |
| S2 | Independent admitted Java captures, compiler selectors, ambiguity candidates and exact source spans |
| S3 | Separate declared origin, endpoints, call site, HTTP method/path, destination key and runtime certainty axes |
| S4 | Bounded local flow plus explicitly declared HTTP transitions; branch, cycle and truncation boundaries |
| S5 | Neutral overview/service/scenario pages, complete-or-gap entrypoint coverage, bound diagrams, contracts and portable normalized observations |
| S6 | Fragment dependencies, affected/unaffected results, semantic versus line movement, stale narrative refusal and atomic bundle publication |
| S7 | Portable baseline recovery, ignored local paths/cache, generated ownership instructions and packaged workflow/examples |
| S8 | Native two-service compiler fixture and public CLI regression covering complete rendering, route changes, source preservation and fresh-home recovery |

Implementation choices replacing draft details:

- Catalogue records are individual versioned JSON files under `catalog/services`
  and `catalog/interactions`; scenarios are separate YAML files. A CLI update does
  not rewrite unrelated records or manually maintained scenario comments.
- `docs init` creates `AGENTS.md` when absent, otherwise a supplemental
  `AGENTS.codeclew-docs.md`, plus editable examples. Existing instructions survive.
- Narratives and normalized evidence are retained in the immutable generated
  bundle's `bindings.json`. `docs/index.html` is the atomic publication pointer.
  Local `.codeclew` state is disposable, never the sole durable knowledge copy.
- `docs context --service` provides the full paginated catalogue;
  `--entrypoint` narrows source-backed authoring and `--scenario` composes the
  selected flow. Retained operations can be recovered without chat history.
- Fresh checks rebuild evidence. Retained context explicitly reports that it has
  not been reverified; cached success does not establish current source truth.
- The renderer uses bundled HTML/CSS/JavaScript and native SVG, with Mermaid
  exports; no CDN, hosted model or external diagram process is required.
- Closed versioned record contracts are enforced by serde rather than a second
  separately maintained JSON Schema implementation.

Focused checks have established native Java 17/21 Spring extraction, two-service
HTTP composition, route-dependent staleness, guard omission rejection, line-only
freshness, repeated rendering and successful relocated/fresh-home recovery.
The full `./scripts/ci-verify.sh` gate passed on 2026-09-08, including the isolated
source-launcher usability smoke test. The runtime input manifest and release
package include the embedded skill references and documentation examples.
Release archives are produced by the existing `v0.5.0` tag workflow. No paid model
acceptance or protected-environment deployment has been claimed. Source analysis
supports Java only in this documentation release; scenarios are bounded to two
services, and runtime/DTO compatibility remains unproven.

## 1. User outcome

An engineer already has documentation for two services. They open their separate
architecture repository and tell an agent:

> During checkout, orders calls inventory over HTTP using POST /reservations.
> The client is InventoryClient.reserve, the receiver is
> ReservationController.create, and the destination uses inventory.base-url.
> Add this interaction and show the complete checkout scenario.

The agent locates both endpoints, records the engineer's declaration, reports
which details the code supports, and produces a sequence diagram and explanation
with links into both repositories. A new agent conversation on another machine
can load the same knowledge without the old conversation or Clew session IDs.

After either service or the declared interaction changes, Clew identifies the
affected interaction, scenario steps, diagram edges, and document fragments.
The agent regenerates those fragments and checks the coherence of the scenario.
Unresolved facts remain visible instead of becoming invented behavior.

The decisive persistence test is a new conversation and a fresh isolated
CODECLEW_HOME: after restoring the documentation repository and binding service
checkouts, all human declarations and scenario definitions are still usable.
Index reconstruction may cost time; human knowledge must not be lost.

## 2. Scope and delivery boundary

### Required first release

- One separate documentation repository and two Java 17+ Maven/Spring services.
- Explicit registration of services, with independently selected source revisions.
- A persistent manual interaction with concrete caller and receiver selectors.
- HTTP method/path and a symbolic destination configuration key.
- Agent-assisted discovery and disambiguation; direct file and CLI editing.
- A bounded local flow on each side and a declared cross-service transition.
- Markdown and a viewable sequence diagram, plus revision-pinned source links.
- Versioned document fragment bindings and an affected-fragment freshness report.
- Recovery across sessions, missing caches, relocated checkouts, and Git clones.
- Retention of user prose and manual declarations during regeneration.

When framework interpretation or local flow analysis is incomplete, the first
release may return an explicitly partial scenario. The acceptance fixture must
nevertheless demonstrate a complete, supported two-service HTTP path. Endpoint
cards alone are not completion of the sequence-documentation requirement.

### Approved documentation experience

- Preserve the reviewed Petclinic HTML design: sequence diagrams and contracts
  are primary, brief business summaries contain no implementation code, and
  selecting a step opens the exact source with highlighted lines.
- Generate one documentation page per registered microservice. Default scope is
  all discovered entrypoints across every catalogue page; a bounded subset needs
  an explicitly selected scope. A catalogue row alone is not a documented endpoint.
  Every entrypoint has either a detailed operation or an explicit, actionable gap.
- Generate a root overview linking available service documentation and named
  cross-service scenario slices. Keep service identity and independently pinned
  revisions visible; never present a declared edge as a compiler call.
- Keep the template service-neutral: no Petclinic-specific routes, findings,
  coverage prose, or participant IDs in reusable rendering code.
- Package the deterministic renderer, closed input model, a small complete
  source-to-events example, and a procedural authoring workflow. The external
  model supplies narrative decisions; byte/reference checks do not prove them.
- Validate the workflow with deterministic fixtures without claiming that a
  less capable model has been evaluated. A live model comparison is separate.
- Everything needed to render existing checked records works locally; no CDN,
  embedded LLM, API key, external diagram renderer, or hosted account is required.

### Subsequent extensions, not required for the first release

- Kafka and other brokers, gateway rules, gRPC, and other language combinations.
- More than two services in one composed scenario, including fan-out and cycles.
- Runtime observation import and environment-specific deployed-version mapping.
- Broad automatic topology discovery or automatic contract compatibility analysis.
- A graphical editor, hosted service, scheduler, or repository webhook integration.

The storage model should allow these extensions without reserving a large set
of unused fields or treating unimplemented transports as supported. A normal
explicit check is sufficient for the first release; no background monitoring
is needed.

## 3. Current implementation and concrete gaps

These are source-inspection findings, not results of a new product acceptance run.

| Existing component | Useful foundation | Gap for this scenario |
| --- | --- | --- |
| `crates/clew/src/thread.rs` and thread CLI | Binds 2-8 analysis units with service aliases and per-member authority | Session-bound analysis view is not a portable knowledge catalog |
| `crates/clew/src/workspace.rs` and README's mission-bound workspaces | Declared catalog edges and independent authority axes | Development workspace is mission/session-bound; documentation must not require a mutation mission |
| `crates/clew/src/thread_callables_service.rs` | Kotlin pairs and one Kotlin/Java pair | `create_bounded` rejects Java/Java; removing that check alone cannot provide Java flow evidence |
| `crates/clew/src/thread_flow.rs` | Bounded flows and declared topology handoffs | Handoff stops at a boundary; no general manual HTTP endpoint mapping and continuation inside the receiving service |
| `crates/clew/src/spring_entrypoints.rs` | Resolved trigger metadata and explicitly conditional runtime registration | Entry existence is not proof of destination routing, deployment activation, or client/server compatibility |
| `crates/clew/src/explanation.rs` and rendering modules | Claims, support references, declared component handoffs, rendered explanations | No complete portable scenario/fragment catalog for independently authored service documents |
| `crates/clew/src/explanation_freshness.rs` | Affected claim IDs, reasons, unaffected IDs, bounded comparisons | Existing comparison consumes retained thread/fact/flow inputs, not a portable documentation repository by itself |
| `crates/clew/src/navigation.rs` and JVM navigation modules | Reusable discovery, exact source bindings, bounded expansion | Need endpoint selection and composition across two Java services without promoting lexical matches |
| `skills/codeclew` and its packaged copies | Agent-facing product workflow | Need a concise persistent documentation workflow and structured interaction operations |

Relevant existing evidence: `docs/product/validation/java17-maven-index.md`
demonstrates Java source/entrypoint documentation with explicit limitations.
It does not establish a complete two-Java-service sequence or durable refresh.

Reuse the shared navigation, repository identity, source reference, diagnostics,
and bounded-result contracts where possible. Do not reshape mission-bound
workspaces into a mandatory documentation authoring process.

## 4. Engineer and agent workflow

1. Initialize the documentation repository or adopt its existing layout. Register
   stable service IDs and repository identities. Bind local checkouts separately.
2. Load existing service documentation. Reuse valid fragment bindings; otherwise
   treat imported prose as unbound until relevant source has been inspected.
3. Accept a natural-language declaration. The agent searches for caller and
   receiver candidates and produces a structured interaction draft.
4. Ask only about missing material information: overload, destination, environment,
   or competing route. An explicit instruction to add an unambiguous interaction
   authorizes the local write; do not add another confirmation step.
5. Save the declaration through the same validated operation available to a
   non-agent CLI caller. Recheck the proposed endpoints on the selected revisions.
6. Show an interaction card: source, destination, transport, route, conditions,
   origin of the declaration, observed checks, unresolved details, and source links.
7. Compose the named scenario from the caller flow, declared boundary, and
   receiving flow. Render its contract summary, explanation, and sequence diagram.
8. On later work, load the same repository and check freshness. Report affected
   fragments before rewriting them; retain manually maintained content.

Example card:

```text
Reservation during checkout
From: orders / InventoryClient.reserve(ReservationRequest)
To: inventory / ReservationController.create(ReservationRequest)
Transport: HTTP POST /reservations
Destination configuration key: inventory.base-url
Relationship: declared by engineer
Checks: both symbols resolved; supported method/path observations agree
Unknown: destination value and service activation in production
Affected scenarios: checkout
```

An engineer may instead add a service-level relationship without known symbols.
Save it as an incomplete declaration and show it on the service overview.
Do not invent method-level steps. Resolving its endpoints later preserves its ID.

## 5. Persistent storage in the documentation repository

Recommended initial layout, configurable when adopting existing documentation:

```text
architecture-docs/
  codeclew-docs.yaml
  catalog/
    services.yaml
    interactions.yaml
  scenarios/
    checkout.yaml
  docs/
    checkout.md
  evidence/
    checkout.bindings.json
  .codeclew/
    local.yaml                 # ignored, machine-specific bindings
    cache/                     # ignored, disposable derived data
```

### Ownership and lifetime

| Layer | Durable owner | Contents and lifetime |
| --- | --- | --- |
| Authored knowledge | Documentation Git repository | Services, interactions, scenario roots, conditions, rationale, explicitly recorded assumptions |
| Document content | Documentation Git repository | Agent-generated fragments and clearly identified manually maintained prose |
| Portable bindings | Documentation Git repository | Fragment/claim/dependency IDs, revision vector, stable source selectors, normalized bounded observations and their digests |
| Local binding | Ignored configuration | Service ID to absolute checkout path and local analysis configuration reference |
| Analysis runtime | Supported Clew local storage | Sessions, indices, retained source, worker outputs, and optional richer evidence; rebuildable |

Authoritative knowledge must not require a session, mission, conversation, CAS
directory path, or running agent to exist. Git records edits to declarations;
machine-specific checkout locations do not belong to tracked service identity.
Do not embed credentials or raw private build configuration in portable bindings.

The document can contain authorized source links, including internal links in a
private documentation repository. Those are different from default path-free CLI
diagnostics. Keep existing diagnostics contracts intact; create links explicitly
for the user-owned documentation output.

### Service identity and revisions

- Use stable service IDs and stable repository IDs; a single repository may
  eventually own several services. Do not use service display names as identity.
- Keep a credential-free repository locator and optional source-link template.
  Keep local paths in the ignored binding file.
- Record language, supported analysis profile, and compilation selector for each
  analysis unit. Unsupported profiles must return a capability diagnostic.
- Every check/render result binds a revision vector: documentation input digest
  and a separate revision/snapshot for each participating service.
- A ref such as `main` is a selection preference, not a permanent evidence ID.
- Represent dirty tracked inputs with an exact content snapshot if supported;
  otherwise report that the check is unresolved. Never silently describe HEAD as
  including uncommitted work, and never clean the user's checkout to proceed.
- A repository rename or explicit ID migration must preserve interaction IDs or
  return a reviewable migration; do not guess that another checkout is equivalent.

### Interaction contract

The following YAML illustrates the required information. Freeze an exact closed,
versioned schema in the first implementation slice; this is not an existing schema.

```yaml
schema: codeclew-documentation-interactions/1.0
interactions:
  - id: orders-reserves-inventory
    title: Reserve inventory during checkout
    from:
      service: orders
      selector:
        language: java
        owner: example.orders.InventoryClient
        name: reserve
        parameterTypes: [example.orders.ReservationRequest]
    to:
      service: inventory
      selector:
        language: java
        owner: example.inventory.ReservationController
        name: create
        parameterTypes: [example.inventory.ReservationRequest]
    transport:
      kind: http
      method: POST
      path: /reservations
      destinationConfigKey: inventory.base-url
    applicability:
      environments: [production]
    declaration:
      origin: human
      rationale: The checkout flow uses the inventory reservation API.
```

Additional contract requirements:

- Durable IDs are independent of the current symbol spelling and revision.
- A selector is a query to resolve, not a claim of compiler identity. Retain exact
  resolved symbol and source references in the check result, with overload handling.
- Support incomplete endpoint selectors explicitly; report missing fields without
  rejecting a useful service-level declaration as if it were complete code evidence.
- When a method has multiple relevant outbound calls, require an unambiguous
  call-site selector or report ambiguity. Method selection alone may be insufficient.
- HTTP direction is caller to receiver. Preserve client path, server path, and
  declared mapping separately when observations differ; never normalize away a mismatch.
- Missing applicability means scope was not specified, not all environments.
- Keep human declarations, agent proposals, and imported observations distinct.
  An agent cannot label its own guess as a human declaration.
- A proposal is not an accepted relationship by default. An engineer's explicit
  instruction may directly create the declaration without an extra approval stage.
- Support an optional contract reference. Identical DTO names do not establish
  wire compatibility; unsupported serialization details remain unknown.
- Refer to interaction IDs from scenarios rather than duplicating their content.
- Detect dangling service, interaction, scenario, and fragment references.

### Portable evidence and cache loss

Portable bindings contain a bounded, versioned projection of the dependencies
needed for document refresh. A CAS hash alone is not sufficient: another machine
may not possess its referenced object. Choose and document the minimum portable
projection rather than committing complete compiler indices or source archives.

Treat checked-in evidence as an input record, not newly verified compiler proof.
Validate schema and digests and rebuild or retrieve source-backed observations
before making a current authority claim. If the old revision cannot be obtained
and retained evidence is insufficient, report `UNRESOLVED`; still load all
declarations and allow analysis of the current revision. Do not pretend that
missing historical evidence was reconstructed successfully.

## 6. Proposed product interface

Use one cohesive `docs` command group if it fits current CLI conventions. Exact
names may be adjusted before implementation, but do not create competing aliases
or require a separate API for the agent. Each operation needs structured JSON
output with bounded lists, typed reasons, and continuation where applicable.

| Proposed operation | Behavior | Persistent write |
| --- | --- | --- |
| `clew docs init --root <dir>` | Adopt/create minimal documentation structure; report existing content conflicts | Documentation repo only |
| `clew docs service add --root <dir> --input <file>` | Validate and register a service | Tracked catalog |
| `clew docs bind --root <dir> --service <id> --repo <dir>` | Verify repository identity and bind its local checkout | Ignored local config |
| `clew docs interaction candidates --root <dir> --input <file>` | Return bounded caller/receiver candidates and source references | No authored knowledge write |
| `clew docs interaction put --root <dir> --input <file>` | Add/update one declaration with expected-input-digest conflict detection | Tracked catalog |
| `clew docs interaction list/show --root <dir> ...` | Inspect declarations and available check summaries | None |
| `clew docs interaction remove --root <dir> --id <id>` | Remove an explicitly requested declaration; diagnose dependent scenarios | Tracked catalog |
| `clew docs check --root <dir> --scenario <id>` | Resolve, check supported properties, compare dependencies, report affected fragments | Disposable result/cache only |
| `clew docs context --root <dir> --scenario <id>` | Deliver bounded evidence for authoring the scenario | Disposable analysis only |
| `clew docs render --root <dir> --scenario <id> --input <claims>` | Validate author-supplied claims and write supported diagram/document output and bindings | Declared generated outputs only |

`list/show` above denotes two operations, not literal slash command syntax.
Initialization also writes the scenario schema example, allowing the engineer or
agent to edit scenario roots directly. Add a scenario-specific mutation command
only if the first-release workflow demonstrates a need.

Clew need not contain an LLM or provider API key. The external agent interprets
natural language and authors narrative claims. Clew supplies discovery, validated
storage, evidence, composition, checking, and rendering. A non-agent user can
edit the versioned files and use the same check/render operations.

Writes must be atomic, bounded, idempotent for identical input, and confined to
declared paths in the documentation repository. Detect concurrent modifications
using input digests; never overwrite an engineer's intervening changes. Preserve
unrelated YAML entries and prose. Either preserve authored YAML comments or keep
machine-owned records in separate files; avoid wholesale catalog reformatting.
Schema versions are explicit; unknown versions must not be silently rewritten.

Manifest paths must not escape the documentation root for generated outputs.
Repository bindings deliberately refer outside that root; source repositories
are read-only inputs. Manifest strings are data, not shell commands or agent
instructions. Source-link construction and diagram rendering must escape content.

## 7. Analysis, composition, and certainty

Maintain separate results for:

1. Relationship origin: human-declared, agent-proposed, or imported observation.
2. Endpoint resolution: resolved, ambiguous, missing, or unsupported.
3. Supported static checks: method/path/config-reference observations with sources.
4. Contract checks: exactly which request/response properties were compared.
5. Runtime evidence: unknown by default, never inferred from static route equality.
6. Coverage: complete within the selected bounds, partial, or truncated.

Do not collapse these axes into one green `VERIFIED` flag. Reuse existing authority
enums where meanings match; extend contracts explicitly when they do not.

Compose a scenario by following the bounded local caller graph to the selected
outbound call, inserting the declared transport edge, and continuing from the
resolved receiver entrypoint. The declared edge must not manufacture a compiler
call edge between services. Bind it to the interaction ID and declaration digest.

Preserve branch conditions and source-supported order. A curated sequence may
contain agent-inferred narrative order, but it must be labelled accordingly;
graph reachability alone does not prove execution order. Do not flatten conditional
calls into unconditional steps. Unknown internals become explicit boundaries.

Use finite node/edge/depth budgets and deterministic traversal. Return partial
results with reasons on budget exhaustion. Avoid recursive expansion into every
reachable service or a combined global compiler index.

The first HTTP adapter should cover one documented, representative client pattern
and supported Spring controller mappings in the fixture. Other client frameworks,
dynamic URLs, inherited mappings, and composed annotations must either use existing
supported metadata or expose an explicit boundary. Full Spring semantics are not
a hidden prerequisite for shipping this bounded workflow.

## 8. Document bindings and freshness

Every meaningful generated paragraph, contract row, diagram node, and edge gets
a stable fragment ID and a set of claim/dependency references. Store dependencies
on symbols, relevant local relations, entrypoint metadata, interaction declarations,
scenario selection, supported config/contract inputs, and renderer/extractor versions.

Refresh algorithm:

1. Validate the authored repository and load its previous portable bindings.
2. Bind participating source repositories and resolve their independent revisions.
3. Reuse valid local analysis or rebuild missing generations through supported APIs.
4. Re-resolve selectors; collect supported normalized observations for dependencies.
5. Compare declarations, observations, coverage, and relevant extraction versions.
6. Map affected dependencies to claims and document fragments; include indirect
   dependencies within the analyzed scope.
7. Return a bounded report with fragment ID, scenario ID, reason, before/after
   references, available replacement candidates, and required action.
8. Let the agent regenerate affected fragments, then validate the complete scenario
   for consistency and write the new bindings with the outputs.

Use `CURRENT`, `PARTIALLY_STALE`, `STALE`, and `UNRESOLVED` consistently with the
existing freshness contract where possible. Additional reasons should distinguish
declaration edits, missing/ambiguous endpoints, route or contract changes, scope
changes, missing baselines, and incompatible evidence versions.

`CURRENT` means current within recorded dependencies and supported analysis.
A changed file hash alone is not semantic staleness. An unchanged method body
alone is not sufficient when a relevant callee, configuration reference, contract,
or declared target changed. Source line movement may refresh links without
requiring a narrative rewrite. Missing coverage cannot count as unchanged behavior.

Catalog additions/removals need a separate coverage notice: a new endpoint has no
old fragment dependency. Report it only for services/catalog scopes the user asked
to cover, without silently expanding a curated scenario. Changing an interaction
alone must trigger affected-fragment reporting even if source revisions are unchanged.

Regeneration preserves unbound/manual prose. Establish explicit generated regions
or separate generated files, detect user edits to generated fragments, and report
conflicts rather than replacing them. Commit new bindings only with their matching
output; interrupted generation must not leave old prose labelled as freshly checked.

## 9. Implementation slices and acceptance

Complete these sequentially. Each slice must deliver a reviewable artifact or a
confirmed behavior, not another process layer. File names below are suggestions;
follow current module boundaries when implementing.

### S1. Portable repository and manual declarations

- Define closed versioned catalog, interaction, scenario, and local-binding schemas.
- Implement parsing, reference validation, atomic declaration writes, service
  registration, local binding, and human/JSON inspection.
- Add the minimal source-controlled example under `fixtures/` using synthetic
  service identities. Do not initialize a real remote documentation repository.
- Keep the storage independent of missions and active sessions.
- Acceptance: create a declaration, end the process, reopen it, relocate service
  bindings, and inspect the same stable interaction ID and rationale.
- Tests: dangling references, duplicate IDs, incomplete declaration, unknown schema,
  concurrent update, and no-op repeat; preserve unrelated entries and comments.

### S2. Endpoint discovery and two-Java-service bindings

- Adapt current Java navigation/entrypoint evidence to resolve both selectors with
  explicit overload and call-site ambiguity. Add a bounded candidates operation.
- Support Java/Java documentation composition using qualified per-service evidence;
  do not simply remove language guards in the Kotlin fact service.
- Reuse source-profile admission and diagnostics. Compile/index in supported
  managed build locations, preserving the source checkouts.
- Acceptance: both endpoints and exact source links resolve in the synthetic two
  Maven service fixture. Competing overloads produce choices, not arbitrary binding.
- Tests: missing receiver, renamed method, duplicate method names, multiple outbound
  calls, missing source, unsupported profile, repository-identity mismatch.

### S3. HTTP interaction checking

- Implement a narrow HTTP observation adapter and the interaction card/report.
- Compare supported caller/receiver method and path observations against the
  declaration; retain destination config key and explicit environment scope.
- Keep unsupported DTO serialization, deployed configuration, gateway routing,
  authentication, retries, and runtime behavior outside automatic proof.
- Acceptance: matching and mismatching HTTP fixtures produce correctly scoped
  checks while relationship origin stays declared and runtime stays unknown.
- Tests: changed route, dynamic destination, missing environment selection,
  same-named DTOs without schema evidence, framework coverage boundary.

### S4. Bounded cross-service scenario

- Implement a documentation composition layer over reusable local flow evidence.
- Add any missing Java local call/branch evidence needed by the selected fixture,
  preserving its actual authority and testing the relevant worker/protocol changes.
- Connect caller call-site to receiver through an interaction-backed transport edge.
- Produce scenario context with source links, boundary reasons, and evidence IDs.
- Acceptance: checkout reaches reservation handling across the declared link and
  displays at least one supported conditional/local step on each relevant side.
- Tests: local cycles, branch preservation, missing callee, truncation, and no
  manufactured cross-service compiler edge or execution order.

### S5. Authoring, rendering, and portable bindings

- Define portable fragment/claim bindings; reuse explanation validation and
  rendering rather than creating an unrelated claim language when possible.
- Accept agent-authored narrative input; render a viewable sequence with source
  links, a contract summary, and visible uncertainty in Markdown/HTML as appropriate.
- Adopt existing documents without promoting their unbound prose to evidence.
- Establish generated/manual ownership and consistent multi-file output writes.
- Acceptance: an engineer can inspect both source locations from the rendered
  result and see exactly which transition was declared manually.
- Tests: malformed claims, escaping, invalid links, manual-content preservation,
  concurrent edits, and failure before output/binding publication completes.

### S6. Freshness and selective regeneration

- Extend or adapt freshness around portable bindings and the revision vector.
- Include interaction/config/contract changes, supported transitive dependencies,
  and catalog coverage notices. Preserve independent authority and coverage axes.
- Emit affected fragments and actionable reasons consumable by an agent or CI.
- Acceptance: change only the inventory route; report the interaction, relevant
  sequence edge, and contract row while unrelated scenario prose remains unchanged.
- Tests: declaration-only target change, caller-only update, callee change, source
  line movement, irrelevant edit, missing baseline, new endpoint coverage notice,
  analyzer version mismatch, and same-revision no-op refresh.

### S7. Cold recovery and agent workflow

- Make a fresh local run discover the root configuration and reload all durable
  declarations/scenarios without session IDs or conversation history.
- Add a concise workflow to the canonical Codeclew skill and synchronize its
  `.agents/skills/codeclew` and `.claude/skills/codeclew` copies.
- Supply a reusable documentation-repository instruction template pointing to its
  catalog and ownership rules. Do not overwrite existing AGENTS.md content.
- Acceptance: copy the tracked documentation repository into a temporary location,
  use a fresh isolated Clew home, bind fixture checkouts, and reproduce the declared
  interaction and a current or honestly unresolved freshness result.
- A second acceptance case must rebuild successfully when all source revisions
  are available; an unresolved result alone is not proof of successful recovery.
- Test missing old revisions separately. Never delete the user's real Clew cache
  or edit private state objects directly for these tests.

### S8. End-to-end completion and documentation

- Add a reproducible public synthetic fixture and a concise engineer walkthrough.
- Exercise the exported operations end to end, including the agent-authored input
  shape without requiring a paid live model run in deterministic CI.
- Document implemented HTTP/client/language coverage and command exit semantics.
- Provide structured statuses for ordinary CI invocation; no scheduled service is
  required. Define exit behavior separately for current, stale, unresolved, and
  invalid-input results so CI never confuses missing evidence with success.
- Run the appropriate final repository gate once after focused checks pass.
- Record actual supported behavior and remaining extensions, not intended behavior
  as if it had already passed.

## 10. Verification and completion criteria

Use the pinned Rust toolchain, Python 3.11+, JDK 21, and repository build wrappers.
During implementation run focused affected tests. For Rust use formatting and
relevant library/CLI tests; for Java/Kotlin or protocol changes run the affected
worker/contract checks. Rust tests alone do not establish worker correctness.

When skill packaging changes, run `python3 -I -S scripts/test_agent_skill.py`.
Validate affected schemas and public English content. Use the existing privacy
check before publication. Run `./scripts/ci-verify.sh` for the final merge-ready
implementation, not repeatedly between small slices. Expensive benchmarks,
release qualification, self-hosting, and paid pilots are outside this plan.

The first release is complete only when all of the following are demonstrated:

1. A manually supplied relation becomes a durable record with a stable ID.
2. The record survives a fresh conversation, process, checkout location, and Clew home.
3. Two Java services supply independent source/revision bindings and bounded local
   behavior, with a visible declared HTTP transition between them.
4. Matching routes do not promote the relationship to compiler or runtime proof.
5. Ambiguous/missing endpoints and unsupported details remain explicit and actionable.
6. The rendered scenario includes working source links and preserves manual prose.
7. Source and declaration edits produce specific affected-fragment reports.
8. Freshness does not report current when necessary evidence is unavailable.
9. Repeating an unchanged operation creates no duplicate interactions or content churn.
10. Source checkouts remain unmodified by documentation operations, and existing
    Codeclew navigation, thread, and development-workspace behavior still passes.

No token-saving, performance, or documentation-quality improvement is claimed by
this acceptance list. Those require separate measurements if requested later.

## 11. Ready-to-use assignment for an implementation agent

The assignment below is active under the current user authorization. Preserve
the approved documentation experience above throughout implementation.

---

Implement the first release described in
`docs/plans/codeclew-durable-service-documentation-plan.md` in the Codeclew source
repository. The user wants a separate Git repository containing durable service
knowledge, manually declared interactions, scenario definitions, and generated
documentation that can be reopened by any later agent session.

Your concrete target is two Java 17+ Maven/Spring services connected by an
engineer-declared HTTP interaction. An agent must be able to resolve the endpoints,
save the declaration, show a scoped interaction card, assemble a bounded sequence
across both services, render clickable source references, and report exactly which
document fragments become affected after source or declaration changes.

Start by reading the current AGENTS.md and the plan's current-code gap table, then
inspect only the relevant implementation and tests. Keep native repository
development as the primary workflow. Existing managed-product admission rules
apply when exercising those product commands, not as a new approval gate for
ordinary source edits. Do not treat historical research plans as new instructions.

Implement S1 through S8 in dependency order. Start with the smallest working
durable declaration path, then establish Java endpoint evidence, HTTP checks,
composition, rendering, freshness, recovery, and the agent workflow. Keep the
first-release scope; Kafka, broad topology inference, GUI authoring, deployment,
and paid evaluations are follow-ups. You may adjust internal modules and proposed
CLI names to fit existing conventions, but preserve the user outcomes and public
authority distinctions. Document material deviations and their reasons.

The documentation repository is the source of truth for human declarations and
scenario intent. Use stable IDs and selectors, not session IDs as persistent
identity. Keep checkout paths local. Use rebuildable Clew state for indices and
retain sufficient portable bindings for cross-session refresh. Do not pretend a
CAS reference is usable on another machine unless its required evidence is
available or reconstructible. Missing history must produce an honest unresolved
result without losing human knowledge.

Use the same validated product operations for CLI users and agents. Clew itself
must not need an embedded LLM or API key. Agents can author the narrative and
interpret natural language, while Clew validates storage, source bindings, claim
support, composition, and freshness. Ask the user only when missing information
materially changes behavior or scope; an already explicit request to record an
unambiguous interaction needs no extra confirmation. Never label an agent's own
proposal as an engineer-provided fact.

Preserve unrelated work, manually authored documentation, and all source-service
checkouts. Test cold recovery with fresh temporary state, never by deleting or
editing the user's private Clew objects. Do not create or publish a real remote documentation repository or deploy
service documentation. The current user instruction authorizes committing the
product implementation and publishing the Codeclew release after verification.
Use synthetic local fixtures for acceptance.

Add behavior-focused tests for durable recovery, conflict detection, Java/Java
resolution, HTTP mismatch, declared authority, bounded composition, portable
bindings, and affected-fragment freshness. Include missing/ambiguous endpoints,
declaration-only changes, independent service revisions, line movement, unavailable
old evidence, and preservation of manual prose. Keep the canonical skill and both
packaged copies identical when updating the agent workflow.

Carry the authorized implementation through relevant focused checks and the final
`./scripts/ci-verify.sh` gate. Do not repeatedly rerun passing expensive checks
without a material change. Report a concrete new artifact, verified behavior, or
resolved blocker as progress. If 30 minutes or 100 tool calls produce none, narrow
the slice and state the facts instead of expanding process or scope.

At completion provide: the implemented engineer workflow, key changed files,
commands actually supported, tests actually run, the cold-recovery result, the
route-change affected-fragment result, and remaining limitations. Mark incomplete
acceptance items honestly. Do not claim that all runtime routing, wire contracts,
or future transports have been proven by the bounded HTTP implementation.

---
