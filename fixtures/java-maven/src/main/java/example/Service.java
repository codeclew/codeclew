package example;

@Deprecated
public final class Service {
    private final Gateway gateway;

    public Service(Gateway gateway) {
        this.gateway = gateway;
    }

    public String fetch(String key) {
        return gateway.load(key);
    }
}
