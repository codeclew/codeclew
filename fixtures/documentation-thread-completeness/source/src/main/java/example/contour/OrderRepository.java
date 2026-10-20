package example.contour;
public class OrderRepository {
    private final ExternalClient client;
    private int attempts;
    public OrderRepository(ExternalClient client) { this.client = client; }
    public String write(String value) {
        attempts++;
        client.send(value);
        return "saved";
    }
}
interface ExternalClient { void send(String value); }
class StorageFailure extends RuntimeException {}
