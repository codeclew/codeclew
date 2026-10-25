# Semantic reader example

`Example.java` is newly constructed public source. Its adapter returns a status
field; it does not start an HTTP server or establish a serialization contract.
Compile with JDK 21 or later and run `example.Example` to check contrasting inputs.

Use the same selected source and question for a normal and debug skill pass:

> Explain the vehicle attributes, the request and result representations, the
> minimum-coverage decision, and context failure behavior for `endpoint`/`quote`.

A correct reader can answer these questions from the resulting documentation:

- Manufacture year, manufacturer and mileage each have one object-scoped
  canonical attribute with explicitly mapped `VehicleWire` representations.
  Repeated reads do not create more attributes. Age is derived, not a year alias.
- `HistoryRecord` is a different object. Its copied manufacture year and mileage
  remain history-scoped attributes. Copying values does not merge object identity;
  `vehicleReference` is a reference in history, not a new field of the vehicle.
- `LegacyInput.year` has no established meaning here. Arithmetic and matching
  spelling do not justify an accepted manufacture-year alias or a proved
  different business meaning. Name the missing definition or mapping.
- The returned receipt is ignored. Sending is attempted; durable storage and
  receiver behavior are unknown. Mapping failure escapes before the send catch.
- Enabled plus five months rejects; six months and null do not satisfy this guard;
  disabled bypasses this check. Current deployment flag state is not supplied.
- The bounded adapter/handler lookup establishes warranty-coverage meaning,
  status 409 and the named error response. An initial fragment containing only
  `quote` is insufficient to call these facts unknown or invent a different label.
- Java record fields describe source representations. They do not automatically
  establish JSON names, required wire properties or a complete request schema.

Normal mode produces the bounded source-grounded answer. Debug mode preserves
that answer and adds selected identities, targeted missing-source lookups and
completed checks. Debug must not dump a whole catalogue or acquire unrelated
context. A one-shot model packet does not authorize extra reads: its coordinator
must supply the targeted handler/mapper/type supplement before author dispatch.

Keep compiler-backed and SOURCE_SYNTAX admission results separate. A successful
`javac` run for this example is not an admitted Codeclew compiler snapshot. Reuse
unchanged admitted producer gates only when the current workflow permits it.
The executable assertions and geometry checks are capability facts, not semantic
approval of generated prose. Independent review must record the actual reader
answer and fail each incorrect or ambiguous answer above.
