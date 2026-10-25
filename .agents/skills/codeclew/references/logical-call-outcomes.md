# Preserve HTTP and logical port returns and failure paths

Use this rule for explicitly registered HTTP and logical storage, publisher or
SDK boundaries. Native READY does not establish that the diagram explains the
complete request/outcome exchange.

## A caller-observed HTTP response requires a reverse arrow

If selected source receives a synchronous HTTP response, show its explicit
`declared-boundary-return` on the normal-call branch, with the same interaction
and reversed participants. Unknown receiver implementation or remote response
schema does not erase that supported response. A local completion note, status
check or returned caller DTO cannot replace the boundary return arrow.

For synthetic `HttpResponse<String> response = client.send(request,
BodyHandlers.ofString())`, the fragment below belongs inside the source-bound
normal-send branch. The destination and HTTP interaction must already be
registered and delivered to this Work. It is not a complete Proposal:

```json
{
  "participants": [{"id":"risk","label":"Risk API","externalService":"risk-api"}],
  "steps": [
    {"kind":"declared-boundary","from":"orders","to":"risk",
     "interaction":"EXACT_HTTP_INTERACTION_REF",
     "meaning":{"text":"Declared HTTP request; receiver implementation and runtime delivery are unverified.","evidence":["EXACT_HTTP_INTERACTION_REF"]}},
    {"kind":"declared-boundary-return","from":"risk","to":"orders",
     "interaction":"EXACT_HTTP_INTERACTION_REF",
     "meaning":{"text":"Declared response on normal HTTP completion; remote schema and runtime execution are unverified.","evidence":["EXACT_HTTP_INTERACTION_REF"]}},
    {"kind":"note","from":"orders",
     "meaning":{"text":"The caller receives HttpResponse<String>; source status/body checks follow this return.","evidence":["EXACT_HTTP_CALL_SOURCE_REF"]}}
  ]
}
```

Keep declaration-only evidence on both boundary arrows and the caller-observed
type/status/body behavior in separate source-bound notes or branches. Position
the return before the caller's status check: even a returned non-success status
is a response, followed by local rejection or fallback if source supports it.
An escaped send failure has no response arrow; a parse failure occurs after the
response and skips later effects unless explicitly handled. Missing callsite
binding remains visible and does not turn an authored declaration into runtime
proof. Do not invent response fields, statuses, receiver behavior or retries.

## Logical API completion is separate from remote effects

For **every synchronous logical port call**, show both `declared-boundary` and an
explicit reverse `declared-boundary-return` for its supported normal API return.
Use the same delivered interaction handle and reversed participants. Both boundary
meanings cite **only delivered DECLARED_INTERACTION evidence**. Keep source-derived
boolean/void/value behavior in a separate source-bound note or branch. A note
about completion cannot replace the reverse arrow required by this sequence.

The declaration arrow labels a local API boundary/outcome with runtime unverified;
it must not claim remote execution, persistence, commit or delivery. If source
behavior is not known, keep the outcome precisely unknown rather than inventing
one. Keep agent-authored declarations as `agent-proposal`.

## Tiny normal-path operation fragment

Assume synthetic source exposes a synchronous `boolean saved = store.save(value)`
call. The destination and logical interaction are already registered and delivered
to this Work. The following is a fragment inside an operation, not a complete
Proposal. Place it on the evidenced normal path; surrounding guards and escaped
errors still need their own source-bound branches.

```json
{
  "participants": [{"id":"store","label":"Logical storage","externalService":"reservation-store"}],
  "steps": [
    {"kind":"declared-boundary","from":"orders","to":"store",
     "interaction":"EXACT_DECLARED_INTERACTION_REF",
     "meaning":{"text":"Declared logical API request; receiver execution and physical transport are unverified.","evidence":["EXACT_DECLARED_INTERACTION_REF"]}},
    {"kind":"declared-boundary-return","from":"store","to":"orders",
     "interaction":"EXACT_DECLARED_INTERACTION_REF",
     "meaning":{"text":"Declared normal local API return; remote completion and durability are unverified.","evidence":["EXACT_DECLARED_INTERACTION_REF"]}},
    {"kind":"note","from":"orders",
     "meaning":{"text":"On normal completion the caller receives a boolean. This does not establish commit or durable storage.","evidence":["EXACT_CALL_SOURCE_REF"]}}
  ]
}
```

Use the actual service/participant identities and this Work's evidence handles.
For a `void` synchronous publisher port, the reverse arrow still represents only
normal local API completion. Its source note states that no result value is
exposed; void completion is neither broker acknowledgement nor consumer/business
response. Preserve actual boolean handling: false may be ignored, branch or
propagate according to source; do not invent a success test.

`declared-boundary-return` remains forbidden for Kafka interactions. A logical
wrapper's local API return and a Kafka broker/consumer event are different facts.
An independently evidenced reply event needs its own interaction.

## Dispatch, HTTP guards and errors stay separate

- For genuinely unjoined asynchronous work, do not draw a synchronous business
  completion before the caller's response. Distinguish the source-exposed local
  submission/dispatch completion or returned future from completion of the
  background task. Show only an evidenced local dispatch outcome and an explicit
  unknown for unobserved background/remote completion. Preserve a required join
  where source actually waits; a fire-and-forget call must not acquire one.
- Preserve HTTP evaluation order. If source inspects `response.body()` only under
  `status == 200`, keep the body/null guard inside that status branch, with parsing
  after its actual body guard. Do not hoist it to every response or invent other
  accepted statuses.
- An uncaught send failure occurs before a response is available: its branch
  cannot pass through the normal response/body/parse/success path. An uncaught
  parse failure similarly skips subsequent store/publish and success steps unless
  source explicitly handles it. End each escaped-error branch with its evidenced
  propagation/outcome; do not merge it visually into success. A documented
  exception is not automatically a remote HTTP status, local catch or retry.

Before review, reconcile each supported HTTP response and synchronous logical
request with its reverse interaction/participants and local source outcome;
inspect boolean/void limits,
actual status/body guards and escaped-error paths. Preserve unknowns and source
authority. A machine-ready sequence that replaces required returns with notes,
loses effects or sends failures into success remains semantically unacceptable.
