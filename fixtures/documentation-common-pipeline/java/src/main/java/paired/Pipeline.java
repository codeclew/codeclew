package paired;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;

class Body {
    String type;
    String name;
    boolean eligible;
}
class Task {
    final String taskType;
    final String type;
    final String name;
    final boolean eligible;
    Task(String taskType, String type, String name, boolean eligible) {
        this.taskType = taskType; this.type = type; this.name = name; this.eligible = eligible;
    }
}
class Config {
    final boolean enabled;
    final String prefix;
    Config(boolean enabled, String prefix) { this.enabled = enabled; this.prefix = prefix; }
}
class DispatchEndpoint {
    private final BlockingQueue<Task> submitted;
    DispatchEndpoint(BlockingQueue<Task> submitted) { this.submitted = submitted; }
    boolean submit(String taskType, Body body) {
        Task task = new Task(taskType, body.type, body.name, body.eligible);
        return submitted.offer(task);
    }
}
class ProcessingLoop {
    private final BlockingQueue<Task> pending;
    private final Gateway gateway;
    private final Config config;
    String lastState = "idle";
    ProcessingLoop(BlockingQueue<Task> pending, Gateway gateway, Config config) {
        this.pending = pending;
        this.gateway = gateway;
        this.config = config;
    }
    void runOnce() {
        Task task = pending.poll();
        if (task == null) { return; }
        Config current = config;
        if (current == null) { return; }
        if (!current.enabled) { return; }
        if (!task.eligible) { return; }
        String chosen = chooseName(task);
        String transformed = current.prefix + chosen.trim();
        int status = gateway.deliver(transformed);
        if (status != 0) { lastState = "rejected"; return; }
        lastState = "sent";
    }
    private String chooseName(Task task) {
        String name = task.name;
        if (name == null) { return "anonymous π🙂 @EXT@ .mdx {probe()} <script>"; }
        return name;
    }
}
interface Gateway { int deliver(String value); }
class Composition {
    static void assemble(Gateway gateway, Config config) {
        BlockingQueue<Task> shared = new LinkedBlockingQueue<Task>();
        DispatchEndpoint endpoint = new DispatchEndpoint(shared);
        ProcessingLoop worker = new ProcessingLoop(shared, gateway, config);
    }
}
