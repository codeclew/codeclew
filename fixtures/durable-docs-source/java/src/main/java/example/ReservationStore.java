package example;

import java.util.HashMap;
import java.util.Map;

public class ReservationStore {
    private final Map<String, Integer> values = new HashMap<>();

    public void save(String sku, int quantity) {
        values.put(sku, quantity);
    }
}
