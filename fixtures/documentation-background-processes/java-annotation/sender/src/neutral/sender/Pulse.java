package neutral.sender;

import java.util.List;
import io.eventuate.tram.events.publisher.DomainEventPublisher;
import neutral.contract.Signal;
import org.springframework.scheduling.annotation.Scheduled;
import org.springframework.stereotype.Component;

@Component
public class Pulse {
    private final DomainEventPublisher stream;

    public Pulse(DomainEventPublisher stream) {
        this.stream = stream;
    }

    @Scheduled(fixedDelay = 1000)
    public void tick() {
        var update = new Signal("steady");
        stream.publish("neutral.signal", "sensor", List.of(update));
    }
}
