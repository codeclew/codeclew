package example.linked;

import java.util.concurrent.BlockingQueue;

public final class ChildWorker {
    private final BlockingQueue<Task> pending;
    private final Gateway gateway;
    private final String prefix;
    private final int minimumPriority;
    public String lastState = "idle";
    public String lastRequest;
    public int attempts = 0;

    public ChildWorker(BlockingQueue<Task> pending, Gateway gateway,
                       String prefix, int minimumPriority) {
        this.pending = pending;
        this.gateway = gateway;
        this.prefix = prefix;
        this.minimumPriority = minimumPriority;
    }

    public void runOnce() {
        Task task = pending.poll();
        if (task == null) {
            lastState = "empty";
            return;
        }
        if (!task.eligible) {
            lastState = "ineligible";
            return;
        }
        if (task.priority < this.minimumPriority) {
            lastState = "below-child-priority";
            return;
        }
        String request = prepare(task);
        lastRequest = request;
        attempts = attempts + 1;
        lastState = "attempting";
        int response = gateway.deliver(request);
        if (response != 0) {
            lastState = "rejected";
            return;
        }
        lastState = "acknowledged";
    }

    public String prepare(Task task) {
        String chosen = task.name;
        if (chosen == null) {
            chosen = "anonymous";
        }
        String transformed = chosen.trim();
        return prefix + transformed;
    }
}
