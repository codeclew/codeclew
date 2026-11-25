package neutral.contract;

import io.eventuate.tram.events.common.DomainEvent;

public record Signal(String value) implements DomainEvent {}
