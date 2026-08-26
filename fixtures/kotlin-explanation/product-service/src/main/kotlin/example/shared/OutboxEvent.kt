package example.shared

data class OutboxEvent(val kind: String, val aggregateId: String)
