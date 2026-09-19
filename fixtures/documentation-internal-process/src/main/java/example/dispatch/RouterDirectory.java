package example.dispatch;

import java.util.Map;

public final class RouterDirectory {
    private final Map<String, String> routes;
    public RouterDirectory(Map<String, String> routes) { this.routes = Map.copyOf(routes); }
    public String endpointFor(String type) {
        String endpoint = routes.get(type);
        if (endpoint == null) { throw new MissingRouteException(type); }
        return endpoint;
    }
}
