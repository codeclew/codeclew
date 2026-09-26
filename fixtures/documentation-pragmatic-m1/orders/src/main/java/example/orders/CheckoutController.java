package example.orders;

import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RestController;

@RestController
public class CheckoutController {
    private final InventoryClient inventory;
    private final ReservationRepository reservations;

    public CheckoutController(InventoryClient inventory, ReservationRepository reservations) {
        this.inventory = inventory;
        this.reservations = reservations;
    }

    @PostMapping("/checkout")
    public String checkout(@RequestBody ReservationRequest request) {
        if (!hasPositiveQuantity(request)) {
            return "invalid quantity";
        }
        reservations.save(request);
        return inventory.reserve(request);
    }

    private static boolean hasPositiveQuantity(ReservationRequest request) {
        return request != null && request.quantity() > 0;
    }

    public Runnable prepareSaveCallback(ReservationRequest request) {
        return () -> reservations.save(request);
    }

    @GetMapping("/ready")
    public String ready() {
        return "ready";
    }
}
