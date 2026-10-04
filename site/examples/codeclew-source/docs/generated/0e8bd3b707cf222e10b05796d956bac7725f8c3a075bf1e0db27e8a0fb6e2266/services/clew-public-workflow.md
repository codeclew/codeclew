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

Describe responsibilities and exclusions from evidence; business intent may need a human note.

Documentation gap: source-bound section content has not been accepted.

## Domain entities

Identify domain concepts separately from DTOs and storage classes; ownership needs an explicit declaration.

Documentation gap: source-bound section content has not been accepted.

## Ingress contracts

Document discovered public boundaries and declared contracts; retain gaps for unsupported discovery.

Documentation gap: source-bound section content has not been accepted.

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

