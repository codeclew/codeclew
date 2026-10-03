package example.linked;

import java.util.concurrent.BlockingQueue;

public final class ParentAEndpoint {
    private final BlockingQueue<Task> submitted;

    public ParentAEndpoint(BlockingQueue<Task> submitted) {
        this.submitted = submitted;
    }

    public boolean submit(Task task) {
        if (task == null) {
            return false;
        }
        return submitted.offer(task);
    }
}
