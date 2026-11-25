package neutral.foreign;

import io.eventuate.tram.events.common.DomainEvent;

public record Signal(String value) implements DomainEvent {}
