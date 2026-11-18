# Declared external relationships and boundaries

External-service registry records and engineer-declared interactions are authored declarations, not captured target implementations or runtime delivery proof. Read them together with source evidence. Explain the contract and receiving fragment, and distinguish sending a request from observing its remote effect.

Register the exact external ID with `clew docs service add` to promote it in
the same documentation root. The repository must match any repository already
recorded for that identity. Bind and capture it through the ordinary service
workflow. The service retains the earlier declaration as `externalOrigin`.
Use `clew docs external show --id ID` to inspect the retained declaration and
resolved service. Existing declared calls keep their original evidence and
historical snapshots. Other IDs and roots use their own declarations.

Use the single [authored-originals publication route](../../skills/codeclew/references/compose-original.md#frozen-source-to-reviewed-pages):
read saved evidence, author the explanation, compose and bind it, then finalize
checked HTML. Keep originals, source manifests and historical exports. No new
source capture is required when the saved evidence is sufficient.

Existing native HTML/Markdown/MDX exports remain historical files. Removed
native publishing commands cannot regenerate them. For source acquisition and
bounded reads, see [source documentation](source-documentation.md).
