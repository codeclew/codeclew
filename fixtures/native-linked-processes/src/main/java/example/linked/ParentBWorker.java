package example.linked;

import java.util.concurrent.BlockingQueue;

public final class ParentBWorker {
    private final BlockingQueue<Task> pending;
    private final ChildEndpoint child;
    private final RetargetEndpoint alternative;
    public boolean useAlternative = false;
    public String lastState = "idle";

    public ParentBWorker(BlockingQueue<Task> pending, ChildEndpoint child,
                         RetargetEndpoint alternative) {
        this.pending = pending;
        this.child = child;
        this.alternative = alternative;
    }

    public void runOnce() {
        Task task = pending.poll();
        if (task == null) {
            lastState = "empty";
            return;
        }
        if (task.priority < 3) {
            lastState = "below-parent-priority";
            return;
        }
        if (task.name == null) {
            lastState = "missing-name";
            return;
        }
        if (useAlternative) {
            boolean held = alternative.submit(task);
            lastState = held ? "held-in-alternative" : "alternative-refused";
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
