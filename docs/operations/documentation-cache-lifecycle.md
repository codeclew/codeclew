# Documentation cache lifecycle

Version 0.10 requires fresh SQLite-only documentation roots. Old loose objects,
inline Check/Work and older narrative/binding formats are unsupported. Reindex
into a new root; the product does not migrate or delete old state.

Operational design and runbook for the normalized docs-local cache. This
documents the first storage slice implemented under
`20260916-docs-cache-normalization-v2` and separates implemented scope from
explicitly deferred scope. It is an operational reference, not an authorization
to delete or migrate user data.

## Implemented: immutable content-addressed object store

Heavy documentation evidence is now persisted once by content identity under
the SQLite database selected by `.codeclew/cache/object-layout.json`.
Objects are immutable and content addressed:

- `documentation::cache::put` stores a payload and returns a validated
  `ObjectRef { schema, digest, size }`. Byte-identical payloads share one object
  regardless of producer/admission key or schema, because payload identity is
  the content digest and provenance is a reference attribute.
- `documentation::cache::get` reads and verifies a reference, honoring a
  per-read bound. Missing objects are a distinct miss (`Ok(None)`); corrupt,
  size-mismatched, unsafe, or oversized objects are explicit typed errors, never
  silently forged evidence.
- `documentation::cache::verify` and `owned_digests` support validated reads and
  bounded read-only enumeration.

## Implemented: reference envelopes

Capture and check records are small reference envelopes, not heavy serialized
copies.

- Service captures (`analysis.rs`) persist a `CaptureManifest` at the keyed
  cache path; the heavy `sources`, `observations`, and `contracts` payloads live
  once in the object store. `cache::load_capture` hydrates a `ServiceEvidence`
  back through the verified store.
- Checks (`check.rs`) persist a `CheckManifest` to
  `.codeclew/cache/latest-check.json`: per-service capture manifests plus one
  dependencies object. `Check::load` hydrates a full `Check`. This removes the
  prior inline duplication of every observation in `Check.dependencies`.

## Implemented: enabled regressions

`crates/clew/tests/docs_cache_regressions.rs` and
`documentation::cache::tests` verify, with enabled tests:

- byte-identical evidence stored once across producer keys;
- capture reference-envelope round-trip and corruption rejection;
- check persisted as a small reference envelope that hydrates back;
- missing/corrupt/oversized/malformed object behavior as explicit typed errors;
- same-path/different-compilation authority round-trips without overwrite and
  without flattening provenance;
- capture reuse skips the producer and rejects damaged captures;
- reuse-key identity binding forces recapture on source-state change;
- ten unchanged reuse sequences keep heavy-object count/bytes stable.

## Implemented: validated capture reuse

The deterministic source-syntax extractor is a fully admitted stable input: its
capture is keyed by the resolved revision, service declaration digest,
extractor identity, and language, persisted as a keyed reference envelope, and
reused when still valid. `load_capture_if_valid` verifies every referenced
object before returning evidence, so a stale or damaged capture is never
silently reused; corruption is an explicit error. Maven/external-state inputs
remain non-cacheable (always recaptured) unless complete build/settings/
dependency authority is proven, so no unsafe reuse is introduced.

## Implemented: publication reference closure (source pruning)

`make_bindings` retains only the complete source closure reachable from the
retained observation references (the fragment dependency closure). Unreferenced
sources are excluded while every source a current claim resolves to is
preserved at its exact version, reducing retained-source breadth without
dropping referenced snippets.

## Implemented: read-only inventory

`documentation::cache::inventory` is a read-only API that reports owned object
count/bytes, keyed manifest bytes, retained publication generations and bytes,
and the current bundle root. It never deletes or migrates data and never
reports an unknown or active root as safely deletable. `owned_digests` bounds
enumeration.

## Deferred (not implemented in this run)

These are explicitly out of scope and must be designed and approved separately
before enabling:

- **Automatic retention and physical deletion.** No docs-local GC, pack
  compaction, automatic publication expiration, or user-cache cleanup is
  implemented. The generic private CAS GC does not manage docs-local state.
- **Read-only inventory CLI / lease ownership / reclaimable marking.** The
  `inventory` library API exists, but no `docs cache inventory` CLI subcommand,
  per-attempt lease ownership, or reclaimable-object marking is implemented.
- **Shared heavy evidence across publication generations.** Bindings still
  serialize source/observation records inline; they are not yet published as
  object references that generations share via the immutable store.
- **Maven/external-state capture reuse.** Non-cacheable recapture remains the
  default for Maven/external-state inputs; verified reuse for those inputs is
  not introduced.
- **Customer migration.** Existing customer caches are not migrated; their size
  is not automatically reduced by these changes.

Until retention/deletion is implemented and approved, no live cache data is
deleted or rewritten. The object store is additive: new captures write objects
and small envelopes alongside current-format retained state.

## Implemented: multi-compilation selection and scope-aware evidence

One documented service may now select several explicit same-repository
compilation scopes without inventing separate services.

- `Service.compilations` is a plural selector set (e.g. `:/web:main`,
  `:/flow:main`, `:/common:main`). Use a one-element list for one scope.
  The former singular field is rejected. Empty native selections, duplicates
  and more than 128 selectors are typed validation errors.
- Capture tags each fact with its admitting compilation scope, including when one
  scope is selected, and projection retains scope-distinct observations and
  source records instead of last-write-wins overwriting. A symbol admitted under
  incompatible scope candidates becomes an explicit `SCOPE_AMBIGUOUS` boundary;
  identical candidates across scopes are not ambiguous.
- Cross-module call chains (web → flow → common) surface the matching admitted
  bodies through the compiler's exact resolution; dynamic/external dispatch
  stays an explicit boundary.

## Implemented: indexed membership and atomic fact deltas

`documentation::fact_index` adds a versioned paged membership/index layer over
the immutable object store, without a competing database or uncached dependency.

- A scope-aware `OccurrenceKey` (repository/revision/source state/scope/domain/
  semantic identity) binds a `FactOccurrence` to a content-addressed payload
  `ObjectRef`. One payload object may carry many contextual memberships and is
  persisted once; identical symbols under incompatible scopes never collide.
- Entries are partitioned into 64 deterministic bucket pages. A point lookup
  reads one page; an upsert/remove (`apply_delta`) copies only the affected page
  and the small root manifest, sharing unchanged pages and never scanning every
  payload. The current root is published by one atomic write under the docs
  lock, so a crash leaves the previous complete snapshot current.
- `replace_scope` authorizes absence only on a successful complete replacement;
  an incomplete capture can never delete previously known facts. Logical removal
  is observable via `reclaimable` (unreferenced payloads/bytes) without any
  physical deletion.
- `documentation::access::RequestScope` provides request-scoped batched access:
  payload fetch/decode counts equal the unique required payload IDs, incoming
  writes are deduplicated by reference (unchanged reruns write nothing), and
  metadata IO and integrity verification are counted separately from hydration.

## Implemented: multi-compilation runbook and scale evidence

`docs/operations/multi-compilation-documentation.md` documents the synthetic
reactor configuration and egress guidance. Enabled tests in
`crates/clew/tests/multisource_evidence_regressions.rs` provide deterministic
storage-scale evidence: >=1,300 distinct generated source records and tens of
thousands of fact memberships prove unique-payload storage, one-page point
lookups, bounded single-fact deltas, and observable logical removal.

## Deferred (still not implemented in this run)

Physical GC, pack compaction, automatic expiry, real-cache migration, and
customer-cache operations remain deferred; no destructive cleanup is authorized.