package example.linked;

import java.util.concurrent.BlockingQueue;

public final class ChildEndpoint {
    private final BlockingQueue<Task> submitted;

    public ChildEndpoint(BlockingQueue<Task> submitted) {
        this.submitted = submitted;
    }

    public boolean submit(Task task) {
        if (task == null) {
            return false;
        }
        return submitted.offer(task);
    }
}
