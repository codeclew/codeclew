# Task context and practical public documentation

Status: implementation authorized on 2026-10-05; in progress.
Baseline release: v0.13.10, source revision
`e2138232b8437f889e7836710ea69f219e0cc3b0`.
Baseline public site: `4a732ce0972eb0be99f08be81b1a02ae421b85f8`.

## Intended outcome

An engineer can obtain a useful answer from adequate saved source context, request
coherent additional evidence when needed, independently review that answer, and
maintain a useful document without losing human contributions. The public website
should teach those practical tasks with fewer, clearly named pages and examples
that can be reproduced on the final installed release.

The prior UX release repaired selected reader and first-document defects. It did
not implement operation-author or operation-reviewer expansion, and its five
public self-documentation sections still explain a saved v0.13.7 source revision.
Those artifacts are historical evidence, not completion of this scope.

## Delivery sequence

1. Identify the actual operation-draft and independent-review context seams, the
   existing generic expansion coordinator, snapshot reuse and citation identity.
   Inventory every public route in
   [the page inventory](../product/public-page-inventory.json), with a reader,
   question, useful result, verification and disposition for each route.
2. Implement a task-relevant initial semantic package and real grouped expansion
   actions for both author and independent reviewer. Reuse retained snapshots,
   public selection mechanisms and coordinator paging/deduplication. Keep guards,
   null behavior, negation, argument origins, mutations before failure, exact
   sources and unavailable evidence explicit. Version changed frozen identities
   deliberately and retain readers for old results.
3. Test actual grouped reads, independent reviewer expansion, duplicate requests,
   recorded evidence, recovery, old result compatibility and snapshot isolation.
   Reuse existing no-reindex and unrelated-service preservation evidence, adding
   measurements only where the requested scenario is not already covered.
4. Qualify the new actual authoring path on the same owned tasks and evidence as
   the existing baseline. Compare semantic correctness and completeness alongside
   total actual input across initial author, expansions and review, elapsed time,
   context requests and compiler invocations. Record failed attempts separately.
   The existing 415099 CLI input tokens cover ten successful role calls for five
   questions, with zero CLI-reported cached input tokens. JSON byte reduction is
   not a token or billing reduction claim. Do not repeat closed paid arms merely
   to produce another report.
5. Review the chosen implementation, run relevant checks and the mandatory full
   final CI, publish the next release through the existing procedure and activate
   it using the supported installer. Record exact revision, version, release asset
   equivalence and installed capabilities.
6. Use that installed release to build current source-backed Clew documentation.
   Cover a first useful result, understanding code and values, understanding a
   change, updating a document without losing a human edit, and recovering from
   an error. Attribute operational instructions separately from source-derived
   explanations. Verify commands and source links against the selected release.
7. Implement the smaller public information architecture and rewrite every retained
   page for its practical reader. Replace or remove the raw working-tree example
   from the primary route. Preserve useful old URLs through redirects where
   appropriate; retain immutable publications as reference material. Publish and
   verify links, search, desktop/mobile/keyboard journeys and privacy.
8. Provide the result for independent research-chat inspection and resolve its
   concrete findings. Do not announce that independent acceptance occurred on
   behalf of the reviewing chat, or award an automatic quality score for CI.

## Implementation boundaries

No new RAG or MCP framework. Removing a prompt prohibition alone is not a tool
implementation. Do not invent fixed bytes, nodes or call ceilings without an
observed problem; existing explicit execution budgets still govern dispatch.
Do not rewrite old frozen results, silently recapture on export/question change,
overwrite another service or replace authored documents with hand-written mock
reader HTML. Preserve unrelated user changes. All implementation, review and
delegated work for this scope uses GPT-6.1 Sol with High reasoning.

## Reviewable artifacts and acceptance

The public-content detail lives in
[the practical content plan](practical-public-documentation.md) and the page
inventory. Code changes, measured context comparison, final release/install facts,
current native self-documentation and live page verification are separate results.
Each retained public page must answer a concrete engineering question, show a
verified command or example, make the useful result visible before technical
details, and provide an understandable next step. No page is accepted merely for
existing, passing syntax checks or increasing documentation coverage.

Execution status and remaining limitations will be updated as each result exists.
No final release or independent acceptance is claimed by this planning artifact.
