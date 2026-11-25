package neutral.receiver;

import io.eventuate.tram.events.subscriber.DomainEventEnvelope;
import io.eventuate.tram.events.subscriber.annotations.EventuateDomainEventHandler;
import neutral.contract.Signal;

public class Unbound {
    @EventuateDomainEventHandler(subscriberId = "alternate-reader", channel = "neutral.signal")
    public void accept(DomainEventEnvelope<Signal> message) {}
}
