# Author readable process descriptions

Use the [authored-originals route](../../skills/codeclew/references/compose-original.md#frozen-source-to-reviewed-pages).
Start at the actual entry and explain the input, supported decisions, actions and
outcome in order. Put failures beside their step: what already changed, what can
be retried and what remains unknown. Cross-service steps link to contracts and
the corresponding service fragments. Keep technical support in disclosures.

Preserve early returns, exclusive alternatives, loops, cleanup and exception
paths supported by the selected source. Distinguish scheduling from execution,
and sending a request from observing its remote effect. A list of source
constructs is not a behavioral explanation or proof of execution order.

Use meaningful short labels and add explanation only when it gives context.
Cite exact retained sources for claims and diagram elements. Do not infer business
meaning from method names or repeat source snippets as prose. Follow returned
helper candidates before declaring evidence unavailable; record precise limits
without inventing an outcome.

Check the normal result, each early exit and each supported failure against the
complete reader question and source scope before finalizing. Structural binding
and successful rendering do not prove arbitrary prose true. Keep user originals,
provenance and historical exports unchanged when producing a new version.
