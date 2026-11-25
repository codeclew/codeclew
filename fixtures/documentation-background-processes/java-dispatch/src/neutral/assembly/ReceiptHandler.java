package neutral.assembly;

import io.eventuate.tram.events.subscriber.DomainEventEnvelope;
import io.eventuate.tram.events.subscriber.DomainEventHandlers;
import io.eventuate.tram.events.subscriber.DomainEventHandlersBuilder;

public final class ReceiptHandler {
    public DomainEventHandlers handlers() {
        var table = DomainEventHandlersBuilder.forAggregateType("receipts.pending");
        return table.onEvent(BatchArrived.class, this::accept).build();
    }

    public void accept(DomainEventEnvelope<BatchArrived> envelope) {
        if (envelope.getEvent() == null) {
            throw new IllegalArgumentException("missing batch");
        }
    }
}
