# Repository privacy and Git authorship

Use your configured Git author and committer identity. Contributor and bot
identities are allowed; neither the hook nor GitHub CI requires a synthetic
maintainer identity. Do not replace another contributor's attribution.

The pre-commit hook checks staged content. CI checks both reachable historical
file content (`--history`) and pending worktree content (`--worktree`). These
checks still reject secrets, personal filesystem paths, private generated
evidence, filled pilot cases and non-placeholder email addresses in file content.
An email in Git author metadata is distinct from an email embedded in a file.
The legacy `--pre-commit` flag remains accepted without an identity restriction.
Normal build, test and privacy CI remain enabled.

Changing published attribution is a separate history migration. Back up remote
heads/tags, local refs and dirty worktree state first. Record the old-to-new map,
verify unchanged trees, messages, dates and parent topology, and preserve real
contributor and bot identities. Coordinate moving active worktrees and remote
refs; do not reset dirty checkouts to make the migration convenient.

Rewritten commits and annotated tags receive new object IDs. Existing signatures
cannot authenticate modified objects. Release manifests, retained evidence,
candidate hashes and benchmark receipts continue to identify their original
objects and must not be silently rewritten or relabelled as newly qualified.
