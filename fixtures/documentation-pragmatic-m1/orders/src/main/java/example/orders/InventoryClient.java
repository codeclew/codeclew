package example.orders;

import org.springframework.beans.factory.annotation.Value;
import org.springframework.web.client.RestClient;
import org.springframework.web.client.RestTemplate;

public class InventoryClient {
    private final RestTemplate http;
    private final RestClient requestBuilder = RestClient.create();

    @Value("${inventory.base-url}")
    private String inventoryBaseUrl;

    public InventoryClient(RestTemplate http) {
        this.http = http;
    }

    public String reserve(ReservationRequest request) {
        return http.postForObject(inventoryBaseUrl + "/reservations", request, String.class);
    }

    public RestClient.RequestBodySpec prepareReservationRequest(ReservationRequest request) {
        return requestBuilder.post().uri(inventoryBaseUrl + "/reservations").body(request);
    }
}
