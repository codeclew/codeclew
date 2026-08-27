# Rust and Python Conditional Mutation Pilot

This qualification decides whether the bounded Rust and Python mutation
profiles are operationally `PILOT_READY`. It does not make their syntax evidence
compiler-backed and never upgrades `UNSURE` to `VERIFIED`.

Run the repository-owned gate on a clean candidate commit:

```bash
python3 -I -S scripts/language_mutation_pilot.py
```

The gate runs three independent changes per language, each in a fresh Git
repository. Every case must prove all of the following:

- the native baseline passes;
- context contains immutable source authority for the edit;
- repeated prepare returns the same run;
- the source ref and worktree do not change before explicit publication;
- strict publication is refused;
- every qualified obligation is explicitly acknowledged;
- repeated conditional publication returns the same final commit;
- the exact expected write set and one-commit boundary hold;
- the native post-publication test passes;
- session close and managed GC succeed without manual state cleanup.

The only passing result is 6/6 with 3/3 Rust and 3/3 Python. Release
qualification additionally requires `runtimeMode: RELEASE`. A failure keeps the
affected profile at `CONDITIONAL_MUTATION`; it must not be relabelled or worked
around with direct state deletion.

`PILOT_READY` therefore means the conditional mutation transaction is ready for
limited team use. Rust name resolution, cfg and macro expansion, and Python
runtime imports, types, decorators, metaclasses and dynamic execution remain
publication obligations verified by project-native tests and human review.
