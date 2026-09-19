package example.dispatch;

import java.time.Instant;

public final class TaskStateTransitions {
    private final TaskRepository repository;
    public TaskStateTransitions(TaskRepository repository) { this.repository = repository; }

    public boolean promote(Task task, Instant now) {
        if (task.state == Task.State.READY) {
            repository.changeState(task.id, Task.State.WAITING_RESPONSE);
            return true;
        }
        if (task.state == Task.State.WAITING_RESPONSE && task.deadline.isBefore(now)) {
            repository.recordError(task.id, "Response deadline expired");
        }
        return false;
    }
}
