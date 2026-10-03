package example.linked;

import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;

public final class Composition {
    private Composition() {}

    public static Pipeline assemble(Gateway gateway) {
        Pipeline pipeline = new Pipeline();
        wire(pipeline, gateway);
        return pipeline;
    }

    public static void wire(Pipeline pipeline, Gateway gateway) {
        BlockingQueue<Task> parentAQueue = new LinkedBlockingQueue<>();
        BlockingQueue<Task> parentBQueue = new LinkedBlockingQueue<>();
        BlockingQueue<Task> childQueue = new LinkedBlockingQueue<>();
        BlockingQueue<Task> alternativeQueue = new LinkedBlockingQueue<>();
        ChildEndpoint childEndpoint = new ChildEndpoint(childQueue);
        ChildWorker childWorker = new ChildWorker(childQueue, gateway, "fixture:", 1);
        RetargetEndpoint alternative = new RetargetEndpoint(alternativeQueue);
        ParentAEndpoint parentAEndpoint = new ParentAEndpoint(parentAQueue);
        ParentAWorker parentAWorker = new ParentAWorker(parentAQueue, childEndpoint);
        ParentBEndpoint parentBEndpoint = new ParentBEndpoint(parentBQueue);
        ParentBWorker parentBWorker = new ParentBWorker(parentBQueue, childEndpoint, alternative);
        pipeline.parentAEndpoint = parentAEndpoint;
        pipeline.parentAWorker = parentAWorker;
        pipeline.parentBEndpoint = parentBEndpoint;
        pipeline.parentBWorker = parentBWorker;
        pipeline.childEndpoint = childEndpoint;
        pipeline.childWorker = childWorker;
        pipeline.alternative = alternative;
    }

    public static final class Pipeline {
        public ParentAEndpoint parentAEndpoint;
        public ParentAWorker parentAWorker;
        public ParentBEndpoint parentBEndpoint;
        public ParentBWorker parentBWorker;
        public ChildEndpoint childEndpoint;
        public ChildWorker childWorker;
        public RetargetEndpoint alternative;

        private Pipeline() {}
    }
}
