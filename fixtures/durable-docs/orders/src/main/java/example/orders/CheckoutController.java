package example.orders;

import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RestController;

@RestController
public class CheckoutController {
    private final InventoryClient inventory;
    public CheckoutController(InventoryClient inventory) { this.inventory = inventory; }
    @PostMapping("/checkout")
    public String checkout(@RequestBody ReservationRequest request) {
        if (request.quantity() <= 0) {
            return "invalid quantity";
        }
        return inventory.reserve(request);
    }
}
