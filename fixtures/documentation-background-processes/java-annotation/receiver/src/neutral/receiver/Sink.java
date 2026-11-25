package neutral.receiver;

import io.eventuate.tram.events.subscriber.DomainEventEnvelope;
import io.eventuate.tram.events.subscriber.annotations.EventuateDomainEventHandler;
import neutral.contract.Signal;

public class Sink {
    private String current;

    @EventuateDomainEventHandler(subscriberId = "signal-reader", channel = "neutral.signal")
    public void accept(DomainEventEnvelope<Signal> message) {
        current = message.getEvent().value();
    }

    @EventuateDomainEventHandler(subscriberId = "signal-reader", channel = "neutral.backup")
    public void acceptBackup(DomainEventEnvelope<Signal> message) {
        current = message.getEvent().value();
    }

    @EventuateDomainEventHandler(subscriberId = "signal-reader", channel = "neutral.signal")
    public void acceptOtherType(DomainEventEnvelope<neutral.foreign.Signal> message) {
        current = message.getEvent().value();
    }

    @neutral.foreign.EventuateDomainEventHandler(subscriberId = "signal-reader", channel = "neutral.signal")
    public void acceptForeignAnnotation(DomainEventEnvelope<Signal> message) {
        current = message.getEvent().value();
    }
}
