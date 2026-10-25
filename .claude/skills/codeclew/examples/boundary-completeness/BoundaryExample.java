import java.util.concurrent.CompletableFuture;

// Synthetic source truth for bounded author/reviewer exercises, not a production service.
final class BoundaryExample {
    record Reservation(String id, int quantity) {}
    record ReservationRequest(String id, int quantity, boolean save, boolean notifyQueue,
                              boolean requiredInference) {}
    interface VersionApi { String activeVersion(); }
    interface Store { void save(Reservation value); }
    interface Queue { void send(String id); }
    interface Inference { int calculate(String id); }
    final VersionApi versions;
    final Store store;
    final Queue queue;
    final Inference inference;
    BoundaryExample(VersionApi versions, Store store, Queue queue, Inference inference) {
        this.versions = versions; this.store = store; this.queue = queue; this.inference = inference;
    }
    String reserve(ReservationRequest request) {
        String version = versions.activeVersion(); // External value; null is possible.
        if (version == null) version = "local-default"; // Local fallback, not a remote response.
        Reservation value = new Reservation(request.id(), request.quantity());
        if (request.save()) store.save(value); // Attempt and local API return; no commit evidence.
        if (request.notifyQueue()) CompletableFuture.runAsync(() -> queue.send(value.id()));
        CompletableFuture<Integer> result = CompletableFuture.supplyAsync(() -> inference.calculate(value.id()));
        if (request.requiredInference()) result.join(); // Optional path does not await completion.
        return version; // Queue consumer completion and durability are not established here.
    }
}
