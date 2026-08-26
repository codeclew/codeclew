package example.outbox

import example.shared.OutboxEvent

interface EventPublisher {
    fun publish(event: OutboxEvent)
}

interface OutboxStore {
    fun markPublished(event: OutboxEvent)
}

class OutboxDispatcher(
    private val publisher: EventPublisher,
    private val store: OutboxStore,
) {
    fun dispatch(event: OutboxEvent) {
        publisher.publish(event)
        store.markPublished(event)
    }
}
