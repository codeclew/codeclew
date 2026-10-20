package example.contour;
import java.util.concurrent.CompletableFuture;
public class OrderService {
    private final Validator validator;
    private final OrderRepository repository;
    public OrderService(Validator validator, OrderRepository repository) {
        this.validator = validator;
        this.repository = repository;
    }
    public String save(String value) {
        String normalized = validator.normalize(value);
        try { return repository.write(normalized); }
        catch (StorageFailure failure) { return "failed"; }
    }
    public String lookup(String value) { return decorate(value); }
    public String decorate(String value) { return "string:" + value; }
    public String decorate(int value) { return "number:" + value; }
    public void submit(String value) {
        CompletableFuture.runAsync(() -> repository.write(value));
    }
}
