package example.orders;

import org.springframework.beans.factory.annotation.Value;
import org.springframework.web.client.RestTemplate;

public class InventoryClient {
    private final RestTemplate http;
    @Value("${inventory.base-url}")
    private String inventoryBaseUrl;
    public InventoryClient(RestTemplate http) { this.http = http; }
    public String reserve(ReservationRequest request) {
        return http.postForObject(inventoryBaseUrl + "/reservations", request, String.class);
    }
}
