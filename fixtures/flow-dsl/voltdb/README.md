# VoltDB inline builder regression source

`OrderProc.java` is an unmodified upstream example from
[VoltDB/voltdb-examples, revision c043973e766d56e918f90c4c72773a81cec52f57](https://github.com/VoltDB/voltdb-examples/blob/c043973e766d56e918f90c4c72773a81cec52f57/compound-proc/src/OrderProc.java).
Its original copyright and permission notice are retained in the file.

The fixture verifies extraction of the declared `newStageList(...).then(...).build()`
chain from exact retained source. It does not execute VoltDB or establish scheduler,
transaction, delivery, or callback completion behavior. Upstream describes this
as an intentionally simplified demonstration, not a production business system.
