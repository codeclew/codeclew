package example.linked;

import java.util.concurrent.BlockingQueue;

public final class ParentAWorker {
    private final BlockingQueue<Task> pending;
    private final ChildEndpoint child;
    public boolean enabled = true;
    public String lastState = "idle";

    public ParentAWorker(BlockingQueue<Task> pending, ChildEndpoint child) {
        this.pending = pending;
        this.child = child;
    }

    public void runOnce() {
        Task task = pending.poll();
        if (task == null) {
            lastState = "empty";
            return;
        }
        if (!enabled) {
            lastState = "disabled";
            return;
        }
        if (!task.eligible) {
            lastState = "ineligible";
            return;
        }
        if (task.priority < 1) {
            lastState = "below-parent-priority";
            return;
        }
        boolean accepted = child.submit(task);
        if (!accepted) {
            lastState = "child-refused";
            return;
        }
        lastState = "submitted-to-child";
    }
}
