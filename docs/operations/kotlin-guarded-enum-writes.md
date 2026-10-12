# Guarded Kotlin enum write attempts

The Kotlin 2.4.10 documentation adapter introduced enum bindings in version 1.1
of `codeclew-kotlin-documentation-data`; version 1.2 retains that contract. Each enum expression binds an exact
original UTF-8 range to a K2-resolved enum entry and its declaring enum class.
Imported aliases retain the compiler identity; matching source spelling alone
does not qualify an entry. Version 1.0 inputs remain readable without enum rows.

The common source-data projection uses `codeclew-source-data-state/1.1` for
enum values. Outputs without newer values retain version 1.0 and its canonical
bytes; accessor attempts use version 1.2.
`ENUM_CONSTANT` values carry `identity`, `enumClass`, `entry` and `authority`.
They describe symbolic entries, with an `ENUM_INITIALIZATION_UNPROVEN` frontier:
neither enum initialization nor successful execution of an access is proved.

Qualified ordinary property assignments retain their exact receiver and source
guards. Explicit integer order comparisons are admitted only for resolved
non-null `kotlin.Int.compareTo`; overloaded operators remain opaque. Branch
conditions are source alternatives, not feasible runtime paths. A property on
an arbitrary receiver remains a write attempt with `RECEIVER_STORAGE_OPAQUE`;
the declaration identity never establishes object identity. Custom, delegated,
virtual and otherwise unqualified accessors do not become storage writes.

The selected-source view distinguishes assignment, subsequent helper calls and
storage calls. None establishes database commit, rollback or delivery. Calls and
unsupported expressions retain their existing frontiers and normal-completion
conditions. The nullness-only public session values profile is unchanged.

Qualification includes native K2 enum aliases, receiver writes, built-in integer
order and negative custom-operator cases. Rust conformance tests cover guarded
storage joins and rejection of missing, duplicate, foreign or unqualified enum
bindings. These fixtures do not qualify a different project/compiler profile;
real project capture must record its analyzer and compatibility decision.
