package beta;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;
import external.Gateway;

class Body {
    String type;
    String name;
    boolean eligible;
}
class Envelope {
    final String taskType;
    final String type;
    final String name;
    final boolean eligible;
    Envelope(String taskType, String type, String name, boolean eligible) {
        this.taskType = taskType; this.type = type; this.name = name; this.eligible = eligible;
    }
}
class Settings {
    final boolean enabled;
    final String prefix;
    Settings(boolean enabled, String prefix) { this.enabled = enabled; this.prefix = prefix; }
}
class IntakeEndpoint {
    private final BlockingQueue<Envelope> accepted;
    IntakeEndpoint(BlockingQueue<Envelope> accepted) { this.accepted = accepted; }
    boolean enqueue(String taskType, Body body) {
        Envelope task = new Envelope(taskType, body.type, body.name, body.eligible);
        return accepted.offer(task);
    }
}
class DeliveryLoop {
    private final BlockingQueue<Envelope> waiting;
    private final Gateway gateway;
    private final Settings config;
    String phase = "idle";
    DeliveryLoop(BlockingQueue<Envelope> waiting, Gateway gateway, Settings config) {
        this.waiting = waiting;
        this.gateway = gateway;
        this.config = config;
    }
    void consume() {
        Envelope task = waiting.poll();
        if (task == null) { return; }
        Settings current = config;
        if (current == null) { phase = "missing-settings"; return; }
        if (!current.enabled) { return; }
        if (!task.eligible) { return; }
        String chosen = task.name == null ? "fallback @EXT@ .mdx {probe()} <script>" : task.name;
        String transformed = current.prefix + chosen.trim();
        int status = gateway.deliver(transformed);
        if (status != 0) { phase = "rejected"; return; }
        phase = "sent";
    }
}
class Assembly {
    static void wire(Gateway gateway, Settings config) {
        BlockingQueue<Envelope> shared = new LinkedBlockingQueue<Envelope>();
        IntakeEndpoint endpoint = new IntakeEndpoint(shared);
        IntakeEndpoint secondEndpoint = new IntakeEndpoint(shared);
        DeliveryLoop worker = new DeliveryLoop(shared, gateway, config);
    }
}
