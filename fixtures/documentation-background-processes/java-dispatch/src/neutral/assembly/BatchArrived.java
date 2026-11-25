package neutral.assembly;

import io.eventuate.tram.events.common.DomainEvent;

public record BatchArrived(String key) implements DomainEvent {}
