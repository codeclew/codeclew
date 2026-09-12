package example;

import example.missing.ExternalPolicy;

public class Reservations {
    private final ReservationStore store = new ReservationStore();
    private final ExternalPolicy externalPolicy;

    public Reservations(ExternalPolicy externalPolicy) { this.externalPolicy = externalPolicy; }

    /** This source describes a memory update, not durable storage. */
    public String reserve(String sku, int quantity) {
        if (quantity <= 0) {
            return "invalid quantity";
        }
        if (quantity > 100) {
            return "insufficient stock";
        }
        store.save(sku, quantity);
        return "reserved: café";
    }
}
