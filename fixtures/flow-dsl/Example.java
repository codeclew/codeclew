package example;
// Synthetic documentation fixtures. No proprietary framework implementation.
class BuilderFactory {
    Object root;
    void construct() {
        root = FlowBuilder.build(this::init);
        root.then(this::readHistory).then(this::fetchA, this::fetchB)
            .then(this::mapData).then(this::validate).then(this::persist)
            .then(this::requestDecision).then(this::finish);
        String misleading = "root.then(this::notReal)";
        // root.then(this::notRealEither);
        other.then(this::unrelated);
    }
    TaskType getTaskType() { return TaskType.BUILDER_EXAMPLE; }
    void init() { context.ready = true; }
    void readHistory() { history.read(); }
    void fetchA() { client.fetchA(); }
    void fetchB() { client.fetchB(); }
    void mapData() { context.requestId = input.id; }
    void validate() { if (context.requestId == null) { throw new IllegalStateException(); } }
    void persist() { repository.save(context); }
    void requestDecision() { odmClient.evaluate(context); }
    void finish() { return; }
}
class QueueFactory {
    Object construct() { return new DynamicFlow<>(registry, exceptionHandler, Operation.INIT.name(), Operation.DECISION.name()); }
    TaskType getTaskType() { return TaskType.QUEUE_EXAMPLE; }
}
class Registry {
    void register() {
        OPERATIONS_TO_CONTEXT_METHODS.put(Operation.DECISION, QueueContext::decision);
        OPERATIONS_TO_CONTEXT_METHODS.put(Operation.INIT, QueueContext::init);
        OPERATIONS_TO_CONTEXT_METHODS.put(Operation.SEND, QueueContext::send);
        OPERATIONS_TO_CONTEXT_METHODS.put(Operation.FINISH, QueueContext::finish);
        other.put(Operation.FAKE, Unrelated::fake);
        // OPERATIONS_TO_CONTEXT_METHODS.put(Operation.GHOST, QueueContext::ghost);
    }
}
class QueueContext {
    void init() { context.phone = input.phone; context.reason = "if (ready) return Operation.GHOST"; while (unknown()) { queue.addOperations(Operation.GHOST.name()); } }
    Object decision() {
        if (context.phone == null) { return List.of(Operation.FINISH.name()); }
        else if (isEligible(context)) { return List.of(Operation.SEND.name(), Operation.FINISH.name()); }
        else { queue.addOperations(Operation.FINISH.name()); }
        return computeNext();
    }
    void send() { odmClient.evaluate(context); context.selection = response.value; notification.submit(context); }
    void finish() { return; }
}
class Unrelated {
    Object decision() { return List.of(Operation.FAKE.name()); }
}
