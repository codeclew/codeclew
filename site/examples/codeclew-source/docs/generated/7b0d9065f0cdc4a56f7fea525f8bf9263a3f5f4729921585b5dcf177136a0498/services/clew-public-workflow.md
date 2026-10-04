# Use Clew to create and maintain source-linked documentation

Static source interpretation. Declared interactions do not establish runtime routing.

## Overview

Clew&#39;s documentation interface supports a separate documentation repository containing service declarations, captured source evidence and explanations linked to that evidence. It separates source freshness from review of an explanation&#39;s meaning.

The supplied installer accepts CODECLEW_VERSION=v0.13.7, verifies archive checksums and package/CLI version agreement, selects a launcher under CODECLEW_BIN_DIR, and invokes capabilities. Supported platforms are macOS arm64/x86_64 and Linux x86_64, including WSL2; Linux requires glibc 2.35 or newer. Use the installed launcher explicitly when a source checkout is also present: the checkout&#39;s clew script rejects a bare upgrade command.

A first service uses clew docs init --root DOCROOT, docs service add --root DOCROOT --input service.json, docs bind --root DOCROOT --service ID --repo CHECKOUT, and docs check --root DOCROOT --service ID. Service records identify the repository, targetRef, language and profile, with source roots/dialect or compilation selectors. Check dispatch requests capture and reports freshness; source records retain revision, file, lines, text and optional URLs.

For a service explanation, docs work prepare --root DOCROOT --subject service:ID --input request.json selects saved evidence. Work retains admitted external inputs and notes and records bounded reads. docs work run --root DOCROOT --work WORK --config execution.json uses configured author/reviewer drivers with finite call and budget limits. Checked proposals receive meaning review; an APPROVE result leads to reviewed publication. Manual proposal publish --unassessed preserves UNASSESSED meaning review.

For maintenance, docs check requests new evidence; docs changes compares saved evidence with a publication baseline, and docs refresh --status-only dispatches freshness publication while retaining explanations. Inspect docs work status for failures or recovery limits before preparing replacement Work.

Source freshness: UNVERIFIED. Meaning review: VERIFIED_WITH_LIMITATIONS.

## Responsibilities

Clew&#39;s captured CLI exposes documentation-root initialization, service registration and checkout binding. Users supply repository/targetRef, language/profile and source roots/dialect or compilation selectors. docs check requests selected-service capture and reports freshness; compatible saved siblings may remain unverified.

docs work prepare selects saved evidence without source capture. Immutable Work binds the request, snapshot, retained narrative, admitted external inputs and notes, handles and influence. Reads are bounded and recorded; missing required inputs or incomplete initial reads block generic authoring.

Generic docs work run uses user-configured author/reviewer drivers, models and finite budgets. Clew reserves calls, checks reply identity, validates proposals and requests meaning review. Rejection uses bounded repair/fallback; APPROVE permits reviewed publication after evidence, read and baseline checks. Missing trusted usage retains maximum charges; reported excess denies further calls. Driver execution is delegated to agent_adapter.

Failure finalization records CANCELLED, EXHAUSTED, NEEDS_EVIDENCE or GENERATION_GAP with recovery guidance and releases unused reservations when configuration was admitted. It generally requests a GENERATION_GAP publication through render with no new narratives. Cancellation and specified input-cap, author-contract and initial-source-budget failures skip that request; recovery refusals return separately.

Manual proposal publish --unassessed retains UNASSESSED meaning review. docs changes compares saved evidence with the publication baseline without publishing. docs refresh --status-only delegates freshness publication while retaining explanations. Users initiate recapture, review affected content, restore inputs/configuration and inspect docs work status. Source freshness, meaning review and runtime truth remain separate.

Source freshness: UNVERIFIED. Meaning review: VERIFIED_WITH_LIMITATIONS.

## Domain entities

The documentation repository is the user-owned home for service declarations, saved evidence and explanations. A Service declaration identifies the documented service through id and repositoryId, selects repository and targetRef, and specifies language/profile with source roots/dialect or compilation selectors. LocalBinding associates that service with a checkout; machine paths are not portable authored identities.

Saved Check snapshots supply evidence to Work without recapture. ServiceEvidence groups revision, coverage, boundaries, observations and sources. Each Source records text, revision, file, lines, digests and an optional URL. Its logical id survives relocation; SourceOccurrence identifies immutable snapshot/blob byte ranges.

Work is an immutable authoring package binding subject, request, saved snapshot, retained narrative, external inputs, handles and influence digests. Its identity is derived from its saved manifest. References such as section3 and source/dependency handles resolve within that Work; they differ from the underlying object IDs. Bounded read receipts record supplied content, omissions and continuation.

A Proposal supplies operations, gaps and uncertainties. Submission materializes source-linked claims and a Narrative, records diagnostics and derives the saved proposal identity from its canonical record. Deterministic readiness is separate from meaning review. Generic publication requires a validated APPROVE result bound to proposal, evidence and reads; manual publication retains UNASSESSED review. Publication receipts bind bundle identity and output hashes. Source freshness remains separate from meaning review and runtime truth.

Execution configuration selects author/reviewer drivers and optional fallback, finite calls, repairs, expansions, per-call caps and a budget account. Reservations bind run and role, remain outside disposable Work state, and retain maximum charges for missing trusted usage.

Source freshness: UNVERIFIED. Meaning review: VERIFIED_WITH_LIMITATIONS.

## Ingress contracts

These source-declared commands use clew docs and --root DOCROOT.

service add --input service.json reads id, repositoryId, repository, targetRef, language and profile, with source.roots/source.dialect or compilations for source selection. bind --service ID --repo CHECKOUT delegates checkout binding. check --service ID requests capture and returns a saved snapshot; compatible saved siblings may remain unverified.

work prepare --subject service:ID --input request.json [--snapshot SNAPSHOT] uses saved evidence, defaulting to the latest saved check without capture; scenario:ID is also supported with selector checks. Request schema is codeclew-documentation-work-request/1.0, with audience and optional entrypoint; maxItems is 1..100 and maxBytes 2048..49152. documentationLanguage or --language selects en/ru; conflicts fail. Work captures protected notes and selected root-relative externalInputs as unverified human/imported text.

work read or expand --work WORK --input selection.json takes up to eight references, eight symbols, or one query. Continue with the same selection and returned cursor. Symbol selectors require a unique captured declaration; SOURCE inventory requires registered service source scope. Navigation rows require full recorded reads before citation; oversized sources use work read-part.

proposal submit --work WORK --input proposal.json checks operations, gaps and uncertainties. proposal publish --proposal PROPOSAL --unassessed retains UNASSESSED meaning review. work run --work WORK --config execution.json selects author/reviewer and optional fallback roles, with adapter, model, command, runtimeReads, environment, network and caps, plus finite calls, repairs, expansions and budget. Missing required inputs or incomplete initial reads block generic authoring.

Source freshness: UNVERIFIED. Meaning review: VERIFIED_WITH_LIMITATIONS.

## Egress contracts

Document external calls and messages; unresolved destinations and runtime activation remain gaps.

Documentation gap: source-bound section content has not been accepted.

## assessment-public-reader-intent

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-0acc80a387b5f47e328b

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-11a7847fdafad548b122

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-3e9a88338b50c1f17975

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-4e94ba14682b266f7ef8

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-5b7a6fc56854bd748d79

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-5be104cfadcf253f3310

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-69e80ad8809db9ab9864

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-7b7904b8c07893dc914d

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-a68fb9be44ec87fadbe2

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-b4376eab32aa5315c108

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-c1e42fad75554a96627c

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-c87aaf89860add1a3f1a

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-d0d8b869a75d8a66a08e

Documentation gap: Outside the requested work entrypoint.

## clew-public-workflow-f10c7bc7b8d207312793

Documentation gap: Outside the requested work entrypoint.


## Human note: Public reader intent

Classification: intention. Period: Codeclew 0.13.7 public documentation.

Original captured text (human/imported, unverified):

<pre># Public reader intent

Declared by the Codeclew documentation agent for the user-authorized public documentation task. This is task context and an intention, not implementation or runtime proof.

Audience: a developer installing the released Clew and using it to create and maintain source-linked documentation for a repository.

Explain the first successful path and help the reader select a supported workflow. Cover installation and installed-release selection; registered source capture; authoring, meaning review and publication; reading source links; freshness and recovery. Use only commands and behavior supported by the retained source packet. Give explicit, actionable limits when delegated implementation, compiler scope, deployment, provider behavior or runtime effects are not proved by that packet.

Use the five native service sections to answer the reader&#39;s practical questions. Favor concrete supported inputs, commands, outcomes and next actions over implementation inventories. Keep source authority, model meaning assessment and currentness separate. Treat this note as a declared reader goal; derive the actual explanations from the captured source.
</pre>

<details><summary>Original metadata and associations</summary><pre>{
  &quot;classification&quot;: &quot;intention&quot;,
  &quot;id&quot;: &quot;public-reader-intent&quot;,
  &quot;metadata&quot;: {
    &quot;author&quot;: &quot;Codeclew documentation agent&quot;,
    &quot;authority&quot;: &quot;DECLARED_TASK_CONTEXT_NOT_SOURCE_PROOF&quot;,
    &quot;origin&quot;: &quot;agent-proposal&quot;
  },
  &quot;path&quot;: &quot;notes/public-reader-intent.md&quot;,
  &quot;period&quot;: &quot;Codeclew 0.13.7 public documentation&quot;,
  &quot;schema&quot;: &quot;codeclew-documentation-note-association/1.0&quot;,
  &quot;service&quot;: &quot;clew-public-workflow&quot;,
  &quot;tags&quot;: [
    &quot;public-documentation&quot;,
    &quot;reader-task&quot;
  ],
  &quot;targets&quot;: [
    &quot;service:clew-public-workflow&quot;
  ],
  &quot;title&quot;: &quot;Public reader intent&quot;
}</pre></details>

Separate agent assessment: UNASSESSED.

