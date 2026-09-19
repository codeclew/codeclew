package example.dispatch;

import java.time.Duration;
import java.time.Instant;

public final class DispatchWorker {
    private static final Duration MISSING_ROUTE_DELAY = Duration.ofMinutes(20);
    private final PendingFinder finder;
    private final TaskStateTransitions transitions;
    private final RouterGateway gateway;
    private final TaskRepository repository;

    public DispatchWorker(PendingFinder finder, TaskStateTransitions transitions,
                          RouterGateway gateway, TaskRepository repository) {
        this.finder = finder;
        this.transitions = transitions;
        this.gateway = gateway;
        this.repository = repository;
    }

    public void processPending(Instant now) {
        for (Task task : finder.pending()) {
            if (transitions.promote(task, now)) {
                dispatchEligible(task, now);
            }
        }
    }

    private void dispatchEligible(Task task, Instant now) {
        try {
            gateway.dispatch(task);
        } catch (MissingRouteException missingRoute) {
            repository.deferUntil(task.id, now.plus(MISSING_ROUTE_DELAY));
        } catch (RuntimeException failedDispatch) {
            repository.recordError(task.id, "Dispatch failed");
        }
    }
}
