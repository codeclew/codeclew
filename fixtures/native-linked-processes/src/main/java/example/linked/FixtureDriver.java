package example.linked;

/** Direct, deterministic method calls: no threads, scheduler, HTTP or network. */
public final class FixtureDriver {
    private FixtureDriver() {}

    public static void main(String[] arguments) {
        RecordingGateway gateway = new RecordingGateway(0, false);
        Composition.Pipeline pipeline = Composition.assemble(gateway);
        pipeline.parentAEndpoint.submit(new Task("a-1", " café ", true, 1));
        pipeline.parentBEndpoint.enqueue(new Task("b-1", " beta ", true, 3));
        pipeline.parentAWorker.runOnce();
        pipeline.parentBWorker.runOnce();
        pipeline.childWorker.runOnce();
        pipeline.childWorker.runOnce();
        System.out.println("parent-a=" + pipeline.parentAWorker.lastState);
        System.out.println("parent-b=" + pipeline.parentBWorker.lastState);
        System.out.println("child=" + pipeline.childWorker.lastState);
        System.out.println("local-attempts=" + pipeline.childWorker.attempts);
        System.out.println("local-requests=" + gateway.requests());
    }
}
