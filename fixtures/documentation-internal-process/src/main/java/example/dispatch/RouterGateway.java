package example.dispatch;

public final class RouterGateway {
    private final RouterDirectory directory;
    private final DeliveryClient client;
    public RouterGateway(RouterDirectory directory, DeliveryClient client) {
        this.directory = directory;
        this.client = client;
    }
    public void dispatch(Task task) {
        String endpoint = directory.endpointFor(task.type);
        client.send(endpoint, task.id);
    }
}
