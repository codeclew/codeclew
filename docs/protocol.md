# Worker protocol

Protocol `1.0` is defined in `schemas/worker.proto`. Frames are a four-byte unsigned big-endian length followed by one Protobuf message. The maximum accepted frame is 64 MiB.

The worker emits an unsolicited capabilities response with `request_id=0` immediately after startup. It declares language, worker/compiler versions, supported protocol versions, operations, features and unsupported features. The core rejects a version mismatch before sending work.

Requests and responses correlate by monotonic `request_id`; every request carries a protocol version, schema version and snapshot. Request DTOs use typed Protobuf fields. Complex immutable IR responses additionally expose canonical JSON in their typed response message so they can be inspected without linking Kotlin/FIR types into Rust.

Sources up to 64 KiB may be inline. Larger sources are written once to the repository-local content-addressed blob store and transported as `BlobRef(content_hash, relative_path, size_bytes)`; the worker checks path confinement and SHA-256 before reading. `BatchRequest` carries repeated complete `WorkerRequest` messages, and the vertical test exercises an actual two-item batch round-trip.

Unknown request kinds and unsupported schema versions are errors. Worker crashes cannot mutate the index or ledger because all durable state commits occur in Rust transactions after a complete response; cache/blob writes are content-addressed and atomically replaced.
