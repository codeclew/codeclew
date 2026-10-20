# Export source links instead of source attachments

Use `--source-mode remote` when readers should open the source repository instead
of receiving retained source fragments with the documentation. `embedded` remains
the default. The option is available on `docs render`, `docs pages render` and
`docs dsl render`.

Render a saved publication into a new directory:

```sh
clew docs pages render --root "$DOCS_ROOT" \
  --snapshot "$SNAPSHOT" --publication "$PUBLICATION" \
  --source-mode remote --output documentation-site
```

Add `--format docusaurus` for the portable site. Optional reading plans, HTTP
comparisons and data-model plans work in either format. Declaration selections
and flow DSL exports accept the same source options.

For the current local publication, this shorter form uses the latest saved
check, or the explicitly selected `--snapshot`:

```sh
clew docs render --root "$DOCS_ROOT" --source-mode remote \
  --snapshot "$SNAPSHOT" --output documentation-site
```

This remote form exports an existing publication. First run the ordinary local
render to accept new inputs, refresh source evidence or publish a documentation
revision. Remote export refuses `--input`, `--refresh`, `--publish` and
`--require-complete`; it creates no new working publication. It requires
`--output`. Existing nonempty output directories are refused.

## Repository bindings

GitHub.com and GitLab.com can be inferred from captured repository metadata.
For self-managed hosts or a different web endpoint, pass `--source-map repos.json`:

```json
{
  "schema": "codeclew-source-repositories/1.0",
  "repositories": {
    "orders-repository": {
      "provider": "gitlab",
      "webBase": "https://git.example.invalid:8443/gitlab/team/orders",
      "origin": "ssh://git@git.example.invalid:2222/team/orders.git"
    }
  }
}
```

Keys are immutable repository IDs from the service registration. Providers are
`github` or `gitlab`. `webBase` is an HTTPS repository URL, including any deployment
prefix, port and nested groups. `origin` is optional when that repository is in
the selected snapshot. Supply its exact captured clone origin for historical
sources from a repository absent from the selected snapshot. SSH transport ports
and web ports are independent. Unknown hosts are never guessed to be GitLab.
No credentials, query strings or fragments are accepted in web bases.

Each link uses its own retained repository, full immutable commit SHA, file and
line range. GitHub links use `/blob/SHA/file#Lstart-Lend`; GitLab uses
`/-/blob/SHA/file#Lstart-end`. Spaces, Unicode and URL punctuation in file paths
are encoded. Historical sources retain their historical repository and revision,
even when the selected service has moved to a different repository. No link falls
back to a branch, `HEAD`, current checkout or a similarly named file.

Missing exact bindings, dirty or generated source evidence, ambiguous mappings
and unsupported URLs stop the export with an error. Use embedded mode for that
evidence, or restore its exact repository metadata. Codeclew does not contact the
provider, authenticate, check access or verify that a committed SHA was pushed.
Readers need permission to open the resulting repository links.

## What is included

Remote exports omit automatically attached source bodies and syntax snippets
from native HTML, JSON, MDX, source appendices, comparison source and test-file attachments,
original/download files and portable shared objects. Automatically generated
source-outline diagrams are omitted too. Source actions open the exact external
range; multiple sources are presented as a list of repository links.

Authored explanations, intentional quotations, contract examples, semantic
identities, coordinates, digests, limitations and provenance remain. This option
does not remove source-like text that an author deliberately put in prose or an
example. Review that content before sharing.

The manifest records `sourceExport`, its policy digest and `exportIdentity`.
Changing mode or repository mappings changes export identity. Source text digests
continue to identify the original evidence; they do not hash the empty exported
text field. A remote export is a reading artifact, not an evidence reimport format.
The local retained evidence, original author inputs and previous immutable
exports remain unchanged. Share the new output directory, not the local root.
