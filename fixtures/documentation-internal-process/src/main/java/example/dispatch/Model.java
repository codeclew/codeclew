package example.dispatch;

import java.time.Instant;
import java.util.List;

final class Task {
    enum State { READY, WAITING_RESPONSE, DONE }
    final String id;
    final String type;
    final State state;
    final Instant deadline;
    Task(String id, String type, State state, Instant deadline) {
        this.id = id; this.type = type; this.state = state; this.deadline = deadline;
    }
}
interface TaskRepository {
    List<Task> findPending();
    void changeState(String id, Task.State state);
    void deferUntil(String id, Instant until);
    void recordError(String id, String reason);
}
final class PendingFinder {
    private final TaskRepository repository;
    PendingFinder(TaskRepository repository) { this.repository = repository; }
    List<Task> pending() { return repository.findPending(); }
}
interface DeliveryClient { void send(String endpoint, String id); }
final class MissingRouteException extends RuntimeException {
    MissingRouteException(String type) { super(type); }
}
