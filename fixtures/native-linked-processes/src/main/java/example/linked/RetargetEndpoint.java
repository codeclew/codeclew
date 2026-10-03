package example.linked;

import java.util.concurrent.BlockingQueue;

/** A different declaration with the same method name; it does not call the child. */
public final class RetargetEndpoint {
    private final BlockingQueue<Task> held;

    public RetargetEndpoint(BlockingQueue<Task> held) {
        this.held = held;
    }

    public boolean submit(Task task) {
        return held.offer(task);
    }

    public int queued() {
        return held.size();
    }
}
