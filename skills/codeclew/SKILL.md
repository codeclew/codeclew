---
name: codeclew
description: Use Codeclew for bounded compiler- or syntax-backed code context, safe changes, multi-repository analysis, freshness checks, recovery, and privacy-safe diagnostics. Trigger when a task asks to inspect, explain, trace, or change supported source code through the clew CLI, or to diagnose a Codeclew run. Development of Codeclew itself follows its repository-native contributor workflow.
license: Apache-2.0
metadata:
  author: codeclew
  version: "0.14.7"
  repository: https://github.com/codeclew/codeclew-skill
---

# Codeclew

## Choose the requested result

A broad request to generate or update service/process documentation defaults to
[human-oriented authored pages](references/compose-original.md#frozen-source-to-reviewed-pages).
Choose by the reader's requested result, not by whether a root or snapshot exists.
Use authored Markdown and the packaged Codeclew-shell renderer for an ordinary
checked local HTML preview; explicit MDX/Docusaurus output keeps its selected route. Interaction
is optional. Reuse a frozen root through supported reads. For a new repository,
[register and capture it](references/service-documentation.md#start-or-recover),
then return to the same human-page route.

| Requested result | Route |
| --- | --- |
| Service/process documentation, including a broad generate/update request | [Human pages](references/compose-original.md#frozen-source-to-reviewed-pages) |
| Bounded SOURCE question or saved-answer maintenance | [Prepared SOURCE](README.md) |
| Explicit native Work, proposal or coordinator deliverable | [Native authoring](references/authoring-example.md#choose-the-existing-authoring-route) |
| Explicit historical native `docs render` or frozen native publication replay | [Service documentation](references/service-documentation.md) |

Before reporting documentation done, apply the
[requested-scope completion check](references/compose-original.md#requested-scope-completion)
and [task-oriented reader checklist](references/task-oriented-readers.md).
A raw catalogue, generated structure, GAP cards or a successful render is a draft,
not a completed explanation. Preserve explicit native and historical routes.

## Prepared SOURCE shortcut

Use this shortcut only when the requested result is bounded SOURCE Q&A or saved
answer maintenance and the caller supplies a documentation root, service, exact
selector and saved snapshot or answer/evidence. Read only [README.md](README.md). It is the complete
short route for SOURCE Q&A and answer maintenance, including the bounded packet
helper. With a previous full packet and exact target, compare directly; no
preliminary packet metadata, Git or hash read is required. Do not load the references below for that route. If relevant inputs are
already verified unchanged, reuse the saved answer/evidence without context fetch
or full JSON output.

A request to create durable documentation or a complete thread uses the authoring
route below, even with the same prepared inputs. Choose by the requested result,
not the shape of its root/service/snapshot arguments.

## Human documentation by default

Read the [human-page authoring route](references/compose-original.md#frozen-source-to-reviewed-pages)
for durable reader documentation, whether evidence already exists or acquisition
is still needed. Keep the full requested scope. Native Work/proposal/coordinator
output is selected explicitly; it is not the fallback for a new repository.

## Other workflows

Read only the reference for the requested result:

| Result | Reference |
| --- | --- |
| Navigate, analyze or change code through an admitted session | [Managed code](references/managed-code.md) |
| Register/capture a service before human-page authoring | [Source acquisition](references/service-documentation.md#start-or-recover) |
| Inventory entrypoints and prepare complete local threads | [Thread preparation](references/thread-preparation.md) |
| Finish a large SOURCE read or batch independent source selections | [Recorded SOURCE parts and batches](references/source-read-parts.md) |
| Connect a separately captured Kotlin library to a service thread | [Independent library source](references/library-sources.md) |
| Collect or explain a saved failed Check | [Evidence and support](references/documentation-evidence.md) |

For explicitly requested native deliverables, use the
[native authoring route](references/authoring-example.md#choose-the-existing-authoring-route).
Its coordinator and Work-local proposal paths remain supported. A bounded SOURCE
answer stays on the short route; it does not require full-service authoring.

This is the installed-product workflow for a target repository. Resolve the
installed `clew` launcher once; never substitute a source launcher, target/capsule
binary or runtime seed. Do not edit `CODECLEW_HOME` objects. Documentation commands
perform their own admission; the managed-session loop is not an extra docs step.
For development of Codeclew itself, use its repository-native contributor workflow.

Preserve the user's scope, exact refs and existing work. Keep authority, coverage,
unresolved/omitted evidence and cursor limits visible; syntax or declared evidence
is not compiler verification or observed runtime behavior. A successful capture
and publication freshness are separate facts. Keep full evidence local, and share
only within the user's authorized scope.
