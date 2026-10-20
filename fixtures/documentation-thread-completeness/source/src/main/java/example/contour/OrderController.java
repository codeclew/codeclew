package example.contour;
import org.springframework.web.bind.annotation.RestController;
import org.springframework.web.bind.annotation.RequestMapping;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RequestParam;
@RestController
@RequestMapping("/orders")
public class OrderController {
    private final OrderService service;
    public OrderController(OrderService service) { this.service = service; }
    @PostMapping("/save")
    public String save(@RequestBody String value) { return service.save(value); }
    @GetMapping("/lookup")
    public String lookup(@RequestParam String value) { return service.lookup(value); }
    @PostMapping("/async")
    public String submit(@RequestBody String value) { service.submit(value); return "accepted"; }
}
