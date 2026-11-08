import java.util.concurrent.CompletionStage;

// Neutral source fixture. Deployment addresses and delivery guarantees are absent.
final class Intake {
    static final String TOPIC = "items.accepted";
    final Config config;
    final Stock stock;
    final Audit audit;
    Intake(Config config, Stock stock, Audit audit) {
        this.config = config; this.stock = stock; this.audit = audit;
    }
    record Request(String itemId, int quantity, boolean test) {}
    record Result(String itemId, boolean checked) {}
    Object route(String method, String path, Request request) {
        return switch (method + " " + path) {
            case "POST /v1/items" -> postV1(request);
            case "PUT /v2/items" -> putV2(request);
            default -> throw new IllegalArgumentException("404");
        };
    }
    Result postV1(Request request) {
        return accept(request, false);
    }
    Result putV2(Request request) {
        if (request.quantity() < 1) throw new IllegalArgumentException("quantity");
        return accept(request, true);
    }
    Result accept(Request request, boolean versionTwo) {
        boolean enabled = flag();
        boolean checked = false;
        if (enabled) checked = checkStock(request);
        if (versionTwo && !isTest(request)) audit.send(TOPIC, request);
        return new Result(request.itemId(), checked);
    }
    boolean flag() { return config.get("stock.check", false); }
    boolean checkStock(Request request) { return stock.check(request.itemId(), request.quantity()); }
    boolean isTest(Request request) { return request.test(); }
    interface Config { boolean get(String key, boolean fallback); }
    interface Stock { boolean check(String id, int quantity); }
    interface Audit { CompletionStage<Void> send(String topic, Request request); }
}
