# Revision coordination fixtures

The `docsys_t13_*` integration cases in `crates/clew/tests/documentation_system.rs`
create isolated two-service producers and a central repository with no usable Git
command. They exercise exact target acceptance, duplicate/delayed events, batch
reconciliation, late artifact rejection, independent local gaps, moved tags,
immutable source pages and canonical explanation hashes, private-cache loss,
expired retained evidence, interrupted status publication, concurrent note/view
edits, and bounded configured queue execution. Fixture events are generated from
actual temporary source commits. No credentials or live CI platform are involved.
