package example.inventory;

import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RestController;

@RestController
public class ReservationController {
    private final ReservationStore store;
    public ReservationController(ReservationStore store) { this.store = store; }
    @PostMapping("/reservations")
    public String create(@RequestBody ReservationRequest request) {
        if (request.quantity() > 100) {
            return "insufficient stock";
        }
        store.save(request);
        return "reserved";
    }
}
