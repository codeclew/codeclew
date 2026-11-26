---
name: codeclew
description: Use Codeclew for bounded compiler- or syntax-backed code context, safe changes, multi-repository analysis, freshness checks, recovery, and privacy-safe diagnostics. Trigger when a task asks to inspect, explain, trace, or change supported source code through the clew CLI, or to diagnose a Codeclew run. Development of Codeclew itself follows its repository-native contributor workflow.
license: Apache-2.0
metadata:
  author: codeclew
  version: "0.15.22"
  repository: https://github.com/codeclew/codeclew-skill
---

# Codeclew

## Prepared documentation author

If the runner supplied a documentation packet, it contains the request,
selected retained source and permitted citations. Follow that prompt and respond
in the supplied schema. The runner admits the release and assembles and verifies
the draft. Use the workspace workflow when the runner supplies a workspace prompt.

## Choose the requested result

Choose by the requested result, not the presence of a root/snapshot:

When drafting HTTP documentation, read [HTTP closure](references/http-reader-closure.md)
**before drafting**: each evidenced method/path needs its own page and
handler-started sequence.

| Result | Read next |
| --- | --- |
| Generate durable service or process documentation | [Standard full documentation job](references/documentation-runner.md#new-full-documentation-sets) |
| Update existing service or process documentation | [Update the prior original and checked export](references/compose-original.md#update-an-existing-checked-export) |
| Bounded SOURCE Q&A or saved-answer maintenance | [Prepared SOURCE](README.md) |
| Inspect shared semantic candidates across retained service caches | [Shared catalog inputs](references/shared-semantic-catalog.md) |

For a new full documentation set, fill a bounded runner job with
`"evidenceMode": "PREPARED_FACTS"` and `"authorMode": "ISOLATED_PAGES"`.
Use the installed skill's `prepare` and `run` commands to assemble originals
and check local HTML with the packaged renderer. A new repository first needs
[registration and capture](references/service-documentation.md#start-or-recover).
For an update, locate the previous checked export and its original, bound index
and citation manifests. Preserve agreed edits and revise only the requested
fragments. Completion requires the requested pages and sequences, checked
provenance, and each author's scope review.

After a full documentation run reaches READY, compare the operation, process
map or route, and guide where they describe the same behavior. Use the retained
source for those pages. Follow a local committed change through any later
external action or failure, then check what a later attempt can observe, skip,
or repeat. Distinguish a missing input from a present input that fails when
decoded, validated, or handled. Record only the order, outcomes, and boundaries
the source supports.

If the pages agree with the source, return the checked site. For a specific
omission or contradiction, use [reviewed page correction](references/documentation-runner.md#correct-a-reviewed-isolated-page).
Supply the exact selector for the affected field and its supporting source
aliases through `--review-findings`. Keep the prior site and history and report
new author usage separately.

## Prepared SOURCE shortcut

For bounded SOURCE Q&A/maintenance with root, service, exact selector and saved
snapshot or answer/evidence, read only [README.md](README.md). It contains the
complete short route. With a previous full packet and exact target, compare
directly; no preliminary packet metadata, Git/hash read or authoring references.
Reuse an already verified unchanged answer/evidence without another context fetch.

## Other workflows

Load only the reference needed at the current step:

[Managed code](references/managed-code.md) ·
[Acquisition](references/service-documentation.md#start-or-recover) ·
[Threads](references/thread-preparation.md) ·
[SOURCE parts/batches](references/source-read-parts.md) ·
[Library sources](references/library-sources.md) ·
[Failed Check/support](references/documentation-evidence.md)

Bounded model Q&A retains validated answer/evidence JSON; it does not publish documents.
For a description of saved Work, read its context and sources, then use the same
authored-originals route. No separate Work document renderer is required.

## Runtime and authority

For installed-product work resolve the installed `clew` once; never substitute a
source launcher, capsule binary or runtime seed. Do not edit CODECLEW_HOME objects.
Docs commands perform admission; a managed-session loop is not an extra docs step.
Codeclew product development follows its repository-native contributor workflow.

Preserve scope, exact refs, originals and source bindings. Keep coverage, omitted/
unresolved evidence and cursors visible. Syntax/declarations are not compiler proof
or observed runtime behavior. Capture success and publication freshness differ.
Keep full evidence local and share only within the user's authorization.
