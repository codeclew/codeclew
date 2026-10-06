# Select endpoints for publication

This command family is introduced after v0.13.17; installed v0.13.17 does not
include it.

Endpoint publication is a persistent reader selection. Use it to omit an
infrastructure endpoint, such as a Swagger handler, while preserving saved
analysis and accepted explanations. Codeclew does not automatically blacklist
Swagger, OpenAPI, or any other route.

First list callable groups from an exact saved Check:

```sh
clew docs endpoint list --root ./architecture --service orders --snapshot SAVED_SNAPSHOT
```

Each item returns an exact `selector` (`service`, compilation `scope`, and
callable `symbol`), `entrypointIds`, and `declarationIds`. The choice applies to
**all registrations and native page selections of that callable**. It does not
select an individual HTTP route when one method owns several routes. Identical
symbols in another service or compilation scope remain separate. Source-syntax
declarations use an empty scope; moves or renames create new identities.

Exclude an endpoint using an ID returned by the list:

```sh
clew docs endpoint exclude --root ./architecture --service orders --snapshot SAVED_SNAPSHOT --endpoint RETURNED_ENDPOINT_ID --expected-policy-digest RETURNED_POLICY_DIGEST
clew docs render --root ./architecture --snapshot SAVED_SNAPSHOT --publish
```

Copy the current `policyDigest` into `--expected-policy-digest` to reject an
intervening selection change. Omit that option to edit the current policy under
the repository lock. Repeating the same exclusion or inclusion is idempotent.
Every exclusion must resolve against the explicit saved Check. Unsupported or
ambiguous publication identities cannot be excluded by guessing an ID.

The ordinary renderer removes the endpoint's operation, gap, catalogue row,
contract row, reader navigation, and operation diagrams from the new publication.
It retains the complete narrative, accepted versions, source records, and
catalogue delivery in the bindings. Other operations remain available. Render
with a saved Check performs no source capture or model call. Registered inputs
must still match the saved Check; publication selection does not change their
digest or require a new analysis.

Restore the callable with a returned endpoint or declaration ID:

```sh
clew docs endpoint include --root ./architecture --service orders --snapshot SAVED_SNAPSHOT --endpoint RETURNED_ENDPOINT_ID
clew docs render --root ./architecture --snapshot SAVED_SNAPSHOT --publish
```

Inclusion restores retained explanations and their source/contract versions.
Exclusions survive new source Checks and renders. A disappeared callable's
selector remains saved and is reported as unmatched; Codeclew does not silently
transfer the choice to a similar method. To remove a disappeared exclusion,
copy its exact selector without requiring that it still resolves:

```sh
clew docs endpoint include --root ./architecture --service orders --scope RETURNED_SCOPE --symbol RETURNED_SYMBOL
```

For source syntax, pass `--scope ''`. `docs refresh --status-only` preserves an
already applied selection and never captures source or asks a model. If the
policy changed since publication, first render the selected saved Check to
apply it; status observation alone cannot change publication scope.

## Native HTML and MDX pages

Native pages use the same policy. Use a returned callable declaration ID when
the method has no ordinary endpoint registration:

```sh
clew docs endpoint exclude --root ./architecture --service orders --snapshot SAVED_SNAPSHOT --declaration RETURNED_DECLARATION_ID
clew docs pages render --root ./architecture --snapshot SAVED_SNAPSHOT --input ./selections.json --output ./selected-pages
```

Keep the original selection file, including frozen authored paragraphs. The
renderer validates the requested declarations before filtering, then derives
linked navigation, reverse memberships, catalogue, and manifest from the
remaining selections. Excluding all selections produces an empty publication.
Use a new or empty output directory for every native render; populated outputs
are never overwritten. Re-inclusion with the same selection file restores the
selected pages without analysis or model calls. The manifest binds the policy
and effective selection when exclusions are present; global page content
digests may change even when an unaffected page's HTML bytes do not.

## Boundaries

The policy is stored at `publication/endpoint-selection.json`, outside registered
analysis and authoring inputs. Use the CLI to edit it. Rendering detects a
concurrent policy change before switching the ordinary reader pointer or writing
native outputs.

Publication selection is not redaction. Complete source facts, accepted prose,
portable bindings, and immutable historical publications remain available.
An excluded endpoint can still appear as a source-backed callee in another
operation. Service-wide authored prose is not rewritten. Exclusion does not
establish analysis completeness or runtime behavior, and does not relax
`--require-complete` evidence checks. `docs process suppress` is a separate
choice for internal process candidates and does not control endpoint publication.
