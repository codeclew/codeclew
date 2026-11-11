# Source-bound authoring example

## Choose the existing authoring route

To describe saved Work, read its context, source bodies, contracts and limitations.
Write authored originals, then use the [compose/bind/finalize commands](compose-original.md#original-pages-and-completion).
No new capture is needed when the retained evidence covers the question.

A useful process description starts at its actual entry. Explain the input,
conditions, meaningful actions and interactions in order. Place failure behavior
beside the relevant step: what was already changed, what can be retried and what
is unknown. Link a cross-service step to the matching contract and service thread.
Keep technical support in disclosures and retain source/provenance links.

Before finalization, compare the draft with the complete user question and the
read evidence. Trace an early exit, a normal result and each supported failure.
Distinguish scheduling a callback from running it, and sending a request from
observing its remote effect. Follow returned helper candidates before recording
a missing-evidence gap. A gap accounts for unfinished work; it does not complete
an explanation.

## Bounded Q&A

A configured finite Work Q&A run uses `docs work run --draft`; a separate
`docs work review-draft` reviews its saved answer JSON. Preserve failed attempts,
usage accounting and the selected runtime. These commands retain validated
answers and evidence; they do not produce standalone HTML, Markdown or diagrams.
Use the [prepared SOURCE shortcut](../README.md) when it already covers the question.

## Preserve existing material

Keep user originals, source manifests, previous indexes, saved answers and
historical exports. Apply a guarded update to authored originals and export into
a new directory. Existing legacy records remain inspectable; removed native
proposal/coordinator/render routes cannot resume or publish on this runtime.

For exact source views, use [the citation helpers](reader-metadata.md#read-an-exact-retained-source-view).
For service-wide acquisition and continuation, use [thread preparation](thread-preparation.md).
