package example.contour;
import org.springframework.web.bind.annotation.RestController;
import org.springframework.web.bind.annotation.RequestMapping;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RequestParam;
@RestController
@RequestMapping("/admin")
public class AdminController {
    private final LookupPort port;
    public AdminController(LookupPort port) { this.port = port; }
    @GetMapping("/status")
    public String status() { return "ready"; }
    @GetMapping("/port")
    public String lookupPort(@RequestParam String value) { return port.lookup(value); }
}
