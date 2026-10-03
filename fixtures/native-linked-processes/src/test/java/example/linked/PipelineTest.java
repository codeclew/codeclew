package example.linked;

import static org.junit.jupiter.api.Assertions.*;

import java.util.List;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;
import org.junit.jupiter.api.Test;

class PipelineTest {
    private static Task task(String name, boolean eligible, int priority) {
        return new Task("fixture-reference", name, eligible, priority);
    }

    @Test
    void bothParentsSubmitToTheSameChildInFifoOrder() {
        RecordingGateway gateway = new RecordingGateway(0, false);
        Composition.Pipeline pipeline = Composition.assemble(gateway);
        assertTrue(pipeline.parentAEndpoint.submit(task(" café ", true, 1)));
        assertTrue(pipeline.parentBEndpoint.enqueue(task(" beta ", true, 3)));
        pipeline.parentAWorker.runOnce();
        pipeline.parentBWorker.runOnce();
        assertEquals("submitted-to-child", pipeline.parentAWorker.lastState);
        assertEquals("submitted-to-child", pipeline.parentBWorker.lastState);
        assertEquals(List.of(), gateway.requests());
        assertEquals(0, pipeline.childWorker.attempts);
        pipeline.childWorker.runOnce();
        pipeline.childWorker.runOnce();
        assertEquals(List.of("fixture:café", "fixture:beta"), gateway.requests());
        assertEquals(2, pipeline.childWorker.attempts);
        assertEquals("acknowledged", pipeline.childWorker.lastState);
    }

    @Test
    void emptyQueuesAndNullEndpointsDoNotAttemptGatewayCalls() {
        RecordingGateway gateway = new RecordingGateway(0, false);
        Composition.Pipeline pipeline = Composition.assemble(gateway);
        assertFalse(pipeline.parentAEndpoint.submit(null));
        assertFalse(pipeline.parentBEndpoint.enqueue(null));
        assertFalse(pipeline.childEndpoint.submit(null));
        pipeline.parentAWorker.runOnce();
        pipeline.parentBWorker.runOnce();
        pipeline.childWorker.runOnce();
        assertEquals("empty", pipeline.parentAWorker.lastState);
        assertEquals("empty", pipeline.parentBWorker.lastState);
        assertEquals("empty", pipeline.childWorker.lastState);
        assertEquals(List.of(), gateway.requests());
    }

    @Test
    void parentAFirstGuardWinsAndConsumedTaskIsNotAutomaticallyRetried() {
        RecordingGateway gateway = new RecordingGateway(0, false);
        Composition.Pipeline pipeline = Composition.assemble(gateway);
        pipeline.parentAWorker.enabled = false;
        pipeline.parentAEndpoint.submit(task("blocked", false, 0));
        pipeline.parentAWorker.runOnce();
        assertEquals("disabled", pipeline.parentAWorker.lastState);
        pipeline.parentAWorker.enabled = true;
        pipeline.parentAWorker.runOnce();
        assertEquals("empty", pipeline.parentAWorker.lastState);
        pipeline.childWorker.runOnce();
        assertEquals(List.of(), gateway.requests());
    }

    @Test
    void parentAGuardsKeepEligibilitySeparateFromThePriorityThreshold() {
        RecordingGateway gateway = new RecordingGateway(0, false);
        Composition.Pipeline pipeline = Composition.assemble(gateway);
        pipeline.parentAEndpoint.submit(task("blocked", false, 1));
        pipeline.parentAWorker.runOnce();
        assertEquals("ineligible", pipeline.parentAWorker.lastState);
        pipeline.parentAEndpoint.submit(task("low", true, 0));
        pipeline.parentAWorker.runOnce();
        assertEquals("below-parent-priority", pipeline.parentAWorker.lastState);
        pipeline.parentAEndpoint.submit(task("equal", true, 1));
        pipeline.parentAWorker.runOnce();
        pipeline.childWorker.runOnce();
        assertEquals(List.of("fixture:equal"), gateway.requests());
    }

    @Test
    void parentBHasItsOwnThresholdAndNullNameGuard() {
        RecordingGateway gateway = new RecordingGateway(0, false);
        Composition.Pipeline pipeline = Composition.assemble(gateway);
        pipeline.parentBEndpoint.enqueue(task(null, true, 2));
        pipeline.parentBWorker.runOnce();
        assertEquals("below-parent-priority", pipeline.parentBWorker.lastState);
        pipeline.parentBEndpoint.enqueue(task(null, true, 3));
        pipeline.parentBWorker.runOnce();
        assertEquals("missing-name", pipeline.parentBWorker.lastState);
        pipeline.parentBEndpoint.enqueue(task("equal", true, 3));
        pipeline.parentBWorker.runOnce();
        pipeline.childWorker.runOnce();
        assertEquals(List.of("fixture:equal"), gateway.requests());
    }

    @Test
    void sameNamedAlternativeSubmitDoesNotFeedTheChild() {
        RecordingGateway gateway = new RecordingGateway(0, false);
        Composition.Pipeline pipeline = Composition.assemble(gateway);
        pipeline.parentBWorker.useAlternative = true;
        pipeline.parentBEndpoint.enqueue(task("held", true, 3));
        pipeline.parentBWorker.runOnce();
        assertEquals("held-in-alternative", pipeline.parentBWorker.lastState);
        assertEquals(1, pipeline.alternative.queued());
        pipeline.childWorker.runOnce();
        assertEquals("empty", pipeline.childWorker.lastState);
        assertEquals(List.of(), gateway.requests());
    }

    @Test
    void childRefusalIsObservableAtBothParentCallOccurrences() {
        BlockingQueue<Task> childQueue = new ArrayBlockingQueue<>(1);
        ChildEndpoint child = new ChildEndpoint(childQueue);
        assertTrue(child.submit(task("already-queued", true, 3)));
        BlockingQueue<Task> aQueue = new LinkedBlockingQueue<>();
        BlockingQueue<Task> bQueue = new LinkedBlockingQueue<>();
        ParentAWorker a = new ParentAWorker(aQueue, child);
        ParentBWorker b = new ParentBWorker(bQueue, child,
            new RetargetEndpoint(new LinkedBlockingQueue<>()));
        aQueue.offer(task("a", true, 1));
        bQueue.offer(task("b", true, 3));
        a.runOnce();
        b.runOnce();
        assertEquals("child-refused", a.lastState);
        assertEquals("child-refused", b.lastState);
        assertEquals("already-queued", childQueue.peek().name);
        assertEquals(1, childQueue.size());
    }

    @Test
    void childTransformsUnicodeAndDefaultsOnlyNullNames() {
        RecordingGateway gateway = new RecordingGateway(0, false);
        Composition.Pipeline pipeline = Composition.assemble(gateway);
        pipeline.childEndpoint.submit(task(" café☕ ", true, 1));
        pipeline.childEndpoint.submit(task(null, true, 1));
        pipeline.childEndpoint.submit(task("   ", true, 1));
        pipeline.childWorker.runOnce();
        pipeline.childWorker.runOnce();
        pipeline.childWorker.runOnce();
        assertEquals(List.of("fixture:café☕", "fixture:anonymous", "fixture:"), gateway.requests());
        assertEquals(3, pipeline.childWorker.attempts);
        assertEquals("fixture:", pipeline.childWorker.lastRequest);
    }

    @Test
    void childEligibilityAndFieldThresholdPreventAnAttempt() {
        BlockingQueue<Task> queue = new LinkedBlockingQueue<>();
        RecordingGateway gateway = new RecordingGateway(0, false);
        ChildEndpoint endpoint = new ChildEndpoint(queue);
        ChildWorker worker = new ChildWorker(queue, gateway, "test:", 4);
        endpoint.submit(task("blocked", false, 4));
        worker.runOnce();
        assertEquals("ineligible", worker.lastState);
        endpoint.submit(task("low", true, 3));
        worker.runOnce();
        assertEquals("below-child-priority", worker.lastState);
        assertEquals(0, worker.attempts);
        endpoint.submit(task("equal", true, 4));
        worker.runOnce();
        assertEquals(List.of("test:equal"), gateway.requests());
    }

    @Test
    void nonzeroLocalResponseIsRejectedAfterExactlyOneAttempt() {
        RecordingGateway gateway = new RecordingGateway(7, false);
        Composition.Pipeline pipeline = Composition.assemble(gateway);
        pipeline.childEndpoint.submit(task("response", true, 1));
        pipeline.childWorker.runOnce();
        assertEquals("rejected", pipeline.childWorker.lastState);
        assertEquals(1, pipeline.childWorker.attempts);
        assertEquals(List.of("fixture:response"), gateway.requests());
    }

    @Test
    void gatewayExceptionPropagatesAfterPreparationWithoutAcknowledgement() {
        RecordingGateway gateway = new RecordingGateway(0, true);
        Composition.Pipeline pipeline = Composition.assemble(gateway);
        pipeline.childEndpoint.submit(task(" failure ", true, 1));
        IllegalStateException failure = assertThrows(IllegalStateException.class,
            () -> pipeline.childWorker.runOnce());
        assertEquals("fixture-local gateway failure", failure.getMessage());
        assertEquals("attempting", pipeline.childWorker.lastState);
        assertEquals("fixture:failure", pipeline.childWorker.lastRequest);
        assertEquals(1, pipeline.childWorker.attempts);
        assertEquals(List.of("fixture:failure"), gateway.requests());
        pipeline.childWorker.runOnce();
        assertEquals("empty", pipeline.childWorker.lastState);
        assertEquals(1, pipeline.childWorker.attempts);
    }

    @Test
    void staticCycleCanBeDrivenWithAnExplicitFiniteCounter() {
        CycleProbe probe = new CycleProbe();
        assertEquals(0, probe.first(0));
        assertEquals(4, probe.first(4));
        assertEquals(3, probe.second(3));
    }
}
