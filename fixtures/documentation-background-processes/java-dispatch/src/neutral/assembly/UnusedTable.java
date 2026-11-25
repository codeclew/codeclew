package neutral.assembly;

import io.eventuate.tram.events.subscriber.DomainEventEnvelope;
import io.eventuate.tram.events.subscriber.DomainEventHandlers;
import io.eventuate.tram.events.subscriber.DomainEventHandlersBuilder;

public final class UnusedTable {
    public DomainEventHandlers handlers() {
        return DomainEventHandlersBuilder.forAggregateType("receipts.other")
            .onEvent(BatchArrived.class, this::observe)
            .build();
    }

    public void observe(DomainEventEnvelope<BatchArrived> envelope) {}
}
