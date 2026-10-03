package example.linked;

import java.util.concurrent.BlockingQueue;

public final class ParentBEndpoint {
    private final BlockingQueue<Task> submitted;

    public ParentBEndpoint(BlockingQueue<Task> submitted) {
        this.submitted = submitted;
    }

    public boolean enqueue(Task task) {
        if (task == null) {
            return false;
        }
        return submitted.offer(task);
    }
}
