package neutral.assembly;

import io.eventuate.tram.events.subscriber.DomainEventDispatcher;
import io.eventuate.tram.events.subscriber.DomainEventDispatcherFactory;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;

@Configuration
public class DispatchWiring {
    @Bean
    public ReceiptHandler handler() {
        return new ReceiptHandler();
    }

    @Bean
    public DomainEventDispatcher dispatch(DomainEventDispatcherFactory factory, ReceiptHandler handler) {
        return factory.make("receipts-reader", handler.handlers());
    }
}
