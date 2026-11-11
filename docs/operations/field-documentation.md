# First useful documentation and selective updates

Start from a practical reader question and an actual entry point. Reuse a saved
source snapshot when it covers that question. Register and capture additional
source only when evidence is missing; see [source acquisition](source-documentation.md).

Prepare bounded Work and read its context, complete source continuations,
contracts and limitations. Follow returned helper candidates before recording
an evidence gap. Work is the saved evidence scope; it is not a finished document.

Use the [authored-originals pipeline](../../skills/codeclew/references/compose-original.md#frozen-source-to-reviewed-pages)
for every new document and update. Explain inputs, decisions, interactions,
side effects and outcomes in order. Put failure behavior beside each relevant
step, and link cross-service steps to their contracts and service fragments.
Technical details belong in disclosures; preserve source and provenance links.

For a bounded question, use the [prepared SOURCE route](../../skills/codeclew/README.md).
`docs work run --draft` and `docs work review-draft` retain answer, packet and
audit JSON. They do not publish documents. Model calls require an explicitly
configured finite budget; failed attempts and accounting remain durable.

For updates, compare saved evidence with the exact authored baseline, keep user
originals and provenance, and use guarded changes to the originals. Export into
a new directory and finalize it. A new snapshot, a source inventory or an
unresolved evidence gap does not by itself complete the requested explanation.
Keep historical HTML and saved answers intact.

Use `docs changes` to inspect differences supported by saved evidence. Use
`docs check` only for explicit current-source acquisition. Missing repositories,
providers or immutable objects must remain explicit; never substitute a different
source profile or rewrite retained state to make a check pass.

See [snapshot and Work contracts](docs-snapshot-store.md),
[diagnostics](support-diagnostics.md) and
[the short source-bound authoring example](../../skills/codeclew/references/authoring-example.md).
