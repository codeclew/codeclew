package example;
import org.springframework.web.bind.annotation.RestController;
import org.springframework.web.bind.annotation.RequestMapping;
import org.springframework.web.bind.annotation.PostMapping;
@RestController
@RequestMapping("/orders")
public class Orders {
    @PostMapping(path = "/reserve")
    public int reserve(int quantity) { return quantity; }
}
