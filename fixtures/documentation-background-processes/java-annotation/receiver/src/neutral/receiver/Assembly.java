package neutral.receiver;

import io.eventuate.tram.spring.events.subscriber.TramEventSubscriberConfiguration;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;
import org.springframework.context.annotation.Import;

@Configuration
@Import(TramEventSubscriberConfiguration.class)
public class Assembly {
    @Bean
    public Sink sink() {
        final var handler = new Sink();
        return handler;
    }
}
