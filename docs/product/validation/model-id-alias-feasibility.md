# Short model IDs: bounded feasibility assessment

Date: 2026-10-06. Source revision: `30c837d` (development), release baseline
`0.13.14`. Disposition: implementation deferred from the immediate scoped-read
release; retain as a promising, separately scoped representation improvement.
This assessment adds no product codec or provider integration.

## Observed opportunity

The retained `prepare-null-boundaries` author and reviewer calls from the accepted
context-efficiency comparison used `gpt-6.1-sol`. Their actual saved prompts are
an instruction prefix followed by compact JSON. The saved provider strict output
schemas are measured separately; the native output schema is also inside each
prompt. No provider calls or five-case qualification reruns were performed.

Counts use `tiktoken` 0.14.0 with the reference `o200k_base` encoding. The library
could not resolve `encoding_for_model("gpt-6.1-sol")`. These are reference text
tokens, excluding chat framing and provider schema wrappers, not confirmed
target-model input tokens or billed usage. See the [primary tokenizer documentation](https://github.com/openai/tiktoken).

| Representation | Role | Prompt bytes | Prompt reference tokens | Strict-schema bytes | Strict-schema reference tokens |
| --- | --- | ---: | ---: | ---: | ---: |
| Canonical baseline | Author | 36,605 | 9,525 | 5,647 | 1,555 |
| Binding-only prospective | Author | 36,700 | 9,482 | 5,628 | 1,547 |
| Typed opaque upper bound | Author | 34,990 | 8,582 | 5,629 | 1,547 |
| Canonical baseline | Reviewer | 55,769 | 14,378 | 3,901 | 1,258 |
| Binding-only prospective | Reviewer | 54,620 | 13,627 | 3,505 | 1,034 |
| Typed opaque upper bound | Reviewer | 53,071 | 12,813 | 3,509 | 1,034 |

The broader prospective representation reduces the sum of independently counted
prompt and schema reference tokens from 11,080 to 10,129 for the author and
15,636 to 13,847 for the reviewer: 26,716 to 23,976 jointly, or 10.26%.
This sum describes measured text components, not an exact provider request count.
Prompt bytes decrease by 4.67% jointly. Byte percentages alone understate this
reference-token opportunity. Binding-only yields a smaller 3.84% joint reference
reduction and increases author prompt bytes.

Both prospective forms include a 246-byte alias instruction in each prompt.
The host-only maps contain 1/9 entries and 81/641 bytes for binding-only, or
21/28 entries and 1,470/1,952 bytes for the broader author/reviewer form.
Mapping tables are excluded from model input; sending them would change benefit.
The broader form includes allowlisted provenance and source-span identities,
in addition to top-level bindings. It leaves existing short citation labels and
hash-bearing semantic dataflow/JVM identities intact.

The offline transformations preserved the 47 author and 70 reviewer protected
field values checked, including source text/tokens, symbols, names and paths.
Exact leaf projection and inverse projection restored the original payload before
adding protocol guidance and adjusting schema patterns. This establishes neither
a complete transport round trip nor typed-domain isolation, stable expansion
aliases, malformed-output recovery or model answer quality. The script is a
measurement aid, not a production codec.

## Canonical validation and the supported seam

All four prospective packets were rejected by the existing measured driver with
`packet content binding differs`. That is expected when projected IDs are put
inside a canonical packet before validating its digest. It does not show that a
projection after canonical validation is unsafe or infeasible.

Keep canonical Work, packets, public CLI and stored evidence unchanged. The
smallest viable design is a host-prepared, versioned model payload and output
schema alongside the canonical request. An explicitly compatible driver first
validates the canonical request, then forwards the prepared representation to
the model. Deliver it through an explicit versioned opt-in carrier containing
the unchanged canonical job and frozen projection/map, retained before dispatch.
The driver cannot implicitly read coordinator state through the sandbox. An
alternative that keeps stdin byte-identical is deterministic reconstruction
through the shared serializer, including order-independent stable aliases and
collision rejection; a host map alone is insufficient. Decode returned identity fields before existing semantic validation
and before producing canonical saved answers or reviewer inputs. Never recalculate
the canonical packet digest from aliased content.

The [supported driver boundary](../../../skills/codeclew/references/service-documentation.md)
already allows a trusted operator-owned serializer after canonical validation.
The public Rust library can expose a shared pure projection API; absence of an
in-tree provider client does not rule out a supported implementation.
The current [adapter](../../../crates/clew/src/documentation/agent_adapter.rs)
serializes immutable stdin, closes it after delivery and accepts one final stdout
reply. Its sandbox denies driver writes. The repository contains a deterministic
[transport fixture](../../../fixtures/documentation-system/agents/README.md),
not a maintained production provider serializer. Updating the private comparison
driver alone would therefore not establish supported product behavior.

A driver-only transformation can preserve canonical checks, but a representation
receipt returned only after execution cannot establish exact prepared model input
after a crash or timeout. Host preparation avoids a new duplex prepare/ack
protocol: persist both representations and their binding before dispatch, and
require the opt-in driver to use the prepared form. This still proves the admitted
driver contract, not independently observed provider delivery.

## Concrete implementation boundary

A follow-up needs these coupled components; this is an implementation estimate,
not a claim that each requires a new public protocol:

1. A typed projection/decoder module with explicit field paths and domains,
   synchronized generated instructions and schema const/enum/pattern rules.
   Protect source, semantic identities and query text; reject unknown,
   ambiguous and wrong-scope aliases without whole-JSON text replacement.
2. [Run checkpoints and dispatch](../../../crates/clew/src/documentation/agent_jobs.rs):
   a map scoped by Work, run, role and encoding version, frozen for each call.
   Sequential short aliases need an append-only map extended without renumbering
   after grouped delivery. Deterministic domain-keyed aliases with collision
   rejection are an alternative; their different lengths need measurement after
   implementation. Persist before dispatch and regenerate identically on resume.
3. [Input/result recovery](../../../crates/clew/src/documentation/agent_jobs/recovery.rs):
   add a frozen projection record bound to the existing canonical input identity,
   exact model payload/schema, map and encoding version. The canonical request
   can remain unchanged in its retained record; the opted-in wire carrier also
   needs an exact durable identity.
   Preserve a delivered wire result before alias decoding can fail, so invalid
   output does not cause a second paid call. Expose canonical results to existing
   answer/review checks while retaining the exact wire result and decoding status.
4. Opt-in role configuration and transport compatibility: keep existing drivers
   and saved runs on their current canonical path. Admit a versioned forwarding
   contract; do not add unknown fields to a legacy closed job or silently weaken
   its packet validation. Driver inventory/config identity must bind the mode.
5. A supported serializer/driver contract that validates canonical input first,
   forwards the prepared payload and strict schema, and returns wire output plus
   independent usage metadata. Host tests must exercise this path with a
   deterministic driver; an actual expansion case later qualifies model behavior.

Operation author/reviewer schema generation and exact saved-input replay are
additional callers that must agree with these components. Allowlist-only
binding aliases are smaller, but do not qualify the measured broader saving.
The existing result-save ordering must be preserved rather than moving a
fallible decoder ahead of durable result storage.

## Disposition and next result

Defer implementation from this bounded assessment and the immediate release.
The reference-token benefit is meaningful; neither old-driver rejection nor the
small byte percentage is a reason to discard the direction. The remaining work
crosses projection, checkpoint/recovery, configuration and driver admission;
adding only an unused codec or changing a private serializer would leave the
product outcome unverified. The estimated complete change touches a new codec,
call/checkpoint and recovery code, role/reply schemas, admission, driver-facing
API/example and tests: roughly 6-9 production files plus integration coverage.
This is feasible as its own implementation increment, but exceeds a quick
serializer-only addition to the accepted scoped-read release. Keep that increment
independent; the deferral applies to release bundling, not feasibility or value.

The next alias implementation slice should prepare the host-side representation
and exercise one compatible deterministic driver through dispatch, expansion and
resume. Required regressions cover stable aliases, role/call isolation, schema
agreement, preserved source, canonical answer/reviewer binding, unknown aliases,
and a delivered invalid alias response that is retained without redispatch.
Do not repeat the successful offline leaf round trips without a material change.
Use focused host tests first; full CI and official installation remain release
gates. No paid model call is authorized by this assessment.

Another measured representation opportunity is the reviewer's 16,158-byte
`savedAuthorContract` (about 29% of its prompt), including policy and native
schema representations. Its size does not prove that all of it is redundant or
removable. Priority between these slices belongs to the research chat; neither
an alternative implementation nor a broad protocol rewrite was started here.

## Supported native candidate (2026-10-06)

A subsequent source candidate implements the complete opt-in host path for
native method authoring 1.6 and its independent reviewer. The
[compatible serializer contract](../../operations/model-id-serializer.md)
provides the public pure Rust API and executable driver example. Canonical
jobs, Work, packets, saved answers and reviews remain canonical. A frozen
versioned carrier and typed, role-scoped map are retained before dispatch;
grouped expansion extends the existing map. Parsed driver JSON and any adapter
failure are retained before fallible Reply validation or identity decoding.
Recovery consumes this record without dispatching again, including unknown or
foreign-scope aliases and valid JSON from a failed driver. Malformed JSON remains
an adapter failure; byte-for-byte stdout retention is not claimed.

The same two retained first-call jobs were passed through the actual native
`prepare` and `forward_model_input` APIs. Their original Work and approved
author/reviewer run scopes were retained. The public serializer example then
produced exactly the prepared model payload and schema for both carriers. The
frozen prompt formatter and pure strict-provider schema projection were applied
after canonical validation. The baseline projection equals the saved provider
schema exactly; JSON property ordering gives the same baseline token counts.

| Actual representation | Role | Prompt bytes | Prompt reference tokens | Strict-schema bytes | Strict-schema reference tokens | Combined reference tokens |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Canonical retained baseline | Author | 36,605 | 9,525 | 5,647 | 1,555 | 11,080 |
| Native prepared model input | Author | 35,726 | 8,970 | 5,688 | 1,581 | 10,551 |
| Canonical retained baseline | Reviewer | 55,769 | 14,378 | 3,901 | 1,258 | 15,636 |
| Native prepared model input | Reviewer | 54,329 | 13,565 | 3,923 | 1,286 | 14,851 |

The joint reference-text reduction is **26,716 to 25,402: 1,314 tokens, or
4.9184%**. Author reduction is 4.7744%; reviewer reduction is 5.0205%. The
complete prepared encoding guidance and changed schema are included. Typed
aliases contain a scope tag, and strict-schema sizes increase slightly. This is
smaller than the earlier 10.26% prospective upper bound; the implementation does
not promise that bound or optimize away scope isolation to approach it.

The author map contains 20 entries and 2,759 bytes; the reviewer map contains
29 entries and 3,908 bytes. Complete serialized carriers are 80,371 and 118,031
bytes, versus model input payload/schema pairs of 40,513 and 57,858 bytes. Maps
and unchanged canonical jobs stay outside model input, but conservative host
admission and reservation still account for the complete carrier and configured
overhead. Operators may need higher explicit role byte caps and budgets. These
byte admission bounds do not measure provider tokens or billing.

The actual canonical jobs and all 96 author / 146 reviewer protected leaf values
checked remain exact, including source text/tokens, semantic symbols, names,
paths, evidence and query selections. Counts use the same `tiktoken` 0.14.0
reference `o200k_base` encoding; target-model encoding remains unresolved. Chat
framing and provider schema wrappers are excluded on both sides. This two-job
result establishes neither general or grouped-expansion savings nor model
quality, billing or actual provider delivery. No model calls, capture or refresh
were performed.

Six pure codec regressions and four actual native host scenarios passed. Host
coverage includes grouped author/reviewer expansion with canonical saved
results, unknown/foreign-scope alias failure without redispatch, missing map-head
publication plus raw-result crash recovery, and a retained driver-exit failure
that cannot become a successful expansion. The grouped-review fixture initially
failed conservative carrier admission; increasing only that synthetic fixture's
explicit cap/budget made the focused rerun pass. Production bounds were retained.
Legacy role serialization and saved-author replay tests also passed. Integrated
release qualification remains pending; this candidate is not in `0.13.15`.


## Compact native delivery follow-up

A later source candidate adds explicit `codeclew-model-ids/1.1`, retaining
version 1.0's frozen behavior. It verifies complete native presentation with the
existing pure builder, then excludes duplicate raw delivery arrays solely from
model input. Canonical archives, presentation, receipts, citations and separate
author/reviewer deliveries remain intact.

The [actual grouped comparison and compatibility checks](../../plans/question-results-and-incremental-evidence.md#compact-role-evidence-representation-candidate-2026-10-06)
record both initial and expanded calls on identical canonical jobs/scopes.
The complete four-call generated Java fixture decreases prompt plus strict-schema
reference counts 163,106 to 104,051 (36.2065%), including protocol; expanded
calls alone decrease 113,714 to 69,559 (38.8299%). The first author call without
duplicate delivery grows by 18 reference tokens (0.2547%).
These are reference text counts, not general production, provider billing or
quality results. The previous actual 4.9184% measurement remains specific to
version 1.0's two retained real first-call jobs. Frozen 1.0 carriers remain
compatible; integrated release qualification is pending.
