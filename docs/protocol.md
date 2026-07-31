# Worker protocol

Protocol `1.0` is defined in `schemas/worker.proto`. Frames are a four-byte unsigned big-endian length followed by one Protobuf message. The maximum accepted frame is 64 MiB.

The worker emits an unsolicited capabilities response with `request_id=0` immediately after startup. It declares language, worker/compiler versions, supported protocol versions, operations, features and unsupported features. The core rejects a version mismatch before sending work.

Requests and responses correlate by monotonic `request_id`. Typed errors use the response error field. Source payloads are currently in a JSON byte field; schema companion messages reserve canonical IR evolution. Production-scale source transfer should replace inline source with the documented content-addressed blob reference without changing envelope framing.

Unknown request kinds are errors. Worker crashes cannot mutate the index or ledger because all state commits occur in Rust transactions after a complete response.

