# CLI documentation generator

Static source interpretation. Declared interactions do not establish runtime routing.

## Overview

The script maintains a source-linked navigation walkthrough. Before either branch, it verifies the recorded Git revision, source file and fragment digests, exact source text and line ranges, source URLs, and graph references. With --check, it compares expected DOT and Mermaid with saved files, parses the saved SVG, and checks that the SVG and rendered claims appear in the HTML page, then returns. It calls Graphviz as dot -Tsvg only in render_svg, reached by the branch without --check; that branch writes diagrams and replaces the page&#39;s diagram and claim sections. The check validates retained byte bindings and generated presentation, not the meaning of the agent-inferred claims. This selected source does not establish that a particular run passed or that the published diagram describes runtime execution. Syntax capture leaves call targets unresolved and provides lexical ordering only. In the explicitly labeled caller-owned local maintenance exercise, --check also rejects saved XML whose root is not an SVG element. This change is separate from the released Codeclew source. The unchanged generation branch still invokes Graphviz only without --check.

Source freshness: UNVERIFIED. Meaning review: UNASSESSED.

## Responsibilities

This script verifies the byte bindings of recorded claims and generates their diagram and HTML presentation. It checks exact committed source, retained source digests and references; it labels the graph as agent-interpreted static flow. It does not determine whether the claims are semantically true or establish runtime behavior. Graphviz contributes SVG rendering on the generation branch. These responsibilities describe the selected source file; they do not establish deployed ownership or a successful run.

Source freshness: STALE. Meaning review: UNASSESSED.

## Domain entities

Identify domain concepts separately from DTOs and storage classes; ownership needs an explicit declaration.

Documentation gap: source-bound section content has not been accepted.

## Ingress contracts

Document discovered public boundaries and declared contracts; retain gaps for unsupported discovery.

Documentation gap: source-bound section content has not been accepted.

## Egress contracts

Document external calls and messages; unresolved destinations and runtime activation remain gaps.

Documentation gap: source-bound section content has not been accepted.

## assessment-walkthrough-owner-guidance

Documentation gap: Outside the requested work entrypoint.

## cli-documentation-4c4e5646fc3afe399640

Documentation gap: Outside the requested work entrypoint.

## cli-documentation-53b99d0f35fa55badac7

Documentation gap: Outside the requested work entrypoint.

## cli-documentation-672ee5015ada81ec8d40

Documentation gap: Outside the requested work entrypoint.

## cli-documentation-6d1878f45ec2b985c9a6

Documentation gap: Outside the requested work entrypoint.

## cli-documentation-7368fc134a99e3cdace1

Documentation gap: Outside the requested work entrypoint.


## Human note: Owner guidance for the walkthrough

Classification: intention. Period: Practical public documentation, 2026-10-05.

Original captured text (human/imported, unverified):

<pre># Owner guidance for this walkthrough

Preserve the byte-binding verification explanation and the responsibilities section when updating the overview. Label the follow-up SVG-root validation as a caller-owned local maintenance exercise, not a feature of the released Codeclew source. Runtime behavior and semantic correctness of agent-inferred claims remain outside this note.
</pre>

<details><summary>Original metadata and associations</summary><pre>{
  &quot;classification&quot;: &quot;intention&quot;,
  &quot;id&quot;: &quot;walkthrough-owner-guidance&quot;,
  &quot;metadata&quot;: {
    &quot;authority&quot;: &quot;DECLARED_EXERCISE_OWNER_GUIDANCE_NOT_SOURCE_PROOF&quot;,
    &quot;origin&quot;: &quot;public-reproduction-input&quot;
  },
  &quot;path&quot;: &quot;notes/walkthrough-owner-guidance.md&quot;,
  &quot;period&quot;: &quot;Practical public documentation, 2026-10-05&quot;,
  &quot;schema&quot;: &quot;codeclew-documentation-note-association/1.0&quot;,
  &quot;service&quot;: &quot;cli-documentation&quot;,
  &quot;tags&quot;: [
    &quot;maintenance-exercise&quot;
  ],
  &quot;targets&quot;: [
    &quot;service:cli-documentation/section-overview&quot;
  ],
  &quot;title&quot;: &quot;Owner guidance for the walkthrough&quot;
}</pre></details>

Separate agent assessment: UNASSESSED.

